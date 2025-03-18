#![deny(unsafe_code)]

#[macro_use]
extern crate serde;

mod builder;
mod error;
mod stream;
mod ws;

pub use builder::{Builder, Listener, ListenerType};
pub use error::MqttError;
pub use stream::{v3, v5, MqttStream};

pub type Error = anyhow::Error;
pub type Result<T> = anyhow::Result<T, Error>;
