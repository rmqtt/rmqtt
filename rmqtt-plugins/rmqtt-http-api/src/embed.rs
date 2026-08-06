//! Embedded Dashboard assets via `rust-embed`.
//!
//! Embeds the `rmqtt-dashboard/` directory directly into the binary at compile time,
//! so the Dashboard SPA can be served without any filesystem dependency.
//!
//! The `#[folder]` path is relative to the crate root
//! (`rmqtt-plugins/rmqtt-http-api/`) and points to `rmqtt-dashboard/`
//! at the workspace root.

use rust_embed::RustEmbed;

/// All files under `rmqtt-dashboard/` embedded at compile time.
#[derive(RustEmbed)]
#[folder = "../../rmqtt-dashboard"]
pub(crate) struct DashboardAssets;
