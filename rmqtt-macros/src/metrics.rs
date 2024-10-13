use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, FieldsNamed, Ident};

pub(crate) fn build(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = input.ident;

    let clone_items = get_fields_named(&input.data)
        .named
        .iter()
        .map(|f| {
            let name = &f.ident;
            quote!(
                #name: AtomicUsize::new(self.#name.load(Ordering::SeqCst)),
            )
        })
        .collect::<Vec<_>>();

    let init_items = get_fields_named(&input.data)
        .named
        .iter()
        .map(|f| {
            let name = &f.ident;
            quote!(
                #name: AtomicUsize::new(0),
            )
        })
        .collect::<Vec<_>>();

    let inc_items = get_fields_named(&input.data)
        .named
        .iter()
        .map(|f| {
            let name = &f.ident;
            let fn_name = name.as_ref().map(|ref i| Ident::new(&format!("{}_inc", i), i.span()));
            quote! {
                #[inline]
                pub fn #fn_name(&self) {
                    self.#name.fetch_add(1, Ordering::SeqCst);
                }
            }
        })
        .collect::<Vec<_>>();

    let get_items = get_fields_named(&input.data)
        .named
        .iter()
        .map(|f| {
            let name = &f.ident;
            let fn_name = name.as_ref().map(|ref i| Ident::new(&format!("{}", i), i.span()));
            quote! {
                #[inline]
                pub fn #fn_name(&self) -> usize {
                    self.#name.load(Ordering::SeqCst)
                }
            }
        })
        .collect::<Vec<_>>();

    let json_items = get_fields_named(&input.data)
        .named
        .iter()
        .map(|f| {
            let name = &f.ident;
            let attr_name = f.ident.as_ref().map(|i| i.to_string().replace('_', "."));
            quote!(
                #attr_name : self.#name.load(Ordering::SeqCst),
            )
        })
        .collect::<Vec<_>>();

    let add_items = get_fields_named(&input.data)
        .named
        .iter()
        .map(|f| {
            let name = &f.ident;
            quote!(
                self.#name.fetch_add(other.#name.load(Ordering::SeqCst), Ordering::SeqCst);
            )
        })
        .collect::<Vec<_>>();

    let prometheus_items = get_fields_named(&input.data)
        .named
        .iter()
        .map(|f| {
            let name = &f.ident;
            let attr_name = f.ident.as_ref().map(|i| i.to_string().replace('_', "."));
            quote!(
                metrics_gauge_vec
                    .with_label_values(&[&label, #attr_name])
                    .set(self.#name.load(Ordering::SeqCst) as i64);
            )
        })
        .collect::<Vec<_>>();

    let expanded = quote! {
        impl Clone for #name{
            fn clone(&self) -> Self{
                Self{
                    #(#clone_items)*
                }
            }
        }

        impl #name {
            #[inline]
            pub fn instance() -> &'static #name {
                static INSTANCE: once_cell::sync::OnceCell<#name> = once_cell::sync::OnceCell::new();
                INSTANCE.get_or_init(|| Self {
                    #(#init_items)*
                })
            }

            #(#inc_items)*

            #(#get_items)*

            #[inline]
            pub fn to_json(&self) -> serde_json::Value {
                serde_json::json!({
                    #(#json_items)*
                })
            }

            #[inline]
            pub fn add(&mut self, other: &Self) {
                #(#add_items)*
            }

            #[inline]
            pub fn build_prometheus_metrics(&self, label: &str, metrics_gauge_vec: &prometheus::IntGaugeVec) {
                #(#prometheus_items)*
            }

        }
    };
    proc_macro::TokenStream::from(expanded)
}

fn get_fields_named(data: &Data) -> &FieldsNamed {
    match *data {
        Data::Struct(ref data) => match data.fields {
            Fields::Named(ref fields) => fields,
            Fields::Unnamed(ref _fields) => {
                unreachable!()
            }
            Fields::Unit => {
                unreachable!()
            }
        },
        Data::Enum(_) | Data::Union(_) => unreachable!(),
    }
}
