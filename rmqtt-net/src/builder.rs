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

#[derive(Clone, Debug)]
pub struct Builder {
    /// The name of the server.
    pub name: String,
    ///The local address the server listens on.
    pub laddr: SocketAddr,
    ///The maximum length of the TCP connection queue.
    ///It indicates the maximum number of TCP connection queues that are being handshaked three times in the system
    pub backlog: i32,
    ///Sets the value of the TCP_NODELAY option on this socket.
    pub nodelay: bool,
    ///Whether to enable the SO_REUSEADDR option.
    pub reuseaddr: Option<bool>,
    ///Whether to enable the SO_REUSEPORT option.
    pub reuseport: Option<bool>,
    ///The maximum number of concurrent connections allowed by the listener.
    pub max_connections: usize,
    ///Maximum concurrent handshake limit, Default: 500
    pub max_handshaking_limit: usize,
    ///Maximum allowed mqtt message length. 0 means unlimited, default: 1M
    pub max_packet_size: u32,

    ///Whether anonymous login is allowed. Default: true
    pub allow_anonymous: bool,
    ///Minimum allowable keepalive value for mqtt connection,
    ///less than this value will reject the connection(MQTT V3),
    ///less than this value will set keepalive to this value in CONNACK (MQTT V5),
    ///default: 0, unit: seconds
    pub min_keepalive: u16,
    ///Maximum allowable keepalive value for mqtt connection,
    ///greater than this value will reject the connection(MQTT V3),
    ///greater than this value will set keepalive to this value in CONNACK (MQTT V5),
    ///default value: 65535, unit: seconds
    pub max_keepalive: u16,
    ///A value of zero indicates disabling the keep-alive feature, where the server
    ///doesn't need to disconnect due to client inactivity, default: true
    pub allow_zero_keepalive: bool,
    ///# > 0.5, Keepalive * backoff * 2, Default: 0.75
    pub keepalive_backoff: f32,
    ///Flight window size. The flight window is used to store the unanswered QoS 1 and QoS 2 messages
    pub max_inflight: NonZeroU16,
    ///Handshake timeout.
    pub handshake_timeout: Duration,
    ///Send timeout.
    pub send_timeout: Duration,
    ///Maximum length of message queue
    pub max_mqueue_len: usize,
    ///The rate at which messages are ejected from the message queue,
    ///default value: "u32::max_value(),1s"
    pub mqueue_rate_limit: (NonZeroU32, Duration),
    ///Maximum length of client ID allowed, Default: 65535
    pub max_clientid_len: usize,
    ///The maximum QoS level that clients are allowed to publish. default value: 2
    pub max_qos_allowed: QoS,
    ///The maximum level at which clients are allowed to subscribe to topics.
    ///0 means unlimited. default value: 0
    pub max_topic_levels: usize,
    ///Session timeout, default value: 2 hours
    pub session_expiry_interval: Duration,
    ///QoS 1/2 message retry interval, 0 means no resend
    pub message_retry_interval: Duration,
    ///Message expiration time, 0 means no expiration, default value: 5 minutes
    pub message_expiry_interval: Duration,
    ///0 means unlimited, default value: 0
    pub max_subscriptions: usize,
    ///Shared subscription switch, default value: true
    pub shared_subscription: bool,
    ///topic alias maximum, default value: 0, topic aliases not enabled. (MQTT 5.0)
    pub max_topic_aliases: u16,
    ///Limit subscription switch, default value: false
    pub limit_subscription: bool,
    ///Delayed publish switch, default value: false
    pub delayed_publish: bool,

