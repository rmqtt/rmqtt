//! # MQTT Server Implementation
//!
//! ## Overall Example
//!
//! ```rust,no_run
//! use std::net::{Ipv4Addr, SocketAddr};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create server configuration
//!     let builder = rmqtt_net::Builder::new()
//!         .name("MyMQTTBroker")
//!         .laddr(SocketAddr::from((Ipv4Addr::LOCALHOST, 1883)))
//!         .max_connections(5000);
//!
//!     // Bind TCP listener
//!     let listener = builder.bind()?;
//!
//!     // Accept and handle connections
//!     loop {
//!         let acceptor = listener.accept().await?;
//!         tokio::spawn(async move {
//!             let dispatcher = acceptor.tcp().unwrap();
//!             // Handle MQTT protocol...
//!         });
//!     }
//!     Ok(())
//! }
//! ```

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use nonzero_ext::nonzero;
use rmqtt_codec::types::QoS;
#[cfg(not(target_os = "windows"))]
#[cfg(feature = "tls")]
use rustls::crypto::aws_lc_rs as provider;
#[cfg(feature = "tls")]
#[cfg(target_os = "windows")]
use rustls::crypto::ring as provider;
#[cfg(feature = "tls")]
use rustls::{pki_types::pem::PemObject, server::WebPkiClientVerifier, RootCertStore, ServerConfig};
use socket2::{Domain, SockAddr, Socket, Type};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
#[cfg(feature = "tls")]
use tokio_rustls::{server::TlsStream, TlsAcceptor};
#[cfg(feature = "ws")]
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::handshake::server::{ErrorResponse, Request, Response},
};

use crate::stream::Dispatcher;
#[cfg(feature = "ws")]
use crate::ws::WsStream;
use crate::{Error, Result};

/// Configuration builder for MQTT server instances
#[derive(Clone, Debug)]
pub struct Builder {
    /// Server identifier for logging and monitoring
    pub name: String,
    /// Network address to listen on
    pub laddr: SocketAddr,
    /// Maximum number of pending connections in the accept queue
    pub backlog: i32,
    /// Enable TCP_NODELAY option for lower latency
    pub nodelay: bool,
    /// Set SO_REUSEADDR socket option
    pub reuseaddr: Option<bool>,
    /// Set SO_REUSEPORT socket option
    pub reuseport: Option<bool>,
    /// Maximum concurrent active connections
    pub max_connections: usize,
    /// Maximum simultaneous handshakes during connection setup
    pub max_handshaking_limit: usize,
    /// Maximum allowed MQTT packet size in bytes (0 = unlimited)
    pub max_packet_size: u32,

    /// Allow unauthenticated client connections
    pub allow_anonymous: bool,
    /// Minimum acceptable keepalive value in seconds
    pub min_keepalive: u16,
    /// Maximum acceptable keepalive value in seconds
    pub max_keepalive: u16,
    /// Allow clients to disable keepalive mechanism
    pub allow_zero_keepalive: bool,
    /// Multiplier for calculating actual keepalive timeout
    pub keepalive_backoff: f32,
    /// Window size for unacknowledged QoS 1/2 messages
    pub max_inflight: NonZeroU16,
    /// Timeout for completing connection handshake
    pub handshake_timeout: Duration,
    /// Network I/O timeout for sending operations
    pub send_timeout: Duration,
    /// Maximum messages queued per client
    pub max_mqueue_len: usize,
    /// Rate limiting for message delivery (messages per duration)
    pub mqueue_rate_limit: (NonZeroU32, Duration),
    /// Maximum length of client identifiers
    pub max_clientid_len: usize,
    /// Highest QoS level permitted for publishing
    pub max_qos_allowed: QoS,
    /// Maximum depth for topic hierarchy (0 = unlimited)
    pub max_topic_levels: usize,
    /// Duration before inactive sessions expire
    pub session_expiry_interval: Duration,
    /// The upper limit for how long a session can remain valid before it must expire,
    /// regardless of the client's requested session expiry interval. (0 = unlimited)
    pub max_session_expiry_interval: Duration,
    /// Retry interval for unacknowledged messages
    pub message_retry_interval: Duration,
    /// Time-to-live for undelivered messages
    pub message_expiry_interval: Duration,
    /// Maximum subscriptions per client (0 = unlimited)
    pub max_subscriptions: usize,
    /// Enable shared subscription support
    pub shared_subscription: bool,
    /// Maximum topic aliases (MQTTv5 feature)
    pub max_topic_aliases: u16,
    /// Enable subscription count limiting
    pub limit_subscription: bool,
    /// Enable future-dated message publishing
    pub delayed_publish: bool,

