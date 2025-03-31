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
use crate::{Builder, Result};

pub struct Dispatcher<Io> {
    pub(crate) io: Framed<Io, MqttCodec>,
    pub remote_addr: SocketAddr,
    pub cfg: Arc<Builder>,
}

impl<Io> Dispatcher<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    pub(crate) fn new(io: Io, remote_addr: SocketAddr, cfg: Arc<Builder>) -> Self {
        Dispatcher { io: Framed::new(io, MqttCodec::Version(VersionCodec)), remote_addr, cfg }
    }

    #[inline]
    pub async fn mqtt(mut self) -> Result<MqttStream<Io>> {
        Ok(match self.probe_version().await? {
            ProtocolVersion::MQTT3 => {
                MqttStream::V3(v3::MqttStream { io: self.io, remote_addr: self.remote_addr, cfg: self.cfg })
            }
            ProtocolVersion::MQTT5 => {
                MqttStream::V5(v5::MqttStream { io: self.io, remote_addr: self.remote_addr, cfg: self.cfg })
            }
        })
    }

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
    //
    // #[inline]
    // pub async fn send_disconnect(&mut self, disc: Option<Disconnect>) -> Result<()> {
    //     match self.ver {
    //         Some(ProtocolVersion::MQTT3) => self.send(MqttPacket::V3(PacketV3::Disconnect)).await,
    //         Some(ProtocolVersion::MQTT5) => {
    //             self.send(MqttPacket::V5(PacketV5::Disconnect(
    //                 disc.unwrap_or_default(),
    //             )))
    //             .await
    //         }
    //         None => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_publish(&mut self, publish: Publish) -> Result<()> {
    //     match self.ver {
    //         Some(ProtocolVersion::MQTT3) => {
    //             self.send(MqttPacket::V3(PacketV3::Publish(publish))).await
    //         }
    //         Some(ProtocolVersion::MQTT5) => {
    //             self.send(MqttPacket::V5(PacketV5::Publish(publish))).await
    //         }
    //         None => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_publish_ack(&mut self, ack: PublishAck) -> Result<()> {
    //     match (self.ver, ack) {
    //         (Some(ProtocolVersion::MQTT3), PublishAck::V3 { packet_id }) => {
    //             self.send(MqttPacket::V3(PacketV3::PublishAck { packet_id }))
    //                 .await
    //         }
    //         (Some(ProtocolVersion::MQTT5), PublishAck::V5(ack)) => {
    //             self.send(MqttPacket::V5(PacketV5::PublishAck(ack))).await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_publish_received(&mut self, ack: PublishAck) -> Result<()> {
    //     match (self.ver, ack) {
    //         (Some(ProtocolVersion::MQTT3), PublishAck::V3 { packet_id }) => {
    //             self.send(MqttPacket::V3(PacketV3::PublishReceived { packet_id }))
    //                 .await
    //         }
    //         (Some(ProtocolVersion::MQTT5), PublishAck::V5(ack)) => {
    //             self.send(MqttPacket::V5(PacketV5::PublishReceived(ack)))
    //                 .await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_publish_release(&mut self, ack2: PublishAck2) -> Result<()> {
    //     match (self.ver, ack2) {
    //         (Some(ProtocolVersion::MQTT3), PublishAck2::V3 { packet_id }) => {
    //             self.send(MqttPacket::V3(PacketV3::PublishRelease { packet_id }))
    //                 .await
    //         }
    //         (Some(ProtocolVersion::MQTT5), PublishAck2::V5(ack2)) => {
    //             self.send(MqttPacket::V5(PacketV5::PublishRelease(ack2)))
    //                 .await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_publish_complete(&mut self, ack2: PublishAck2) -> Result<()> {
    //     match (self.ver, ack2) {
    //         (Some(ProtocolVersion::MQTT3), PublishAck2::V3 { packet_id }) => {
    //             self.send(MqttPacket::V3(PacketV3::PublishComplete { packet_id }))
    //                 .await
    //         }
    //         (Some(ProtocolVersion::MQTT5), PublishAck2::V5(ack2)) => {
    //             self.send(MqttPacket::V5(PacketV5::PublishComplete(ack2)))
    //                 .await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_subscribe_ack(&mut self, ack: SubscribeAck) -> Result<()> {
    //     match (self.ver, ack) {
    //         (Some(ProtocolVersion::MQTT3), SubscribeAck::V3 { packet_id, status }) => {
    //             self.send(MqttPacket::V3(PacketV3::SubscribeAck { packet_id, status }))
    //                 .await
    //         }
    //         (Some(ProtocolVersion::MQTT5), SubscribeAck::V5(ack)) => {
    //             self.send(MqttPacket::V5(PacketV5::SubscribeAck(ack))).await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_unsubscribe_ack(&mut self, unack: UnsubscribeAck) -> Result<()> {
    //     match (self.ver, unack) {
    //         (Some(ProtocolVersion::MQTT3), UnsubscribeAck::V3 { packet_id }) => {
    //             self.send(MqttPacket::V3(PacketV3::UnsubscribeAck { packet_id }))
    //                 .await
    //         }
    //         (Some(ProtocolVersion::MQTT5), UnsubscribeAck::V5(unack)) => {
    //             self.send(MqttPacket::V5(PacketV5::UnsubscribeAck(unack)))
    //                 .await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_connect(&mut self, connect: Connect) -> Result<()> {
    //     match (self.ver, connect) {
    //         (Some(ProtocolVersion::MQTT3), Connect::V3(connect)) => {
    //             self.send(MqttPacket::V3(PacketV3::Connect(Box::new(connect))))
    //                 .await
    //         }
    //         (Some(ProtocolVersion::MQTT5), Connect::V5(connect)) => {
    //             self.send(MqttPacket::V5(PacketV5::Connect(Box::new(connect))))
    //                 .await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_connect_ack(&mut self, ack: ConnectAck) -> Result<()> {
    //     match (self.ver, ack) {
    //         (Some(ProtocolVersion::MQTT3), ConnectAck::V3(ack)) => {
    //             self.send(MqttPacket::V3(PacketV3::ConnectAck(ack))).await
    //         }
    //         (Some(ProtocolVersion::MQTT5), ConnectAck::V5(ack)) => {
    //             self.send(MqttPacket::V5(PacketV5::ConnectAck(Box::new(ack))))
    //                 .await
    //         }
    //         _ => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_ping_request(&mut self) -> Result<()> {
    //     match self.ver {
    //         Some(ProtocolVersion::MQTT3) => {
    //             self.send(MqttPacket::V3(PacketV3::PingRequest {})).await
    //         }
    //         Some(ProtocolVersion::MQTT5) => {
    //             self.send(MqttPacket::V5(PacketV5::PingRequest {})).await
    //         }
    //         None => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_ping_response(&mut self) -> Result<()> {
    //     match self.ver {
    //         Some(ProtocolVersion::MQTT3) => {
    //             self.send(MqttPacket::V3(PacketV3::PingResponse {})).await
    //         }
    //         Some(ProtocolVersion::MQTT5) => {
    //             self.send(MqttPacket::V5(PacketV5::PingResponse {})).await
    //         }
    //         None => Err(MqttError::MissingVersion.into()),
    //     }
    // }
    //
    // #[inline]
    // pub async fn send_auth(&mut self, auth: Auth) -> Result<()> {
    //     self.send(MqttPacket::V5(PacketV5::Auth(auth))).await
    // }
    //
    // #[inline]
    // pub async fn send(&mut self, packet: MqttPacket) -> Result<()> {
    //     if self.cfg.send_timeout.is_zero() {
    //         self.io.send(packet).await?;
    //         Ok(())
    //     } else {
    //         match tokio::time::timeout(self.cfg.send_timeout, self.io.send(packet)).await {
    //             Ok(Ok(())) => Ok(()),
    //             Ok(Err(e)) => Err(MqttError::SendPacket(SendPacketError::Encode(e))),
    //             Err(_) => Err(MqttError::WriteTimeout),
    //         }?;
    //         Ok(())
    //     }
    // }
    //
    // #[inline]
    // pub async fn flush(&mut self) -> Result<()> {
    //     if self.cfg.send_timeout.is_zero() {
    //         self.io.flush().await?;
    //         Ok(())
    //     } else {
    //         match tokio::time::timeout(self.cfg.send_timeout, self.io.flush()).await {
    //             Ok(Ok(())) => Ok(()),
    //             Ok(Err(e)) => Err(MqttError::SendPacket(SendPacketError::Encode(e))),
    //             Err(_) => Err(MqttError::FlushTimeout),
    //         }?;
    //         Ok(())
    //     }
    // }
    //
    // #[inline]
    // pub async fn close(&mut self) -> Result<()> {
    //     if self.cfg.send_timeout.is_zero() {
    //         self.io.close().await?;
    //         Ok(())
    //     } else {
    //         match tokio::time::timeout(self.cfg.send_timeout, self.io.close()).await {
    //             Ok(Ok(())) => Ok(()),
    //             Ok(Err(e)) => Err(MqttError::Encode(e)),
    //             Err(_) => Err(MqttError::CloseTimeout),
    //         }?;
    //         Ok(())
    //     }
    // }
    //
    // #[inline]
    // pub async fn recv_v3(&mut self) -> Option<Result<PacketV3>> {
    //     match self.io.next().await {
    //         Some(Ok((MqttPacket::V3(p), _))) => Some(Ok(p)),
    //         Some(Ok((MqttPacket::V5(_), _))) => Some(Err(DecodeError::MalformedPacket.into())),
    //         Some(Ok((MqttPacket::Version(_), _))) => Some(Err(DecodeError::MalformedPacket.into())),
    //         Some(Err(e)) => Some(Err(e.into())),
    //         None => None,
    //     }
    // }
    //
    // #[inline]
    // pub async fn recv_v5(&mut self) -> Option<Result<PacketV5>> {
    //     match self.io.next().await {
    //         Some(Ok((MqttPacket::V5(p), _))) => Some(Ok(p)),
    //         Some(Ok((MqttPacket::V3(_), _))) => Some(Err(DecodeError::MalformedPacket.into())),
    //         Some(Ok((MqttPacket::Version(_), _))) => Some(Err(DecodeError::MalformedPacket.into())),
    //         Some(Err(e)) => Some(Err(e.into())),
    //         None => None,
    //     }
    // }
}

