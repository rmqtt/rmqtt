use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use futures::SinkExt;
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use rmqtt_codec::error::{DecodeError, SendPacketError};
use rmqtt_codec::v3::Codec as CodecV3;
use rmqtt_codec::v5::Codec as CodecV5;
use rmqtt_codec::version::{ProtocolVersion, VersionCodec};
use rmqtt_codec::{MqttCodec, MqttPacket};

use crate::error::MqttError;
use crate::CertInfo;
use crate::{Builder, Result};

/// MQTT protocol dispatcher handling version negotiation
///
/// Manages initial protocol detection and creates version-specific streams
pub struct Dispatcher<Io> {
    /// Framed IO layer with MQTT codec
    pub(crate) io: Framed<Io, MqttCodec>,
    /// Remote client's network address
    pub remote_addr: SocketAddr,
    /// Shared configuration builder
    pub cfg: Arc<Builder>,

    pub cert_info: Option<CertInfo>,
}

impl<Io> Dispatcher<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    /// Creates a new Dispatcher instance
    pub(crate) fn new(
        io: Io,
        remote_addr: SocketAddr,
        cert_info: Option<CertInfo>,
        cfg: Arc<Builder>,
    ) -> Self {
        Dispatcher { io: Framed::new(io, MqttCodec::Version(VersionCodec)), remote_addr, cfg, cert_info }
    }

    /// Negotiates protocol version and returns appropriate stream
    #[inline]
    pub async fn mqtt(mut self) -> Result<MqttStream<Io>> {
        Ok(match self.probe_version().await? {
            ProtocolVersion::MQTT3 => MqttStream::V3(v3::MqttStream {
                io: self.io,
                remote_addr: self.remote_addr,
                cfg: self.cfg,
                #[cfg(feature = "tls")]
                cert_info: self.cert_info,
            }),
            ProtocolVersion::MQTT5 => MqttStream::V5(v5::MqttStream {
                io: self.io,
                remote_addr: self.remote_addr,
                cfg: self.cfg,
                #[cfg(feature = "tls")]
                cert_info: self.cert_info,
            }),
        })
    }

    /// Detects protocol version from initial handshake
    #[inline]
    async fn probe_version(&mut self) -> Result<ProtocolVersion> {
        let Some(Ok((MqttPacket::Version(ver), _))) = self.io.next().await else {
            return Err(anyhow!(DecodeError::InvalidProtocol));
        };

        let codec = match ver {
            ProtocolVersion::MQTT3 => MqttCodec::V3(CodecV3::new(self.cfg.max_packet_size)),
            ProtocolVersion::MQTT5 => {
                MqttCodec::V5(CodecV5::new(self.cfg.max_packet_size, self.cfg.max_packet_size))
            }
        };

        *self.io.codec_mut() = codec;
        Ok(ver)
    }
}

/// Version-specific MQTT protocol streams
pub enum MqttStream<Io> {
    /// MQTT v3.1.1 implementation
    V3(v3::MqttStream<Io>),
    /// MQTT v5.0 implementation
    V5(v5::MqttStream<Io>),
}

pub mod v3 {

    use std::net::SocketAddr;
    use std::num::NonZeroU16;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use futures::StreamExt;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio_util::codec::Framed;

    use rmqtt_codec::error::DecodeError;
    use rmqtt_codec::types::Publish;
    use rmqtt_codec::v3::{Connect, ConnectAckReason, Packet as PacketV3, Packet};
    use rmqtt_codec::{MqttCodec, MqttPacket};

    use crate::error::MqttError;
    use crate::{Builder, Error, Result};

    #[cfg(feature = "tls")]
    use crate::CertInfo;

    /// MQTT v3.1.1 protocol stream implementation
    pub struct MqttStream<Io> {
        /// Framed IO layer with MQTT codec
        pub io: Framed<Io, MqttCodec>,
        /// Remote client's network address
        pub remote_addr: SocketAddr,
        /// Shared configuration builder
        pub cfg: Arc<Builder>,
        #[cfg(feature = "tls")]
        /// TLS certificate information (if available)
        pub cert_info: Option<CertInfo>,
    }

