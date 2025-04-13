#![deny(unsafe_code)] // Enforce memory safety across the entire crate
#![recursion_limit = "256"] // Allow deeper recursion for complex macros

//! # Overall Example
//! ```rust,no_run
//!
//! use rmqtt::context::ServerContext;
//! use rmqtt::net::{Builder, Result};
//! use rmqtt::server::MqttServer;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!    
//!    let scx = ServerContext::new().build().await;
//!
//!    MqttServer::new(scx)
//!       .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
//!       .listener(Builder::new().name("internal/tcp").laddr(([0, 0, 0, 0], 11883).into()).bind()?.tcp()?)
//!       .listener(Builder::new().name("external/ws").laddr(([0, 0, 0, 0], 8080).into()).bind()?.ws()?)
//!       .build()
//!       .run()
//!       .await?;
//!       Ok(())
//! }
//!
//! ```

/// Core MQTT Broker Components
pub mod acl; // Access Control List management
pub mod args; // Command-line argument parsing
pub mod context; // Shared execution context

/// Feature-gated Modules
#[cfg(feature = "delayed")] // Delayed message publishing
pub mod delayed;
#[cfg(feature = "grpc")] // gRPC API integration
pub mod grpc;
#[cfg(feature = "msgstore")] // Message storage subsystem
pub mod message;
#[cfg(feature = "metrics")] // Metrics collection and reporting
pub mod metrics;
#[cfg(feature = "plugin")] // Plugin system infrastructure
pub mod plugin;
#[cfg(feature = "retain")] // Retained message handling
pub mod retain;
#[cfg(feature = "stats")] // Runtime statistics tracking
pub mod stats;

/// Essential Services
pub mod executor; // Async task executor
pub mod extend; // Extension points
pub mod fitter; // Message fitting strategies
pub mod hook; // Event hook system
pub mod inflight; // In-flight message tracking
pub mod node; // Cluster node management
pub mod queue; // Message queue implementation
pub mod router; // Message routing core
pub mod server; // Server lifecycle management
pub mod session; // Client session handling
pub mod shared; // Shared state management

/// Subscription Management
#[cfg(any(feature = "auto-subscription", feature = "shared-subscription"))]
pub mod subscribe; // Subscription services

/// Topic Handling
pub mod topic; // Topic parsing and validation
pub mod trie; // Topic trie structure

/// Protocol Support
pub mod types; // Common data types
pub mod v3; // MQTT v3.1.1 implementation
pub mod v5; // MQTT v5.0 implementation

/// External Crate Re-exports
pub use net::{Error, Result}; // Network error types

/// Feature-gated Re-exports
pub use rmqtt_codec as codec; // MQTT protocol codec
#[cfg(any(feature = "metrics", feature = "plugin"))] // Macro utilities
pub use rmqtt_macros as macros;
pub use rmqtt_net as net; // Network abstractions
pub use rmqtt_utils as utils; // Common utilities