pub enum MqttStream<Io> {
    V3(v3::MqttStream<Io>),
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

    pub struct MqttStream<Io> {
        pub io: Framed<Io, MqttCodec>,
        pub remote_addr: SocketAddr,
        pub cfg: Arc<Builder>,
    }

    impl<Io> MqttStream<Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        #[inline]
        pub async fn send_disconnect(&mut self) -> Result<()> {
            self.send(PacketV3::Disconnect).await
        }

        #[inline]
        pub async fn send_publish(&mut self, publish: Box<Publish>) -> Result<()> {
            self.send(PacketV3::Publish(publish)).await
        }

        #[inline]
        pub async fn send_publish_ack(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishAck { packet_id }).await
        }

        #[inline]
        pub async fn send_publish_received(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishReceived { packet_id }).await
        }

        #[inline]
        pub async fn send_publish_release(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishRelease { packet_id }).await
        }

        #[inline]
        pub async fn send_publish_complete(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::PublishComplete { packet_id }).await
        }

        #[inline]
        pub async fn send_subscribe_ack(
            &mut self,
            packet_id: NonZeroU16,
            status: Vec<rmqtt_codec::v3::SubscribeReturnCode>,
        ) -> Result<()> {
            self.send(PacketV3::SubscribeAck { packet_id, status }).await
        }

        #[inline]
        pub async fn send_unsubscribe_ack(&mut self, packet_id: NonZeroU16) -> Result<()> {
            self.send(PacketV3::UnsubscribeAck { packet_id }).await
        }

        #[inline]
        pub async fn send_connect(&mut self, connect: rmqtt_codec::v3::Connect) -> Result<()> {
            self.send(PacketV3::Connect(Box::new(connect))).await
        }

        #[inline]
        pub async fn send_connect_ack(
            &mut self,
            return_code: ConnectAckReason,
            session_present: bool,
        ) -> Result<()> {
            self.send(PacketV3::ConnectAck(rmqtt_codec::v3::ConnectAck { session_present, return_code }))
                .await
        }

        #[inline]
        pub async fn send_ping_request(&mut self) -> Result<()> {
            self.send(PacketV3::PingRequest {}).await
        }

        #[inline]
        pub async fn send_ping_response(&mut self) -> Result<()> {
            self.send(PacketV3::PingResponse {}).await
        }

        #[inline]
        pub async fn send(&mut self, packet: rmqtt_codec::v3::Packet) -> Result<()> {
            super::send(&mut self.io, MqttPacket::V3(packet), self.cfg.send_timeout).await
        }

        #[inline]
        pub async fn flush(&mut self) -> Result<()> {
            super::flush(&mut self.io, self.cfg.send_timeout).await
        }

        #[inline]
        pub async fn close(&mut self) -> Result<()> {
            super::close(&mut self.io, self.cfg.send_timeout).await
        }

        #[inline]
        pub async fn recv(&mut self, tm: Duration) -> Result<Option<rmqtt_codec::v3::Packet>> {
            match tokio::time::timeout(tm, self.next()).await {
                Ok(Some(Ok(msg))) => Ok(Some(msg)),
                Ok(Some(Err(e))) => Err(e),
                Ok(None) => Ok(None),
                Err(_) => Err(MqttError::ReadTimeout.into()),
            }
        }

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
    use crate::{Builder, Error, Result};

    pub struct MqttStream<Io> {
        pub io: Framed<Io, MqttCodec>,
        pub remote_addr: SocketAddr,
        pub cfg: Arc<Builder>,
    }

    impl<Io> MqttStream<Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        #[inline]
        pub async fn send_disconnect(&mut self, disc: Disconnect) -> Result<()> {
            self.send(PacketV5::Disconnect(disc)).await
        }

        #[inline]
        pub async fn send_publish(&mut self, publish: Box<Publish>) -> Result<()> {
            self.send(PacketV5::Publish(publish)).await
        }

        #[inline]
        pub async fn send_publish_ack(&mut self, ack: rmqtt_codec::v5::PublishAck) -> Result<()> {
            self.send(PacketV5::PublishAck(ack)).await
        }

        #[inline]
        pub async fn send_publish_received(&mut self, ack: rmqtt_codec::v5::PublishAck) -> Result<()> {
            self.send(PacketV5::PublishReceived(ack)).await
        }

        #[inline]
        pub async fn send_publish_release(&mut self, ack2: rmqtt_codec::v5::PublishAck2) -> Result<()> {
            self.send(PacketV5::PublishRelease(ack2)).await
        }

        #[inline]
        pub async fn send_publish_complete(&mut self, ack2: rmqtt_codec::v5::PublishAck2) -> Result<()> {
            self.send(PacketV5::PublishComplete(ack2)).await
        }

        #[inline]
        pub async fn send_subscribe_ack(&mut self, ack: rmqtt_codec::v5::SubscribeAck) -> Result<()> {
            self.send(PacketV5::SubscribeAck(ack)).await
        }

        #[inline]
        pub async fn send_unsubscribe_ack(&mut self, unack: rmqtt_codec::v5::UnsubscribeAck) -> Result<()> {
            self.send(PacketV5::UnsubscribeAck(unack)).await
        }

        #[inline]
        pub async fn send_connect(&mut self, connect: rmqtt_codec::v5::Connect) -> Result<()> {
            self.send(PacketV5::Connect(Box::new(connect))).await
        }

        #[inline]
        pub async fn send_connect_ack(&mut self, ack: rmqtt_codec::v5::ConnectAck) -> Result<()> {
            self.send(PacketV5::ConnectAck(Box::new(ack))).await
        }

        #[inline]
        pub async fn send_ping_request(&mut self) -> Result<()> {
            self.send(PacketV5::PingRequest {}).await
        }

        #[inline]
        pub async fn send_ping_response(&mut self) -> Result<()> {
            self.send(PacketV5::PingResponse {}).await
        }

        #[inline]
        pub async fn send_auth(&mut self, auth: Auth) -> Result<()> {
            self.send(PacketV5::Auth(auth)).await
        }

        #[inline]
        pub async fn send(&mut self, packet: rmqtt_codec::v5::Packet) -> Result<()> {
            super::send(&mut self.io, MqttPacket::V5(packet), self.cfg.send_timeout).await
        }

        #[inline]
        pub async fn flush(&mut self) -> Result<()> {
            super::flush(&mut self.io, self.cfg.send_timeout).await
        }

        #[inline]
        pub async fn close(&mut self) -> Result<()> {
            super::close(&mut self.io, self.cfg.send_timeout).await
        }

        #[inline]
        pub async fn recv(&mut self, tm: Duration) -> Result<Option<rmqtt_codec::v5::Packet>> {
            match tokio::time::timeout(tm, self.next()).await {
                Ok(Some(Ok(msg))) => Ok(Some(msg)),
                Ok(Some(Err(e))) => Err(e),
                Ok(None) => Ok(None),
                Err(_) => Err(MqttError::ReadTimeout.into()),
            }
        }

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
