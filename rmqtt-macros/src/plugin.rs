use quote::quote;
use syn::{parse_macro_input, DeriveInput};

pub(crate) fn build(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = input.ident;

    let expanded = quote! {

        impl rmqtt::plugin::PackageInfo for #name {
            #[inline]
            fn name(&self) -> &str {
                env!("CARGO_PKG_NAME")
            }

            #[inline]
            fn version(&self) -> &str {
                env!("CARGO_PKG_VERSION")
            }

            #[inline]
            fn descr(&self) -> Option<&str> {
                option_env!("CARGO_PKG_DESCRIPTION").and_then(|descr| {
                    if descr.is_empty() {
                        None
                    } else {
                        Some(descr)
                    }
                })
            }

            #[inline]
            fn authors(&self) -> Option<Vec<&str>> {
                option_env!("CARGO_PKG_AUTHORS").and_then(|authors| {
                    if authors.is_empty() {
                        None
                    } else {
                        Some(authors.split(":").collect())
                    }
                })
            }

            #[inline]
            fn homepage(&self) -> Option<&str> {
                option_env!("CARGO_PKG_HOMEPAGE").and_then(|homepage| {
                    if homepage.is_empty() {
                        None
                    } else {
                        Some(homepage)
                    }
                })
            }

            #[inline]
            fn license(&self) -> Option<&str> {
                option_env!("CARGO_PKG_LICENSE").and_then(|license| {
                    if license.is_empty() {
                        None
                    } else {
                        Some(license)
                    }
                })
            }

            #[inline]
            fn repository(&self) -> Option<&str> {
                option_env!("CARGO_PKG_REPOSITORY").and_then(|repository| {
                    if repository.is_empty() {
                        None
                    } else {
                        Some(repository)
                    }
                })
            }
        }

    };

    proc_macro::TokenStream::from(expanded)
}