    /// Enable mutual TLS authentication
    pub tls_cross_certificate: bool,
    /// Path to TLS certificate chain
    pub tls_cert: Option<String>,
    /// Path to TLS private key
    pub tls_key: Option<String>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// # Examples
/// ```
/// use std::net::SocketAddr;
/// use rmqtt_net::Builder;
///
/// let builder = Builder::new()
///     .name("EdgeBroker")
///     .laddr("127.0.0.1:1883".parse().unwrap())
///     .max_connections(10_000);
/// ```
impl Builder {
    /// Creates a new builder with default configuration values
    pub fn new() -> Builder {
        Builder {
            name: Default::default(),
            laddr: SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 1883)),
            max_connections: 1_000_000,
            max_handshaking_limit: 1_000,
            max_packet_size: 1024 * 1024,
            backlog: 512,
            nodelay: false,
            reuseaddr: None,
            reuseport: None,

            allow_anonymous: true,
            min_keepalive: 0,
            max_keepalive: 65535,
            allow_zero_keepalive: true,
            keepalive_backoff: 0.75,
            max_inflight: nonzero!(16u16),
            handshake_timeout: Duration::from_secs(30),
            send_timeout: Duration::from_secs(10),
            max_mqueue_len: 1000,

            mqueue_rate_limit: (nonzero!(u32::MAX), Duration::from_secs(1)),
            max_clientid_len: 65535,
            max_qos_allowed: QoS::ExactlyOnce,
            max_topic_levels: 0,
            session_expiry_interval: Duration::from_secs(2 * 60 * 60),
            max_session_expiry_interval: Duration::ZERO,
            message_retry_interval: Duration::from_secs(20),
            message_expiry_interval: Duration::from_secs(5 * 60),
            max_subscriptions: 0,
            shared_subscription: true,
            max_topic_aliases: 0,

            limit_subscription: false,
            delayed_publish: false,

