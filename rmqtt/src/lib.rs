#![deny(unsafe_code)]
#![recursion_limit = "256"]

#[macro_use]
extern crate serde;
#[macro_use]
extern crate serde_json;
extern crate core;

pub mod acl;
pub mod context;
pub mod delayed;
pub mod executor;
pub mod extend;
pub mod fitter;
pub mod grpc;
pub mod hook;
pub mod inflight;
pub mod logger;
pub mod message;
pub mod metrics;
pub mod node;
pub mod plugin;
pub mod queue;
pub mod retain;
pub mod router;
pub mod server;
pub mod session;
pub mod shared;
pub mod stats;
pub mod subscribe;
pub mod topic;
pub mod trie;
pub mod types;
pub mod utils;
pub mod v3;
pub mod v5;

pub use crate::types::*;
pub use net::{Error, Result};

pub use rmqtt_codec as codec;
pub use rmqtt_conf as conf;
// pub use rmqtt_grpc as grpc2;
pub use rmqtt_macros as macros;
pub use rmqtt_net as net;