    /// # Examples
    /// ```
    /// use std::net::{SocketAddr, IpAddr, Ipv4Addr};
    /// use std::sync::Arc;
    /// use tokio::net::TcpStream;
    /// use tokio_util::codec::Framed;
    /// use rmqtt_codec::{MqttCodec, types::Publish};
    /// use rmqtt_net::{Builder,v3};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1883);
    /// let stream = TcpStream::connect(addr).await?;
    /// let mut mqtt_stream = v3::MqttStream {
    ///     io: Framed::new(stream, MqttCodec::V3(Default::default())),
    ///     remote_addr: addr,
    ///     cfg: Arc::new(Builder::default()),
    /// };
    ///
    /// // Send a PING request
    /// mqtt_stream.send_ping_request().await?;
    /// # Ok(())
    /// # }
    /// ```
    impl<Io> MqttStream<Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        /// Sends DISCONNECT packet and flushes buffers
        #[inline]
        pub async fn send_disconnect(&mut self) -> Result<()> {
            self.send(PacketV3::Disconnect).await?;
            self.flush().await
        }

        /// Publishes a message to the broker
        #[inline]
        pub async fn send_publish(&mut self, publish: Box<Publish>) -> Result<()> {
            self.send(PacketV3::Publish(publish)).await
        }

        /// Acknowledges a received publish (QoS 1)
        #[inline]
        pub async fn send_publish_ack(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishAck { packet_id }).await
        }

        /// Confirms receipt of a publish (QoS 2 step 1)
        #[inline]
        pub async fn send_publish_received(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishReceived { packet_id }).await
        }

        /// Releases a stored publish (QoS 2 step 2)
        #[inline]
        pub async fn send_publish_release(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishRelease { packet_id }).await
        }

        /// Confirms publish completion (QoS 2 step 3)
        #[inline]
        pub async fn send_publish_complete(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishComplete { packet_id }).await
        }

        /// Acknowledges a subscription request
        #[inline]
        pub async fn send_subscribe_ack(
            &mut self,
            packet_id: NonZeroU16,
            status: Vec<rmqtt_codec::v3::SubscribeReturnCode>,
        ) -> Result<()> {
            self.send(PacketV3::SubscribeAck { packet_id, status }).await
        }

        /// Acknowledges an unsubscribe request
        #[inline]
        pub async fn send_unsubscribe_ack(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::UnsubscribeAck { packet_id }).await
        }

        /// Initiates connection to the broker
        #[inline]
        pub async fn send_connect(&mut self, connect: rmqtt_codec::v3::Connect) -> Result<()> {
            self.send(PacketV3::Connect(Box::new(connect))).await
        }

        /// Responds to connection request
        #[inline]
        pub async fn send_connect_ack(
            &mut self,
            return_code: ConnectAckReason,
            session_present: bool,
        ) -> Result<()> {
            self.send(PacketV3::ConnectAck(rmqtt_codec::v3::ConnectAck { session_present, return_code }))
                .await
        }

        /// Sends keep-alive ping request
        #[inline]
        pub async fn send_ping_request(&mut self) -> Result<()> {
            self.send(PacketV3::PingRequest {}).await
        }

        /// Responds to ping request
        #[inline]
        pub async fn send_ping_response(&mut self) -> Result<()> {
            self.send(PacketV3::PingResponse {}).await
        }

        /// Generic packet sending method
        #[inline]
        pub async fn send(&mut self, packet: rmqtt_codec::v3::Packet) -> Result<()> {
            super::send(&mut self.io, MqttPacket::V3(packet), self.cfg.send_timeout).await
        }

        /// Flushes write buffers
        #[inline]
        pub async fn flush(&mut self) -> Result<()> {
            super::flush(&mut self.io, self.cfg.send_timeout).await
        }

        /// Closes the connection gracefully
        #[inline]
        pub async fn close(&mut self) -> Result<()> {
            super::close(&mut self.io, self.cfg.send_timeout).await
        }

        /// Receives next packet with timeout
        #[inline]
        pub async fn recv(&mut self, tm: Duration) -> Result<Option<rmqtt_codec::v3::Packet>> {
            match tokio::time::timeout(tm, self.next()).await {
                Ok(Some(Ok(msg))) => Ok(Some(msg)),
                Ok(Some(Err(e))) => Err(e),
                Ok(None) => Ok(None),
                Err(_) => Err(MqttError::ReadTimeout.into()),
            }
        }

        /// Waits for CONNECT packet with timeout
        #[inline]
        pub async fn recv_connect(&mut self, tm: Duration) -> Result<Box<Connect>> {
            let connect = match self.recv(tm).await {
                Ok(Some(Packet::Connect(mut connect))) => {
                    #[cfg(feature = "tls")]
                    {
                        if self.cfg.cert_cn_as_username {
                            if let Some(cert) = &self.cert_info {
                                if let Some(cn) = &cert.common_name {
                                    connect.username = Some(cn.clone().into());
                                }
                            }
                        }
                    }
                    connect
                }
                Err(e) => {
                    return Err(e);
                }
                _ => {
                    return Err(MqttError::InvalidProtocol.into());
                }
            };
            Ok(connect)
        }
    }

    impl<Io> futures::Stream for MqttStream<Io>
    where
        Io: AsyncRead + Unpin,
    {
        type Item = Result<rmqtt_codec::v3::Packet>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let next = Pin::new(&mut self.io).poll_next(cx);
            Poll::Ready(match futures::ready!(next) {
                Some(Ok((MqttPacket::V3(packet), _))) => Some(Ok(packet)),
                Some(Ok(_)) => Some(Err(MqttError::Decode(DecodeError::MalformedPacket).into())),
                Some(Err(e)) => Some(Err(Error::from(e))),
                None => None,
            })
        }
    }
}

