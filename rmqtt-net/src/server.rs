use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::{NonZeroU16, NonZeroU32};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;

#[cfg(not(target_os = "windows"))]
use rustls::crypto::aws_lc_rs as provider;
#[cfg(target_os = "windows")]
use rustls::crypto::ring as provider;
use rustls::pki_types::pem::PemObject;
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};

use anyhow::anyhow;
use nonzero_ext::nonzero;
use socket2::{Domain, SockAddr, Socket, Type};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

use crate::stream::Dispatcher;
use crate::ws::WsStream;
use crate::{Error, MqttError, Result};

#[derive(Clone, Debug)]
pub struct Builder {
    /// The name of the server.
    pub name: String,
    ///The local address the server listens on.
    pub laddr: SocketAddr,
    ///The maximum length of the TCP connection queue.
    ///It indicates the maximum number of TCP connection queues that are being handshaked three times in the system
    pub backlog: i32,
    ///TCP_NODELAY
    pub nodelay: bool,
    ///Whether to enable the SO_REUSEADDR option.
    pub reuseaddr: Option<bool>,
    ///Whether to enable the SO_REUSEPORT option.
    pub reuseport: Option<bool>,

    pub max_connections: usize,
    ///Maximum concurrent handshake limit, Default: 500
    pub max_handshaking_limit: usize,
    ///Maximum allowed mqtt message length. 0 means unlimited, default: 1M
    pub max_packet_size: u32,

    ///Whether anonymous login is allowed. Default: false
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
    /// max_qos_allowed: QoS,
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

            allow_anonymous: false,
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
            // max_qos_allowed: QoS,
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

