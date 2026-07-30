//! QUIC bidirectional stream adapter for MQTT over QUIC transport.
//!
//! Wraps Quinn's separate [`SendStream`] and [`RecvStream`] into a single [`QuinnBiStream`]
//! that implements [`AsyncRead`] and [`AsyncWrite`], allowing it to be used as a uniform
//! I/O transport for MQTT connections.

use quinn::{RecvStream, SendStream};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Bidirectional QUIC stream wrapping separate send/recv streams into a unified I/O type.
///
/// Combines Quinn's [`SendStream`] and [`RecvStream`] into a single struct that
/// implements [`AsyncRead`] and [`AsyncWrite`], enabling use as a transport for MQTT.
#[allow(dead_code)]
pub struct QuinnBiStream {
    send: SendStream,
    recv: RecvStream,
}

impl QuinnBiStream {
    /// Creates a new `QuinnBiStream` from Quinn send and receive stream halves
    #[allow(dead_code)]
    pub fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
    }
}

impl AsyncRead for QuinnBiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.recv).poll_read_buf(cx, buf).map_err(std::io::Error::other)
    }
}

impl AsyncWrite for QuinnBiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.send).poll_write(cx, buf).map_err(std::io::Error::other)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.send).poll_flush(cx).map_err(std::io::Error::other)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.send).poll_shutdown(cx).map_err(std::io::Error::other)
    }
}

impl Unpin for QuinnBiStream {}
