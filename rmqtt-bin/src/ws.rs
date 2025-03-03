use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{
    io::{self, ErrorKind},
    marker,
    time::Duration,
};

use rmqtt::bytes::Bytes;
use rmqtt::futures::{ready, FutureExt, Sink, Stream};
use rmqtt::ntex::codec::ReadBuf;
use rmqtt::ntex::codec::{AsyncRead, AsyncWrite};
use rmqtt::ntex::rt::time::{sleep, Sleep};
use rmqtt::ntex::util::Ready;
use rmqtt::ntex::{Service, ServiceFactory};
use rmqtt::ntex_mqtt;
use rmqtt::pin_project_lite;
use rmqtt::tokio_tungstenite::accept_hdr_async;
use rmqtt::tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use rmqtt::tokio_tungstenite::tungstenite::Error as WSError;
use rmqtt::tokio_tungstenite::tungstenite::Message;
use rmqtt::tokio_tungstenite::WebSocketStream;
use rmqtt::{log, MqttError};

pub struct WSServer<T> {
    timeout: Duration,
    io: marker::PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite> WSServer<T> {
    pub fn new(timeout: Duration) -> Self {
        WSServer { timeout, io: marker::PhantomData }
    }
}

impl<T> Clone for WSServer<T> {
    fn clone(&self) -> Self {
        Self { timeout: self.timeout, io: marker::PhantomData }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> ServiceFactory for WSServer<T> {
    type Request = T;
    type Response = WsStream<T>;
    type Error = ntex_mqtt::MqttError<MqttError>;
    type Config = ();

    type Service = WSService<T>;
    type InitError = ();
    type Future = Ready<Self::Service, Self::InitError>;

    fn new_service(&self, _: ()) -> Self::Future {
        Ready::Ok(WSService { timeout: self.timeout, io: marker::PhantomData })
    }
}

pub struct WSService<T> {
    io: marker::PhantomData<T>,
    timeout: Duration,
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> Service for WSService<T> {
    type Request = T;
    type Response = WsStream<T>;
    type Error = ntex_mqtt::MqttError<MqttError>;
    type Future = WSServiceFut<T>;

    #[inline]
    fn poll_ready(&self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&self, req: Self::Request) -> Self::Future {
        WSServiceFut {
            fut: accept_hdr_async(req, on_handshake).boxed_local(),
            delay: if self.timeout == Duration::ZERO { None } else { Some(sleep(self.timeout)) },
        }
    }
}

type WebSocketStreamType<T> = Pin<Box<dyn Future<Output = Result<WebSocketStream<T>, WSError>>>>;

pin_project_lite::pin_project! {
    pub struct WSServiceFut<T>
    where
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,
    {
        fut: WebSocketStreamType<T>,
        #[pin]
        delay: Option<Sleep>,
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Future for WSServiceFut<T> {
    type Output = Result<WsStream<T>, ntex_mqtt::MqttError<MqttError>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let Some(delay) = this.delay.as_pin_mut() {
            match delay.poll(cx) {
                Poll::Pending => (),
                Poll::Ready(_) => return Poll::Ready(Err(ntex_mqtt::MqttError::HandshakeTimeout)),
            }
        }
        match Pin::new(&mut this.fut).poll(cx) {
            Poll::Ready(Ok(io)) => Poll::Ready(Ok(WsStream::new(io))),
            Poll::Ready(Err(e)) => Poll::Ready(Err(ntex_mqtt::MqttError::Service(MqttError::from(e)))),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub struct WsStream<S> {
    s: WebSocketStream<S>,
    cached_data: Option<Bytes>,
    idx: usize,
}

impl<S> WsStream<S> {
    pub fn new(s: WebSocketStream<S>) -> Self {
        Self { s, cached_data: None, idx: 0 }
    }
}

impl<S> WsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    #[inline]
    pub fn get_ref(&self) -> &S {
        self.s.get_ref()
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
            if cached_buf.len() <= buf.remaining() {
                buf.put_slice(cached_buf);
                self.idx = 0;
                self.cached_data = None;
            } else {
                let cached_buf = &cached_buf[0..buf.remaining()];
                buf.put_slice(cached_buf);
                self.idx += cached_buf.len();
            }
            return Poll::Ready(Ok(()));
        }

        match ready!(Pin::new(&mut self.s).poll_next(cx)) {
            Some(Ok(msg)) => {
                let data = msg.into_data();
                if data.len() <= buf.remaining() {
                    buf.put_slice(data.as_ref());
                } else {
                    let enable_buf = &data[0..buf.remaining()];
                    buf.put_slice(enable_buf);
                    self.idx = enable_buf.len();
                    self.cached_data = Some(data)
                }
                Poll::Ready(Ok(()))
            }
            Some(Err(e)) => {
                log::debug!("{:?}", e);
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
        if let Err(e) = Pin::new(&mut self.s).start_send(Message::Binary(buf.to_vec().into())) {
            return Poll::Ready(Err(to_error(e)));
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if let Err(e) = ready!(Pin::new(&mut self.s).poll_flush(cx)) {
            return Poll::Ready(Err(to_error(e)));
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if let Err(e) = ready!(Pin::new(&mut self.s).poll_close(cx)) {
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
        _ => io::Error::new(ErrorKind::Other, e.to_string()),
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