            tls_cross_certificate: false,
            tls_cert: None,
            tls_key: None,
        }
    }

    /// Sets the server name identifier
    pub fn name<N: Into<String>>(mut self, name: N) -> Self {
        self.name = name.into();
        self
    }

    /// Configures the network listen address
    pub fn laddr(mut self, laddr: SocketAddr) -> Self {
        self.laddr = laddr;
        self
    }

    /// Sets the TCP backlog size
    pub fn backlog(mut self, backlog: i32) -> Self {
        self.backlog = backlog;
        self
    }

    /// Enables/disables TCP_NODELAY option
    pub fn nodelay(mut self, nodelay: bool) -> Self {
        self.nodelay = nodelay;
        self
    }

    /// Configures SO_REUSEADDR socket option
    pub fn reuseaddr(mut self, reuseaddr: Option<bool>) -> Self {
        self.reuseaddr = reuseaddr;
        self
    }

    /// Configures SO_REUSEPORT socket option
    pub fn reuseport(mut self, reuseport: Option<bool>) -> Self {
        self.reuseport = reuseport;
        self
    }

    /// Sets maximum concurrent connections
    pub fn max_connections(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }

    /// Sets maximum concurrent handshakes
    pub fn max_handshaking_limit(mut self, max_handshaking_limit: usize) -> Self {
        self.max_handshaking_limit = max_handshaking_limit;
        self
    }

    /// Configures maximum MQTT packet size
    pub fn max_packet_size(mut self, max_packet_size: u32) -> Self {
        self.max_packet_size = max_packet_size;
        self
    }

    /// Enables anonymous client access
    pub fn allow_anonymous(mut self, allow_anonymous: bool) -> Self {
        self.allow_anonymous = allow_anonymous;
        self
    }

    /// Sets minimum acceptable keepalive value
    pub fn min_keepalive(mut self, min_keepalive: u16) -> Self {
        self.min_keepalive = min_keepalive;
        self
    }

    /// Sets maximum acceptable keepalive value
    pub fn max_keepalive(mut self, max_keepalive: u16) -> Self {
        self.max_keepalive = max_keepalive;
        self
    }

    /// Allows clients to disable keepalive
    pub fn allow_zero_keepalive(mut self, allow_zero_keepalive: bool) -> Self {
        self.allow_zero_keepalive = allow_zero_keepalive;
        self
    }

    /// Configures keepalive backoff multiplier
    pub fn keepalive_backoff(mut self, keepalive_backoff: f32) -> Self {
        self.keepalive_backoff = keepalive_backoff;
        self
    }

    /// Sets inflight message window size
    pub fn max_inflight(mut self, max_inflight: NonZeroU16) -> Self {
        self.max_inflight = max_inflight;
        self
    }

    /// Configures handshake timeout duration
    pub fn handshake_timeout(mut self, handshake_timeout: Duration) -> Self {
        self.handshake_timeout = handshake_timeout;
        self
    }

    /// Sets network send timeout duration
    pub fn send_timeout(mut self, send_timeout: Duration) -> Self {
        self.send_timeout = send_timeout;
        self
    }

    /// Configures maximum message queue length
    pub fn max_mqueue_len(mut self, max_mqueue_len: usize) -> Self {
        self.max_mqueue_len = max_mqueue_len;
        self
    }

    /// Sets message rate limiting parameters
    pub fn mqueue_rate_limit(mut self, rate_limit: NonZeroU32, duration: Duration) -> Self {
        self.mqueue_rate_limit = (rate_limit, duration);
        self
    }

    /// Sets maximum client ID length
    pub fn max_clientid_len(mut self, max_clientid_len: usize) -> Self {
        self.max_clientid_len = max_clientid_len;
        self
    }

    /// Configures maximum allowed QoS level
    pub fn max_qos_allowed(mut self, max_qos_allowed: QoS) -> Self {
        self.max_qos_allowed = max_qos_allowed;
        self
    }

    /// Sets maximum topic hierarchy depth
    pub fn max_topic_levels(mut self, max_topic_levels: usize) -> Self {
        self.max_topic_levels = max_topic_levels;
        self
    }

    /// Configures session expiration interval
    pub fn session_expiry_interval(mut self, session_expiry_interval: Duration) -> Self {
        self.session_expiry_interval = session_expiry_interval;
        self
    }

    /// Configures max session expiration interval
    pub fn max_session_expiry_interval(mut self, max_session_expiry_interval: Duration) -> Self {
        self.max_session_expiry_interval = max_session_expiry_interval;
        self
    }

    /// Sets message retry interval for QoS 1/2
    pub fn message_retry_interval(mut self, message_retry_interval: Duration) -> Self {
        self.message_retry_interval = message_retry_interval;
        self
    }

    /// Configures message expiration time
    pub fn message_expiry_interval(mut self, message_expiry_interval: Duration) -> Self {
        self.message_expiry_interval = message_expiry_interval;
        self
    }

    /// Sets maximum subscriptions per client
    pub fn max_subscriptions(mut self, max_subscriptions: usize) -> Self {
        self.max_subscriptions = max_subscriptions;
        self
    }

    /// Enables shared subscription support
    pub fn shared_subscription(mut self, shared_subscription: bool) -> Self {
        self.shared_subscription = shared_subscription;
        self
    }

    /// Configures maximum topic aliases (MQTTv5)
    pub fn max_topic_aliases(mut self, max_topic_aliases: u16) -> Self {
        self.max_topic_aliases = max_topic_aliases;
        self
    }

    /// Enables subscription count limiting
    pub fn limit_subscription(mut self, limit_subscription: bool) -> Self {
        self.limit_subscription = limit_subscription;
        self
    }

    /// Enables delayed message publishing
    pub fn delayed_publish(mut self, delayed_publish: bool) -> Self {
        self.delayed_publish = delayed_publish;
        self
    }

    /// Enables mutual TLS authentication
    pub fn tls_cross_certificate(mut self, cross_certificate: bool) -> Self {
        self.tls_cross_certificate = cross_certificate;
        self
    }

    /// Sets path to TLS certificate chain
    pub fn tls_cert<N: Into<String>>(mut self, tls_cert: Option<N>) -> Self {
        self.tls_cert = tls_cert.map(|c| c.into());
        self
    }

    /// Sets path to TLS private key
    pub fn tls_key<N: Into<String>>(mut self, tls_key: Option<N>) -> Self {
        self.tls_key = tls_key.map(|c| c.into());
        self
    }

    /// Binds the server to the configured address
    #[allow(unused_variables)]
    pub fn bind(self) -> Result<Listener> {
        let builder = match self.laddr {
            SocketAddr::V4(_) => Socket::new(Domain::IPV4, Type::STREAM, None)?,
            SocketAddr::V6(_) => Socket::new(Domain::IPV6, Type::STREAM, None)?,
        };

        builder.set_linger(Some(Duration::from_secs(10)))?;

        builder.set_nonblocking(true)?;

        if let Some(reuseaddr) = self.reuseaddr {
            builder.set_reuse_address(reuseaddr)?;
        }

        #[cfg(not(windows))]
        if let Some(reuseport) = self.reuseport {
            builder.set_reuse_port(reuseport)?;
        }

        builder.bind(&SockAddr::from(self.laddr))?;
        builder.listen(self.backlog)?;
        let tcp_listener = TcpListener::from_std(std::net::TcpListener::from(builder))?;

        log::info!(
            "MQTT Broker Listening on {} {}",
            self.name,
            tcp_listener.local_addr().unwrap_or(self.laddr)
        );
        Ok(Listener {
            typ: ListenerType::TCP,
            cfg: Arc::new(self),
            tcp_listener,
            #[cfg(feature = "tls")]
            tls_acceptor: None,
        })
    }
}

