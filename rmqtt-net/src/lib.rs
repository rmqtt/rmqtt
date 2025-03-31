#![deny(unsafe_code)]

mod builder;
mod error;
mod stream;
#[cfg(feature = "ws")]
mod ws;

pub use builder::{Builder, Listener, ListenerType};
pub use error::MqttError;
#[cfg(feature = "tls")]
pub use rustls;
#[cfg(not(target_os = "windows"))]
#[cfg(feature = "tls")]
pub use rustls::crypto::aws_lc_rs as tls_provider;
#[cfg(target_os = "windows")]
#[cfg(feature = "tls")]
pub use rustls::crypto::ring as tls_provider;

pub use stream::{v3, v5, MqttStream};

pub type Error = anyhow::Error;
pub type Result<T> = anyhow::Result<T, Error>;
