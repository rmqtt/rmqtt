#![deny(unsafe_code)]
#![recursion_limit = "256"]

extern crate proc_macro;
#[macro_use]
extern crate serde;
#[macro_use]
pub extern crate serde_json;
#[macro_use]
pub extern crate async_trait;

pub use ahash;
pub use anyhow;
pub use base64;
pub use bincode;
pub use bytes;
pub use bytestring;
pub use chrono;
pub use crossbeam;
pub use dashmap;
pub use futures;
pub use itertools;
pub use log;
pub use ntex;
pub use ntex_mqtt;
pub use once_cell;
pub use pin_project_lite;
pub use rand;
pub use reqwest;
pub use rust_box;
pub use structopt;
pub use tokio;
pub use tokio_cron_scheduler;
pub use tokio_tungstenite;
pub use url;

pub use crate::broker::{
    error::MqttError,
    metrics,
    session::{Session, SessionState},
    stats,
    types::*,
};
pub use crate::runtime::Runtime;

pub type Result<T, E = MqttError> = anyhow::Result<T, E>;

pub mod broker;
pub mod extend;
pub mod grpc;
pub mod logger;
pub mod node;
pub mod plugin;
pub mod runtime;
pub mod settings;
