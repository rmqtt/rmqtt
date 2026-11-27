//! WebSocket stream adapter wrapping `tokio_tungstenite` for MQTT transport.
//!
//! Provides [`WsStream`] which implements [`AsyncRead`] and [`AsyncWrite`] over a
//! WebSocket connection, translating WebSocket binary messages into a byte-stream
//! interface suitable for MQTT protocol codecs.

use std::io::{self, ErrorKind};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::{ready, Sink, Stream};
use tokio::io::ReadBuf;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::Error as WSError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::bytes;

/// How many frames a single [`WsStream::poll_read`] call may consume and skip
/// (empty data frames, WebSocket control frames) before it yields. Bounds the
/// work done per poll so a peer flooding such frames cannot starve the task.
const MAX_SKIPPED_FRAMES: usize = 64;

/// WebSocket stream adapter implementing [`AsyncRead`] and [`AsyncWrite`] for MQTT.
///
/// Wraps a `tokio_tungstenite` `WebSocketStream` and translates binary WebSocket messages
/// into a continuous byte stream, with internal buffering for fragmented messages.
pub struct WsStream<S> {
    inner: WebSocketStream<S>,
    cached_data: Option<Bytes>,
    idx: usize,
}

impl<S> WsStream<S> {
    /// Creates a new `WsStream` wrapping the given WebSocket stream
    pub fn new(inner: WebSocketStream<S>) -> Self {
        Self { inner, cached_data: None, idx: 0 }
    }

    /// Returns a reference to the underlying `WebSocketStream`
    pub fn get_inner(&self) -> &WebSocketStream<S> {
        &self.inner
    }
}

impl<S> AsyncRead for WsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // No path below may report a read of zero bytes unless the stream is
        // really over: `AsyncRead` defines that as EOF, and `Framed` — the MQTT
        // codec driving this adapter — acts on it by ending the session. A
        // zero-length data frame is legal (RFC 6455) and paho.golang writes one
        // per packet buffer, so it must be consumed and skipped instead of being
        // returned as "0 bytes read" (issue #551).
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        for _ in 0..MAX_SKIPPED_FRAMES {
            if let Some(cached_data) = &self.cached_data {
                let cached_buf = &cached_data[self.idx..];
                let remaining = buf.remaining();
                if cached_buf.len() <= remaining {
                    buf.put_slice(cached_buf);
                    self.idx = 0;
                    self.cached_data = None;
                } else {
                    let cached_buf = &cached_buf[0..remaining];
                    buf.put_slice(cached_buf);
                    self.idx += cached_buf.len();
                }
                return Poll::Ready(Ok(()));
            }

            match ready!(Pin::new(&mut self.inner).poll_next(cx)) {
                Some(Ok(msg)) => {
                    if msg.is_close() {
                        // A genuine close: EOF is the honest answer, and it is
                        // the only frame allowed to produce one.
                        return Poll::Ready(Ok(()));
                    }
                    if msg.is_ping() || msg.is_pong() {
                        // Control frames never carry MQTT bytes. Their payload
                        // would desync the decoder if spliced into the stream
                        // (`into_data()` returns it for every frame type), and
                        // tungstenite answers a Ping on its own.
                        continue;
                    }
                    let data = msg.into_data();
                    if data.is_empty() {
                        // Empty data frame: nothing to hand over, keep reading.
                        continue;
                    }
                    let remaining = buf.remaining();
                    if data.len() <= remaining {
                        buf.put_slice(data.as_ref());
                    } else {
                        let cached_buf = &data[0..remaining];
                        buf.put_slice(cached_buf);
                        self.idx = cached_buf.len();
                        self.cached_data = Some(data)
                    }
                    return Poll::Ready(Ok(()));
                }
                Some(Err(e)) => {
                    log::warn!("{e}");
                    return Poll::Ready(Err(to_error(e)));
                }
                None => return Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))),
            }
        }

        // Skip budget exhausted: continue on the next poll rather than looping.
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

impl<S> AsyncWrite for WsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        // An empty write has nothing to send; emitting a zero-length frame would
        // only give the peer something to misread (see `poll_read`, issue #551).
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if let Err(e) = Pin::new(&mut self.inner).start_send(Message::Binary(buf.to_vec().into())) {
            return Poll::Ready(Err(to_error(e)));
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if let Err(e) = ready!(Pin::new(&mut self.inner).poll_flush(cx)) {
            return Poll::Ready(Err(to_error(e)));
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if let Err(e) = ready!(Pin::new(&mut self.inner).poll_close(cx)) {
            return Poll::Ready(Err(to_error(e)));
        }
        Poll::Ready(Ok(()))
    }
}

fn to_error(e: WSError) -> io::Error {
    match e {
        WSError::ConnectionClosed => io::Error::from(ErrorKind::ConnectionAborted),
        WSError::AlreadyClosed => io::Error::from(ErrorKind::NotConnected),
        WSError::Io(io_e) => io_e,
        _ => io::Error::other(e.to_string()),
    }
}
