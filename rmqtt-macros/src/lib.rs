extern crate proc_macro;
extern crate proc_macro2;

mod metrics;

#[proc_macro_derive(Metrics)]
pub fn derive_metrics(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    metrics::build(input)
}
