//! Embedded Dashboard assets via `rust-embed`.
//!
//! Embeds the `dashboard/` directory directly into the binary at compile time,
//! so the Dashboard can be served without any filesystem dependency.
//! `dashboard/` is the SINGLE source of the Dashboard SPA — it lives inside
//! this crate and is edited here directly.
//!
//! IMPORTANT: `dashboard/` must live INSIDE this crate. A path pointing
//! outside the crate (e.g. `../../rmqtt-dashboard`) breaks `cargo publish`,
//! because the packaged tarball only contains files inside the crate root and
//! the verify step re-compiles under `target/package/rmqtt-http-api-<ver>/`.

use rust_embed::RustEmbed;

/// All files under `dashboard/` embedded at compile time.
#[derive(RustEmbed)]
#[folder = "dashboard"]
pub(crate) struct DashboardAssets;