    ///Whether to enable cross-certification, default value: false
    pub tls_cross_certificate: bool,
    ///This certificate is used to authenticate the server during TLS handshakes.
    pub tls_cert: Option<String>,
    ///This key is used to establish a secure connection with the client.
    pub tls_key: Option<String>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            name: Default::default(),
            laddr: SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 1883)),
            max_connections: 1_000_000,
            max_handshaking_limit: 1_000,
            max_packet_size: 1024 * 1024, //"1M"
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

    pub fn name<N: Into<String>>(mut self, name: N) -> Self {
        self.name = name.into();
        self
    }

    pub fn laddr(mut self, laddr: SocketAddr) -> Self {
        self.laddr = laddr;
        self
    }

    pub fn backlog(mut self, backlog: i32) -> Self {
        self.backlog = backlog;
        self
    }

    pub fn nodelay(mut self, nodelay: bool) -> Self {
        self.nodelay = nodelay;
        self
    }

    pub fn reuseaddr(mut self, reuseaddr: Option<bool>) -> Self {
        self.reuseaddr = reuseaddr;
        self
    }

    pub fn reuseport(mut self, reuseport: Option<bool>) -> Self {
        self.reuseport = reuseport;
        self
    }

    pub fn max_connections(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }

    pub fn max_handshaking_limit(mut self, max_handshaking_limit: usize) -> Self {
        self.max_handshaking_limit = max_handshaking_limit;
        self
    }

    pub fn max_packet_size(mut self, max_packet_size: u32) -> Self {
        self.max_packet_size = max_packet_size;
        self
    }

    pub fn allow_anonymous(mut self, allow_anonymous: bool) -> Self {
        self.allow_anonymous = allow_anonymous;
        self
    }

    pub fn min_keepalive(mut self, min_keepalive: u16) -> Self {
        self.min_keepalive = min_keepalive;
        self
    }

    pub fn max_keepalive(mut self, max_keepalive: u16) -> Self {
        self.max_keepalive = max_keepalive;
        self
    }

    pub fn allow_zero_keepalive(mut self, allow_zero_keepalive: bool) -> Self {
        self.allow_zero_keepalive = allow_zero_keepalive;
        self
    }

    pub fn keepalive_backoff(mut self, keepalive_backoff: f32) -> Self {
        self.keepalive_backoff = keepalive_backoff;
        self
    }

    pub fn max_inflight(mut self, max_inflight: NonZeroU16) -> Self {
        self.max_inflight = max_inflight;
        self
    }

    pub fn handshake_timeout(mut self, handshake_timeout: Duration) -> Self {
        self.handshake_timeout = handshake_timeout;
        self
    }

    pub fn send_timeout(mut self, send_timeout: Duration) -> Self {
        self.send_timeout = send_timeout;
        self
    }

    pub fn max_mqueue_len(mut self, max_mqueue_len: usize) -> Self {
        self.max_mqueue_len = max_mqueue_len;
        self
    }

    pub fn mqueue_rate_limit(mut self, rate_limit: NonZeroU32, duration: Duration) -> Self {
        self.mqueue_rate_limit = (rate_limit, duration);
        self
    }

    pub fn max_clientid_len(mut self, max_clientid_len: usize) -> Self {
        self.max_clientid_len = max_clientid_len;
        self
    }

    pub fn max_qos_allowed(mut self, max_qos_allowed: QoS) -> Self {
        self.max_qos_allowed = max_qos_allowed;
        self
    }

    pub fn max_topic_levels(mut self, max_topic_levels: usize) -> Self {
        self.max_topic_levels = max_topic_levels;
        self
    }

    pub fn session_expiry_interval(mut self, session_expiry_interval: Duration) -> Self {
        self.session_expiry_interval = session_expiry_interval;
        self
    }

    pub fn message_retry_interval(mut self, message_retry_interval: Duration) -> Self {
        self.message_retry_interval = message_retry_interval;
        self
    }

    pub fn message_expiry_interval(mut self, message_expiry_interval: Duration) -> Self {
        self.message_expiry_interval = message_expiry_interval;
        self
    }

    pub fn max_subscriptions(mut self, max_subscriptions: usize) -> Self {
        self.max_subscriptions = max_subscriptions;
        self
    }

    pub fn shared_subscription(mut self, shared_subscription: bool) -> Self {
        self.shared_subscription = shared_subscription;
        self
    }

    pub fn max_topic_aliases(mut self, max_topic_aliases: u16) -> Self {
        self.max_topic_aliases = max_topic_aliases;
        self
    }

    pub fn limit_subscription(mut self, limit_subscription: bool) -> Self {
        self.limit_subscription = limit_subscription;
        self
    }

    pub fn delayed_publish(mut self, delayed_publish: bool) -> Self {
        self.delayed_publish = delayed_publish;
        self
    }

    pub fn tls_cross_certificate(mut self, cross_certificate: bool) -> Self {
        self.tls_cross_certificate = cross_certificate;
        self
    }

    pub fn tls_cert<N: Into<String>>(mut self, tls_cert: Option<N>) -> Self {
        self.tls_cert = tls_cert.map(|c| c.into());
        self
    }

    pub fn tls_key<N: Into<String>>(mut self, tls_key: Option<N>) -> Self {
        self.tls_key = tls_key.map(|c| c.into());
        self
    }

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
        log::info!("MQTT Broker Listening on {} {}", self.name, self.laddr);
        Ok(Listener {
            typ: ListenerType::TCP,
            cfg: Arc::new(self),
            tcp_listener,
            #[cfg(feature = "tls")]
            tls_acceptor: None,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ListenerType {
    TCP,
    #[cfg(feature = "tls")]
    TLS,
    #[cfg(feature = "ws")]
    WS,
    #[cfg(feature = "tls")]
    #[cfg(feature = "ws")]
    WSS,
}

pub struct Listener {
    pub typ: ListenerType,
    pub cfg: Arc<Builder>,
    tcp_listener: TcpListener,
    #[cfg(feature = "tls")]
    tls_acceptor: Option<TlsAcceptor>,
}

impl Listener {
    pub fn tcp(mut self) -> Result<Self> {
        let _err = anyhow!(
                "Upgrading from ListenerType::TLS or ListenerType::WS or ListenerType::WSS to ListenerType::TCP is not allowed."
            );
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
    pub fn ws(mut self) -> Result<Self> {
        if matches!(self.typ, ListenerType::TCP | ListenerType::WS) {
            self.typ = ListenerType::WS;
        } else {
            return Err(anyhow!(
                "Upgrading from ListenerType::TLS or ListenerType::WSS to ListenerType::WS is not allowed."
            ));
        }
        Ok(self)
    }

    #[cfg(feature = "tls")]
    #[cfg(feature = "ws")]
    pub fn wss(mut self) -> Result<Self> {
        if matches!(self.typ, ListenerType::TCP | ListenerType::WS) {
            self = self.tls()?;
        }
        self.typ = ListenerType::WSS;
        Ok(self)
    }

    #[cfg(feature = "tls")]
    pub fn tls(mut self) -> Result<Listener> {
        match self.typ {
            #[cfg(feature = "ws")]
            ListenerType::WS | ListenerType::WSS => {
                return Err(anyhow!(
                    "Upgrading from ListenerType::WS or ListenerType::WSS to ListenerType::TLS is not allowed."
                ));
            }
            ListenerType::TLS => return Ok(self),
            ListenerType::TCP => {}
        }

        let cert_file = self.cfg.tls_cert.as_ref().ok_or(anyhow!("tls cert filename is None"))?;
        let key_file = self.cfg.tls_key.as_ref().ok_or(anyhow!("tls key filename is None"))?;

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
            .map_err(|e| anyhow!(format!("bad certs/private key, {}", e)))?;

        let acceptor = TlsAcceptor::from(Arc::new(tls_config));
        self.tls_acceptor = Some(acceptor);
        self.typ = ListenerType::TLS;
        Ok(self)
    }

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
}

pub struct Acceptor<S> {
    pub(crate) socket: S,
    #[cfg(feature = "tls")]
    acceptor: Option<TlsAcceptor>,
    pub remote_addr: SocketAddr,
    pub cfg: Arc<Builder>,
    pub typ: ListenerType,
}

impl<S> Acceptor<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    #[inline]
    pub fn tcp(self) -> Result<Dispatcher<S>> {
        if matches!(self.typ, ListenerType::TCP) {
            Ok(Dispatcher::new(self.socket, self.remote_addr, self.cfg))
        } else {
            Err(anyhow!("Mismatched ListenerType"))
        }
    }

    #[inline]
    #[cfg(feature = "tls")]
    pub async fn tls(self) -> Result<Dispatcher<TlsStream<S>>> {
        if !matches!(self.typ, ListenerType::TLS) {
            return Err(anyhow!("Mismatched ListenerType"));
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

    #[inline]
    #[cfg(feature = "ws")]
    pub async fn ws(self) -> Result<Dispatcher<WsStream<S>>> {
        if !matches!(self.typ, ListenerType::WS) {
            return Err(anyhow!("Mismatched ListenerType"));
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

    #[inline]
    #[cfg(feature = "tls")]
    #[cfg(feature = "ws")]
    pub async fn wss(self) -> Result<Dispatcher<WsStream<TlsStream<S>>>> {
        if !matches!(self.typ, ListenerType::WSS) {
            return Err(anyhow!("Mismatched ListenerType"));
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
fn on_handshake(req: &Request, mut response: Response) -> std::result::Result<Response, ErrorResponse> {
    const PROTOCOL_ERROR: &str = "No \"Sec-WebSocket-Protocol: mqtt\" in client request";
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
