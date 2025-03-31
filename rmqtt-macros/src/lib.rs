#![deny(unsafe_code)]
extern crate proc_macro;
extern crate proc_macro2;

#[cfg(feature = "metrics")]
mod metrics;
#[cfg(feature = "plugin")]
mod plugin;

#[cfg(feature = "metrics")]
#[proc_macro_derive(Metrics)]
pub fn derive_metrics(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    metrics::build(input)
}

#[cfg(feature = "plugin")]
#[proc_macro_derive(Plugin)]
pub fn derive_plugin(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    plugin::build(input)
}