pub mod v5 {
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use futures::StreamExt;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio_util::codec::Framed;

    use rmqtt_codec::error::DecodeError;
    use rmqtt_codec::types::Publish;
    use rmqtt_codec::v5::{Auth, Connect, Disconnect, Packet as PacketV5, Packet};
    use rmqtt_codec::{MqttCodec, MqttPacket};

    use crate::error::MqttError;
    use crate::{Builder, CertInfo, Error, Result};

    /// MQTT v5.0 protocol stream implementation
    pub struct MqttStream<Io> {
        /// Framed IO layer with MQTT codec
        pub io: Framed<Io, MqttCodec>,
        /// Remote client's network address
        pub remote_addr: SocketAddr,
        /// Shared configuration builder
        pub cfg: Arc<Builder>,
        #[cfg(feature = "tls")]
        /// TLS certificate information (if available)
        pub cert_info: Option<CertInfo>,
    }

    /// # Examples
    /// ```
    /// use std::net::{SocketAddr, IpAddr, Ipv4Addr};
    /// use std::sync::Arc;
    /// use tokio::net::TcpStream;
    /// use tokio_util::codec::Framed;
    /// use rmqtt_codec::{MqttCodec, types::Publish};
    /// use rmqtt_net::{Builder,v5};
    /// use rmqtt_codec::v5::Connect;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1883);
    /// let stream = TcpStream::connect(addr).await?;
    /// let mut mqtt_stream = v5::MqttStream {
    ///     io: Framed::new(stream, MqttCodec::V5(Default::default())),
    ///     remote_addr: addr,
    ///     cfg: Arc::new(Builder::default()),
    /// };
    ///
    /// // Send authentication packet
    /// mqtt_stream.send_auth(rmqtt_codec::v5::Auth::default()).await?;
    /// # Ok(())
    /// # }
    /// ```
    impl<Io> MqttStream<Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        /// Sends DISCONNECT packet with reason code
        #[inline]
        pub async fn send_disconnect(&mut self, disc: Disconnect) -> Result<()> {
            self.send(PacketV5::Disconnect(disc)).await?;
            self.flush().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(())
        }

        /// Publishes a message to the broker
        #[inline]
        pub async fn send_publish(&mut self, publish: Box<Publish>) -> Result<()> {
            self.send(PacketV5::Publish(publish)).await
        }

        /// Acknowledges a received publish (QoS 1)
        #[inline]
        pub async fn send_publish_ack(&mut self, ack: rmqtt_codec::v5::PublishAck) -> Result<()> {
            self.send(PacketV5::PublishAck(ack)).await
        }

        /// Confirms receipt of a publish (QoS 2 step 1)
        #[inline]
        pub async fn send_publish_received(&mut self, ack: rmqtt_codec::v5::PublishAck) -> Result<()> {
            self.send(PacketV5::PublishReceived(ack)).await
        }

        /// Releases a stored publish (QoS 2 step 2)
        #[inline]
        pub async fn send_publish_release(&mut self, ack2: rmqtt_codec::v5::PublishAck2) -> Result<()> {
            self.send(PacketV5::PublishRelease(ack2)).await
        }

        /// Confirms publish completion (QoS 2 step 3)
        #[inline]
        pub async fn send_publish_complete(&mut self, ack2: rmqtt_codec::v5::PublishAck2) -> Result<()> {
            self.send(PacketV5::PublishComplete(ack2)).await
        }

        /// Acknowledges a subscription request
        #[inline]
        pub async fn send_subscribe_ack(&mut self, ack: rmqtt_codec::v5::SubscribeAck) -> Result<()> {
            self.send(PacketV5::SubscribeAck(ack)).await
        }

        /// Acknowledges an unsubscribe request
        #[inline]
        pub async fn send_unsubscribe_ack(&mut self, unack: rmqtt_codec::v5::UnsubscribeAck) -> Result<()> {
            self.send(PacketV5::UnsubscribeAck(unack)).await
        }