/// Protocol variants for network listeners
#[derive(Debug, Copy, Clone)]
pub enum ListenerType {
    /// Plain TCP listener
    TCP,
    #[cfg(feature = "tls")]
    /// TLS-secured TCP listener
    TLS,
    #[cfg(feature = "ws")]
    /// WebSocket listener
    WS,
    #[cfg(feature = "tls")]
    #[cfg(feature = "ws")]
    /// TLS-secured WebSocket listener
    WSS,
}

/// Network listener for accepting client connections
pub struct Listener {
    /// Active listener protocol type
    pub typ: ListenerType,
    /// Shared server configuration
    pub cfg: Arc<Builder>,
    tcp_listener: TcpListener,
    #[cfg(feature = "tls")]
    tls_acceptor: Option<TlsAcceptor>,
}

/// # Examples
/// ```
/// # use rmqtt_net::{Builder, Listener};
/// # fn setup() -> Result<(), Box<dyn std::error::Error>> {
/// let builder = Builder::new();
/// let listener = builder.bind()?;
/// # Ok(())
/// # }
/// ```
impl Listener {
    /// Converts listener to plain TCP mode
    pub fn tcp(mut self) -> Result<Self> {
        let _err = anyhow!("Protocol downgrade from TLS/WS/WSS to TCP is not permitted");
        #[cfg(feature = "tls")]
        if matches!(self.typ, ListenerType::TLS) {
            return Err(_err);
        }
        #[cfg(feature = "tls")]
        #[cfg(feature = "ws")]
        if matches!(self.typ, ListenerType::WSS) {
            return Err(_err);
        }
        #[cfg(feature = "ws")]
        if matches!(self.typ, ListenerType::WS) {
            return Err(_err);
        }
        self.typ = ListenerType::TCP;
        Ok(self)
    }

