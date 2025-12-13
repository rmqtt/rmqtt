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

pub struct WsStream<S> {
    inner: WebSocketStream<S>,
    cached_data: Option<Bytes>,
    idx: usize,
}

impl<S> WsStream<S> {
    pub fn new(inner: WebSocketStream<S>) -> Self {
        Self { inner, cached_data: None, idx: 0 }
    }

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
                let data = msg.into_data();
                let remaining = buf.remaining();
                if data.len() <= remaining {
                    buf.put_slice(data.as_ref());
                } else {
                    let cached_buf = &data[0..remaining];
                    buf.put_slice(cached_buf);
                    self.idx = cached_buf.len();
                    self.cached_data = Some(data)
                }
                Poll::Ready(Ok(()))
            }
            Some(Err(e)) => {
                log::warn!("{e:?}");
                Poll::Ready(Err(to_error(e)))
            }
            None => Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))),
        }
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
