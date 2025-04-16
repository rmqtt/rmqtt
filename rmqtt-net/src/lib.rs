#![deny(unsafe_code)]
//! # MQTT Server Implementation
//!
//! A complete MQTT broker implementation supporting v3.1.1 and v5.0 protocols
//! with TLS and WebSocket capabilities.
//!
//! ## Basic Usage
//!
//! ```rust,no_run
//! use rmqtt_net::{Builder, ListenerType};
//! use std::net::SocketAddr;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let builder = Builder::new()
//!         .name("MyBroker")
//!         .laddr("127.0.0.1:1883".parse()?);
//!
//!     let listener = builder.bind()?;
//!     loop {
//!         let acceptor = listener.accept().await?;
//!         let dispatcher = acceptor.tcp()?;
//!         // Handle connection...
//!     }
//!     Ok(())
//! }
//! ```

mod builder;
mod error;
mod stream;
#[cfg(feature = "ws")]
mod ws;

/// Server configuration and listener management
pub use builder::{Builder, Listener, ListenerType};

/// Error types for MQTT operations
pub use error::MqttError;

/// TLS implementation providers
#[cfg(feature = "tls")]
pub use rustls;

/// AWS-LC based TLS provider (non-Windows platforms)
#[cfg(not(target_os = "windows"))]
#[cfg(feature = "tls")]
pub use rustls::crypto::aws_lc_rs as tls_provider;

/// Ring-based TLS provider (Windows platforms)
#[cfg(target_os = "windows")]
#[cfg(feature = "tls")]
pub use rustls::crypto::ring as tls_provider;

/// MQTT protocol implementations and stream handling
pub use stream::{v3, v5, MqttStream};

/// Convenience type alias for generic errors
pub type Error = anyhow::Error;

/// Result type alias using crate's Error type
pub type Result<T> = anyhow::Result<T, Error>;
