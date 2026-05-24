//! TCP transport for MQTT v5.0
//!
//! The TCP stream is split into owned read/write halves to avoid deadlocks
//! caused by holding a tokio Mutex across async I/O operations. The reader
//! loop holds the read half exclusively, while send methods share the write
//! half behind a Mutex.

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use rmqtt_codec::v5::Codec as V5Codec;
use rmqtt_codec::v5::Packet as PacketV5;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::time;
use tokio_util::codec::{Decoder, Encoder};

/// Read half of the TCP transport – used exclusively by the reader loop
pub struct TcpTransportV5Reader {
    stream: Option<OwnedReadHalf>,
    read_buf: BytesMut,
    codec: V5Codec,
}

/// Write half of the TCP transport – shared behind a Mutex for concurrent sends
pub struct TcpTransportV5Writer {
    stream: Option<OwnedWriteHalf>,
    write_buf: BytesMut,
    codec: V5Codec,
}

/// Connect to a remote address with a timeout, returning split read/write halves
pub async fn connect(
    addr: &str,
    timeout: Duration,
) -> Result<(TcpTransportV5Reader, TcpTransportV5Writer), anyhow::Error> {
    let stream = time::timeout(timeout, TcpStream::connect(addr)).await??;
    stream.set_nodelay(true)?;
    let (read_half, write_half) = stream.into_split();

    let reader = TcpTransportV5Reader {
        stream: Some(read_half),
        read_buf: BytesMut::with_capacity(8192),
        codec: V5Codec::new(1024 * 1024, 1024 * 1024),
    };
    let writer = TcpTransportV5Writer {
        stream: Some(write_half),
        write_buf: BytesMut::with_capacity(4096),
        codec: V5Codec::new(1024 * 1024, 1024 * 1024),
    };

    Ok((reader, writer))
}

impl TcpTransportV5Reader {
    /// Read data into the internal buffer and return the number of bytes read
    pub async fn recv(&mut self) -> Result<usize, anyhow::Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("not connected"))?;
        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("connection closed by broker"));
        }
        self.read_buf.extend_from_slice(&tmp[..n]);
        tracing::debug!(bytes = %format_hex(&tmp[..n]), "RECV {} bytes", n);
        Ok(n)
    }

    /// Try to decode a complete MQTT packet from the read buffer
    pub fn try_decode(&mut self) -> Result<Option<PacketV5>, anyhow::Error> {
        if self.read_buf.len() < 2 {
            return Ok(None);
        }
        match self.codec.decode(&mut self.read_buf) {
            Ok(Some((packet, _consumed))) => {
                tracing::debug!(packet = %packet_name_v5(&packet), "DECODED");
                Ok(Some(packet))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("decode error: {}", e)),
        }
    }

    /// Read a complete MQTT packet from the stream
    pub async fn read_packet(&mut self) -> Result<PacketV5, anyhow::Error> {
        loop {
            if let Some(packet) = self.try_decode()? {
                return Ok(packet);
            }
            self.recv().await?;
        }
    }

    /// Check if the reader is still connected
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    /// Take the read half (for shutdown)
    pub fn take_stream(&mut self) -> Option<OwnedReadHalf> {
        self.stream.take()
    }
}

impl TcpTransportV5Writer {
    /// Send raw bytes over the TCP stream
    async fn send(&mut self, data: &Bytes) -> Result<(), anyhow::Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("not connected"))?;
        stream.write_all(data).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Encode and send an MQTT packet
    pub async fn send_packet(&mut self, packet: &PacketV5) -> Result<(), anyhow::Error> {
        self.write_buf.clear();
        self.codec
            .encode(packet.clone(), &mut self.write_buf)
            .map_err(|e| anyhow::anyhow!("encode error: {}", e))?;
        tracing::debug!(
            packet = %packet_name_v5(packet),
            bytes = %format_hex(&self.write_buf),
            "SEND"
        );
        let data = self.write_buf.split().freeze();
        self.send(&data).await?;
        Ok(())
    }

    /// Shut down the write half
    pub async fn shutdown(&mut self) -> Result<(), anyhow::Error> {
        if let Some(stream) = self.stream.take() {
            let mut s = stream;
            let _ = s.shutdown().await;
        }
        Ok(())
    }

    /// Check if the writer is still connected
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }
}

/// Get a human-readable name for a packet type
pub(crate) fn packet_name_v5(packet: &PacketV5) -> &'static str {
    use rmqtt_codec::v5::Packet;
    match packet {
        Packet::Connect(_) => "CONNECT",
        Packet::ConnectAck(_) => "CONNACK",
        Packet::Publish(_) => "PUBLISH",
        Packet::PublishAck(_) => "PUBACK",
        Packet::PublishReceived(_) => "PUBREC",
        Packet::PublishRelease(_) => "PUBREL",
        Packet::PublishComplete(_) => "PUBCOMP",
        Packet::Subscribe(_) => "SUBSCRIBE",
        Packet::SubscribeAck(_) => "SUBACK",
        Packet::Unsubscribe(_) => "UNSUBSCRIBE",
        Packet::UnsubscribeAck(_) => "UNSUBACK",
        Packet::PingRequest => "PINGREQ",
        Packet::PingResponse => "PINGRESP",
        Packet::Disconnect(_) => "DISCONNECT",
        Packet::Auth(_) => "AUTH",
    }
}

/// Format bytes as hex string (max 64 bytes shown, truncated if longer)
pub(crate) fn format_hex(data: &[u8]) -> String {
    const MAX_SHOW: usize = 64;
    if data.len() <= MAX_SHOW {
        data.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")
    } else {
        let shown: String = data[..MAX_SHOW]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{}... ({} bytes total)", shown, data.len())
    }
}