    #[cfg(feature = "ws")]
    /// Upgrades listener to WebSocket protocol
    pub fn ws(mut self) -> Result<Self> {
        if matches!(self.typ, ListenerType::TCP | ListenerType::WS) {
            self.typ = ListenerType::WS;
        } else {
            return Err(anyhow!("Protocol upgrade from TLS/WSS to WS is not permitted"));
        }
        Ok(self)
    }

    #[cfg(feature = "tls")]
    #[cfg(feature = "ws")]
    /// Upgrades listener to secure WebSocket (WSS)
    pub fn wss(mut self) -> Result<Self> {
        if matches!(self.typ, ListenerType::TCP | ListenerType::WS) {
            self = self.tls()?;
        }
        self.typ = ListenerType::WSS;
        Ok(self)
    }

    #[cfg(feature = "tls")]
    /// Upgrades listener to TLS-secured TCP
    pub fn tls(mut self) -> Result<Listener> {
        match self.typ {
            #[cfg(feature = "ws")]
            ListenerType::WS | ListenerType::WSS => {
                return Err(anyhow!("Protocol downgrade from WS/WSS to TLS is not permitted"));
            }
            ListenerType::TLS => return Ok(self),
            ListenerType::TCP => {}
        }

        let cert_file = self.cfg.tls_cert.as_ref().ok_or(anyhow!("TLS certificate path not set"))?;
        let key_file = self.cfg.tls_key.as_ref().ok_or(anyhow!("TLS key path not set"))?;

        let cert_chain = rustls::pki_types::CertificateDer::pem_file_iter(cert_file)
            .map_err(|e| anyhow!(e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| anyhow!(e))?;
        let key = rustls::pki_types::PrivateKeyDer::from_pem_file(key_file).map_err(|e| anyhow!(e))?;

        let provider = Arc::new(provider::default_provider());
        let client_auth = if self.cfg.tls_cross_certificate {
            let root_chain = cert_chain.clone();
            let mut client_auth_roots = RootCertStore::empty();
            for root in root_chain {
                client_auth_roots.add(root).map_err(|e| anyhow!(e))?;
            }
            WebPkiClientVerifier::builder_with_provider(client_auth_roots.into(), provider.clone())
                .build()
                .map_err(|e| anyhow!(e))?
        } else {
            WebPkiClientVerifier::no_client_auth()
        };

        let tls_config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| anyhow!(e))?
            .with_client_cert_verifier(client_auth)
            .with_single_cert(cert_chain, key)
            .map_err(|e| anyhow!(format!("Certificate error: {}", e)))?;

        let acceptor = TlsAcceptor::from(Arc::new(tls_config));
        self.tls_acceptor = Some(acceptor);
        self.typ = ListenerType::TLS;
        Ok(self)
    }

    /// Accepts incoming client connections
    pub async fn accept(&self) -> Result<Acceptor<TcpStream>> {
        let (socket, remote_addr) = self.tcp_listener.accept().await?;
        if let Err(e) = socket.set_nodelay(self.cfg.nodelay) {
            return Err(Error::from(e));
        }
        Ok(Acceptor {
            socket,
            remote_addr,
            #[cfg(feature = "tls")]
            acceptor: self.tls_acceptor.clone(),
            cfg: self.cfg.clone(),
            typ: self.typ,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.tcp_listener.local_addr()?)
    }
}

/// Connection handler for processing client streams
pub struct Acceptor<S> {
    /// Underlying network transport
    pub(crate) socket: S,
    #[cfg(feature = "tls")]
    acceptor: Option<TlsAcceptor>,
    /// Remote client address
    pub remote_addr: SocketAddr,
    /// Shared server configuration
    pub cfg: Arc<Builder>,
    /// Active protocol type
    pub typ: ListenerType,
}

impl<S> Acceptor<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Creates TCP protocol dispatcher
    #[inline]
    pub fn tcp(self) -> Result<Dispatcher<S>> {
        if matches!(self.typ, ListenerType::TCP) {
            Ok(Dispatcher::new(self.socket, self.remote_addr, self.cfg))
        } else {
            Err(anyhow!("Protocol mismatch: Expected TCP listener"))
        }
    }

