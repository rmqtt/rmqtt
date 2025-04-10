#![deny(unsafe_code)]
#![recursion_limit = "256"]

pub mod acl;
pub mod args;
pub mod context;
#[cfg(feature = "delayed")]
pub mod delayed;
pub mod executor;
pub mod extend;
pub mod fitter;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod hook;
pub mod inflight;
#[cfg(feature = "msgstore")]
pub mod message;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod node;
#[cfg(feature = "plugin")]
pub mod plugin;
pub mod queue;
#[cfg(feature = "retain")]
pub mod retain;
pub mod router;
pub mod server;
pub mod session;
pub mod shared;
#[cfg(feature = "stats")]
pub mod stats;
#[cfg(any(feature = "auto-subscription", feature = "shared-subscription"))]
pub mod subscribe;
pub mod topic;
pub mod trie;
pub mod types;
pub mod v3;
pub mod v5;

// pub use crate::types::*;
pub use net::{Error, Result};

pub use rmqtt_codec as codec;
#[cfg(feature = "conf")]
pub use rmqtt_conf as conf;
#[cfg(any(feature = "metrics", feature = "plugin"))]
pub use rmqtt_macros as macros;
pub use rmqtt_net as net;
pub use rmqtt_utils as utils;
