use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{self, parse::Parser, Field, Fields, ItemStruct};

#[proc_macro_attribute]
pub fn service(metadata: TokenStream, input: TokenStream) -> TokenStream {
    let metadata: syn::LitStr = syn::parse(metadata).unwrap();

    let mut service_struct: ItemStruct = syn::parse(input).unwrap();
    match &mut service_struct.fields {
        Fields::Named(fields) => fields
            .named
            .push(Field::parse_named.parse2(quote! { _name: String }).unwrap()),
        _f => {
            return quote!(compile_error!(format!(
                "Expected structure to have names fields, got {:?}",
                _f
            ),))
            .into();
        }
    }

    let struct_ident = &service_struct.ident;
    let implementation_stream = quote! {
        impl caro_bus_lib::Service for #struct_ident {
            fn init(&mut self) {
                self._name = #metadata.into()
            }

            fn print(&self) {
                println!("Service name: {}", self._name)
            }
        }
    };
    let service_stream = service_struct.into_token_stream();

    quote! {
        #service_stream

        #implementation_stream
    }
    .into()
}