    pub fn name(mut self, name: &str) -> Self {
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

    pub fn nodelay(mut self) -> Self {
        self.nodelay = true;
        self
    }

    pub fn reuseaddr(mut self) -> Self {
        self.reuseaddr = Some(true);
        self
    }

    pub fn reuseport(mut self) -> Self {
        self.reuseport = Some(true);
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

    pub fn tls_cross_certificate(mut self) -> Self {
        self.tls_cross_certificate = true;
        self
    }

    pub fn tls_cert(mut self, tls_cert: &str) -> Self {
        self.tls_cert = Some(tls_cert.into());
        self
    }

    pub fn tls_key(mut self, tls_key: &str) -> Self {
        self.tls_key = Some(tls_key.into());
        self
    }

    #[allow(unused_variables)]
    pub fn bind(self) -> Result<Listener> {
        let builder = match self.laddr {
            SocketAddr::V4(_) => Socket::new(Domain::IPV4, Type::STREAM, None)?,
            SocketAddr::V6(_) => Socket::new(Domain::IPV6, Type::STREAM, None)?,
        };

        builder.set_nonblocking(true)?;

        #[cfg(not(windows))]
        if let Some(reuseaddr) = reuseaddr {
            builder.set_reuse_address(reuseaddr)?;
        }

        #[cfg(not(windows))]
        if let Some(reuseport) = reuseport {
            builder.set_reuse_port(reuseport)?;
        }

        builder.bind(&SockAddr::from(self.laddr))?;
        builder.listen(self.backlog)?;
        let l = TcpListener::from_std(std::net::TcpListener::from(builder))?;
        log::info!("Starting {} Listening on {}", self.name, self.laddr);
        Ok(Listener { cfg: Arc::new(self), l })
    }
}

pub struct Listener {
    pub cfg: Arc<Builder>,
    l: TcpListener,
}

impl Listener {
    pub fn tls(self) -> Result<TlsListener> {
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

        Ok(TlsListener { inner: self, acceptor })
    }

    // pub async fn accept_ws(
    //     &self,
    //     tm: Duration,
    // ) -> Result<impl Future<Output = Result<Dispatcher<WsStream<TcpStream>>>> + use<'_>> {
    //     let (socket, remote_addr) = self.l.accept().await?;
    //     if let Err(e) = socket.set_nodelay(self.cfg.nodelay) {
    //         return Err(Error::from(e));
    //     }
    //     Ok(async move {
    //         match tokio::time::timeout(tm, accept_hdr_async(socket, on_handshake)).await {
    //             Ok(Ok(ws_stream)) => {
    //                 Ok(Dispatcher::new(WsStream::new(ws_stream), remote_addr, self.cfg.clone()))
    //             }
    //             Ok(Err(e)) => Err(e.into()),
    //             Err(_) => Err(MqttError::ReadTimeout.into()),
    //         }
    //     })
    // }

    // pub async fn accept(&self) -> Result<Dispatcher<TcpStream>> {
    //     let (socket, remote_addr) = self.l.accept().await?;
    //     if let Err(e) = socket.set_nodelay(self.cfg.nodelay) {
    //         return Err(Error::from(e));
    //     }
    //     Ok(Dispatcher::new(socket, remote_addr, self.cfg.clone()))
    // }

    pub async fn accept(&self) -> Result<Acceptor<TcpStream>> {
        let (socket, remote_addr) = self.l.accept().await?;
        if let Err(e) = socket.set_nodelay(self.cfg.nodelay) {
            return Err(Error::from(e));
        }
        Ok(Acceptor { socket, remote_addr, acceptor: None, cfg: self.cfg.clone() })
    }
}

pub struct Acceptor<S> {
    pub(crate) socket: S,
    acceptor: Option<TlsAcceptor>,
    pub remote_addr: SocketAddr,
    pub cfg: Arc<Builder>,
}

impl<S> Acceptor<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    #[inline]
    pub fn tcp(self) -> Dispatcher<S> {
        Dispatcher::new(self.socket, self.remote_addr, self.cfg)
    }

    #[inline]
    pub async fn tls(self) -> Result<Dispatcher<TlsStream<S>>> {
        let acceptor = self.acceptor.ok_or_else(|| MqttError::ServiceUnavailable)?;
        let tls_s = match tokio::time::timeout(self.cfg.handshake_timeout, acceptor.accept(self.socket)).await
        {
            Ok(Ok(tls_s)) => tls_s,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(MqttError::ReadTimeout.into()),
        };
        Ok(Dispatcher::new(tls_s, self.remote_addr, self.cfg))
    }

    #[inline]
    pub async fn ws(self) -> Result<Dispatcher<WsStream<S>>> {
        match tokio::time::timeout(self.cfg.handshake_timeout, accept_hdr_async(self.socket, on_handshake))
            .await
        {
            Ok(Ok(ws_stream)) => Ok(Dispatcher::new(WsStream::new(ws_stream), self.remote_addr, self.cfg)),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err(MqttError::ReadTimeout.into()),
        }
    }

    #[inline]
    pub async fn wss(self) -> Result<Dispatcher<WsStream<TlsStream<S>>>> {
        let acceptor = self.acceptor.ok_or_else(|| MqttError::ServiceUnavailable)?;
        let tls_s = match tokio::time::timeout(self.cfg.handshake_timeout, acceptor.accept(self.socket)).await
        {
            Ok(Ok(tls_s)) => tls_s,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(MqttError::ReadTimeout.into()),
        };
        match tokio::time::timeout(self.cfg.handshake_timeout, accept_hdr_async(tls_s, on_handshake)).await {
            Ok(Ok(ws_stream)) => Ok(Dispatcher::new(WsStream::new(ws_stream), self.remote_addr, self.cfg)),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err(MqttError::ReadTimeout.into()),
        }
    }
}

pub struct TlsListener {
    inner: Listener,
    acceptor: TlsAcceptor,
}

impl Deref for TlsListener {
    type Target = Listener;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TlsListener {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl TlsListener {
    pub async fn accept(&self) -> Result<Acceptor<TcpStream>> {
        let (socket, remote_addr) = self.l.accept().await?;
        if let Err(e) = socket.set_nodelay(self.cfg.nodelay) {
            return Err(Error::from(e));
        }
        Ok(Acceptor { socket, remote_addr, acceptor: Some(self.acceptor.clone()), cfg: self.cfg.clone() })
    }
}

#[allow(clippy::result_large_err)]
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