        /// Initiates connection to the broker
        #[inline]
        pub async fn send_connect(&mut self, connect: rmqtt_codec::v5::Connect) -> Result<()> {
            self.send(PacketV5::Connect(Box::new(connect))).await
        }

        /// Responds to connection request
        #[inline]
        pub async fn send_connect_ack(&mut self, ack: rmqtt_codec::v5::ConnectAck) -> Result<()> {
            self.send(PacketV5::ConnectAck(Box::new(ack))).await
        }

        /// Sends keep-alive ping request
        #[inline]
        pub async fn send_ping_request(&mut self) -> Result<()> {
            self.send(PacketV5::PingRequest {}).await
        }

        /// Responds to ping request
        #[inline]
        pub async fn send_ping_response(&mut self) -> Result<()> {
            self.send(PacketV5::PingResponse {}).await
        }

        /// Sends authentication exchange packet
        #[inline]
        pub async fn send_auth(&mut self, auth: Auth) -> Result<()> {
            self.send(PacketV5::Auth(auth)).await
        }

        /// Generic packet sending method
        #[inline]
        pub async fn send(&mut self, packet: rmqtt_codec::v5::Packet) -> Result<()> {
            super::send(&mut self.io, MqttPacket::V5(packet), self.cfg.send_timeout).await
        }

        /// Flushes write buffers
        #[inline]
        pub async fn flush(&mut self) -> Result<()> {
            super::flush(&mut self.io, self.cfg.send_timeout).await
        }

        /// Closes the connection gracefully
        #[inline]
        pub async fn close(&mut self) -> Result<()> {
            super::close(&mut self.io, self.cfg.send_timeout).await
        }

        /// Receives next packet with timeout
        #[inline]
        pub async fn recv(&mut self, tm: Duration) -> Result<Option<rmqtt_codec::v5::Packet>> {
            match tokio::time::timeout(tm, self.next()).await {
                Ok(Some(Ok(msg))) => Ok(Some(msg)),
                Ok(Some(Err(e))) => Err(e),
                Ok(None) => Ok(None),
                Err(_) => Err(MqttError::ReadTimeout.into()),
            }
        }

        /// Waits for CONNECT packet with timeout
        #[inline]
        pub async fn recv_connect(&mut self, tm: Duration) -> Result<Box<Connect>> {
            let connect = match self.recv(tm).await {
                Ok(Some(Packet::Connect(connect))) => connect,
                Err(e) => {
                    return Err(e);
                }
                _ => {
                    return Err(MqttError::InvalidProtocol.into());
                }
            };
            Ok(connect)
        }
    }

    impl<Io> futures::Stream for MqttStream<Io>
    where
        Io: AsyncRead + Unpin,
    {
        type Item = Result<rmqtt_codec::v5::Packet>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let next = Pin::new(&mut self.io).poll_next(cx);
            Poll::Ready(match futures::ready!(next) {
                Some(Ok((MqttPacket::V5(packet), _))) => Some(Ok(packet)),
                Some(Ok(_)) => Some(Err(MqttError::Decode(DecodeError::MalformedPacket).into())),
                Some(Err(e)) => Some(Err(Error::from(e))),
                None => None,
            })
        }
    }
}

#[inline]
async fn send<Io>(io: &mut Framed<Io, MqttCodec>, packet: MqttPacket, send_timeout: Duration) -> Result<()>
where
    Io: AsyncWrite + Unpin,
{
    if send_timeout.is_zero() {
        io.send(packet).await?;
        Ok(())
    } else {
        match tokio::time::timeout(send_timeout, io.send(packet)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(MqttError::SendPacket(SendPacketError::Encode(e))),
            Err(_) => Err(MqttError::WriteTimeout),
        }?;
        Ok(())
    }
}

#[inline]
async fn flush<Io>(io: &mut Framed<Io, MqttCodec>, send_timeout: Duration) -> Result<()>
where
    Io: AsyncWrite + Unpin,
{
    if send_timeout.is_zero() {
        io.flush().await?;
        Ok(())
    } else {
        match tokio::time::timeout(send_timeout, io.flush()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(MqttError::SendPacket(SendPacketError::Encode(e))),
            Err(_) => Err(MqttError::FlushTimeout),
        }?;
        Ok(())
    }
}

#[inline]
async fn close<Io>(io: &mut Framed<Io, MqttCodec>, send_timeout: Duration) -> Result<()>
where
    Io: AsyncWrite + Unpin,
{
    if send_timeout.is_zero() {
        io.close().await?;
        Ok(())
    } else {
        match tokio::time::timeout(send_timeout, io.close()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(MqttError::Encode(e)),
            Err(_) => Err(MqttError::CloseTimeout),
        }?;
        Ok(())
    }
}
