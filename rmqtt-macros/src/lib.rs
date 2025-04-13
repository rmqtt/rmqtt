#![deny(unsafe_code)]
//! # Procedural Macro Utilities
//!
//! Provides custom derive macros for metrics collection and plugin systems.
//! Features must be explicitly enabled through crate features.
//!
//! ## Available Features
//! - `metrics`: Enables metrics collection derive macro
//! - `plugin`: Enables plugin system derive macro
//!
//! ## Example Usage
//! ```rust,ignore
//! #[cfg(feature = "metrics")]
//! #[derive(Metrics)]
//! struct NetworkMetrics {
//!     bytes_sent: Counter,
//!     bytes_received: Counter,
//! }
//!
//! #[cfg(feature = "plugin")]
//! #[derive(Plugin)]
//! struct MyPlugin {
//!     config: PluginConfig,
//! }
//! ```

extern crate proc_macro;
extern crate proc_macro2;

#[cfg(feature = "metrics")]
mod metrics;
#[cfg(feature = "plugin")]
mod plugin;

/// Derive macro for implementing metrics collection
///
/// # Examples
/// ```rust,ignore
/// #[cfg(feature = "metrics")]
/// #[derive(Metrics)]
/// struct ServiceMetrics {
///     requests: Counter,
///     latency: Histogram,
///     errors: Gauge,
/// }
/// ```
#[cfg(feature = "metrics")]
#[proc_macro_derive(Metrics)]
pub fn derive_metrics(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    metrics::build(input)
}

/// Derive macro for creating plugin system interfaces
///
/// # Examples
/// ```rust,ignore
/// #[cfg(feature = "plugin")]
/// #[derive(Plugin)]
/// struct AuthPlugin {
///     priority: i32,
///     enabled: bool,
/// }
/// ```
#[cfg(feature = "plugin")]
#[proc_macro_derive(Plugin)]
pub fn derive_plugin(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    plugin::build(input)
}
