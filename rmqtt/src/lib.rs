#![deny(unsafe_code)] // Enforce memory safety across the entire crate
#![recursion_limit = "256"] // Allow deeper recursion for complex macros

//! RMQTT Broker Core Implementation (v2025.04)  
//!  
//! Implements high-performance MQTT broker architecture with full protocol compliance (v3.1.1 & v5.0),  
//! designed for mission-critical IoT systems and large-scale distributed deployments. Key features:  
//!  
//! 1. **Protocol Engine**  
//!    - Dual-stack MQTT v3/v5 support via `v3`/`v5` modules  
//!    - Zero-copy codec implementation from `rmqtt_codec`  
//!    - QoS 0/1/2 message handling with `inflight` tracking  
//!
//!  
//! 2. **Enterprise Features**  
//!    - Distributed session management via `shared` module  
//!    - Cluster node coordination in `node` module  
//!    - TLS/SSL support with certificate validation  
//!    - Retained message store (`retain` feature)  
//!  
//! 3. **Extensibility**  
//!    - Plugin system architecture (`plugin` module)  
//!    - Custom authentication hooks (`acl` module)  
//!    - Metrics collection pipeline (`metrics` feature)  
//!
//!  
//! [MQTT Spec Compliance](https://docs.oasis-open.org/mqtt/mqtt/v5.0/os/mqtt-v5.0-os.html)  
//!
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

pub mod acl; // Access Control List management
pub mod args; // Command-line argument parsing
pub mod context; // Shared execution context

// Feature-gated Modules
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

// Essential Services
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

// Subscription Management
#[cfg(any(feature = "auto-subscription", feature = "shared-subscription"))]
pub mod subscribe; // Subscription services

// Topic Handling
pub mod topic; // Topic parsing and validation
pub mod trie; // Topic trie structure

// Protocol Support
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