    #[cfg(feature = "tls")]
    /// Performs TLS handshake and creates secure dispatcher
    #[inline]
    pub async fn tls(self) -> Result<Dispatcher<TlsStream<S>>> {
        if !matches!(self.typ, ListenerType::TLS) {
            return Err(anyhow!("Protocol mismatch: Expected TLS listener"));
        }

        let acceptor = self.acceptor.ok_or_else(|| crate::MqttError::ServiceUnavailable)?;
        let tls_s = match tokio::time::timeout(self.cfg.handshake_timeout, acceptor.accept(self.socket)).await
        {
            Ok(Ok(tls_s)) => tls_s,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(crate::MqttError::ReadTimeout.into()),
        };
        Ok(Dispatcher::new(tls_s, self.remote_addr, self.cfg))
    }

    #[cfg(feature = "ws")]
    /// Performs WebSocket upgrade and creates WS dispatcher
    #[inline]
    pub async fn ws(self) -> Result<Dispatcher<WsStream<S>>> {
        if !matches!(self.typ, ListenerType::WS) {
            return Err(anyhow!("Protocol mismatch: Expected WS listener"));
        }

        match tokio::time::timeout(self.cfg.handshake_timeout, accept_hdr_async(self.socket, on_handshake))
            .await
        {
            Ok(Ok(ws_stream)) => {
                Ok(Dispatcher::new(WsStream::new(ws_stream), self.remote_addr, self.cfg.clone()))
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err(crate::MqttError::ReadTimeout.into()),
        }
    }

    #[cfg(feature = "tls")]
    #[cfg(feature = "ws")]
    /// Performs TLS handshake and WebSocket upgrade
    #[inline]
    pub async fn wss(self) -> Result<Dispatcher<WsStream<TlsStream<S>>>> {
        if !matches!(self.typ, ListenerType::WSS) {
            return Err(anyhow!("Protocol mismatch: Expected WSS listener"));
        }

        let acceptor = self.acceptor.ok_or_else(|| crate::MqttError::ServiceUnavailable)?;
        let tls_s = match tokio::time::timeout(self.cfg.handshake_timeout, acceptor.accept(self.socket)).await
        {
            Ok(Ok(tls_s)) => tls_s,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(crate::MqttError::ReadTimeout.into()),
        };
        match tokio::time::timeout(self.cfg.handshake_timeout, accept_hdr_async(tls_s, on_handshake)).await {
            Ok(Ok(ws_stream)) => {
                Ok(Dispatcher::new(WsStream::new(ws_stream), self.remote_addr, self.cfg.clone()))
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err(crate::MqttError::ReadTimeout.into()),
        }
    }
}

#[allow(clippy::result_large_err)]
#[cfg(feature = "ws")]
/// Validates WebSocket handshake requests for MQTT protocol
fn on_handshake(req: &Request, mut response: Response) -> std::result::Result<Response, ErrorResponse> {
    const PROTOCOL_ERROR: &str = "Missing required 'Sec-WebSocket-Protocol: mqtt' header";
    let mqtt_protocol = req
        .headers()
        .get("Sec-WebSocket-Protocol")
        .ok_or_else(|| ErrorResponse::new(Some(PROTOCOL_ERROR.into())))?;
    if mqtt_protocol != "mqtt" {
        return Err(ErrorResponse::new(Some(PROTOCOL_ERROR.into())));
    }
    response.headers_mut().append(
        "Sec-WebSocket-Protocol",
        "mqtt".parse().map_err(|_| ErrorResponse::new(Some("InvalidHeaderValue".into())))?,
    );
    Ok(response)
}
