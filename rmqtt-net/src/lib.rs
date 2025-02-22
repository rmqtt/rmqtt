#![deny(unsafe_code)]

mod error;
mod server;
mod stream;
mod ws;

pub use server::{Builder, Listener, TlsListener};
pub use stream::{v3, v5, MqttStream};
pub use error::MqttError;

pub type Error = anyhow::Error;
pub type Result<T> = anyhow::Result<T, Error>;
