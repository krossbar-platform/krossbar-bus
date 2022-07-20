mod service;

use proc_macro::TokenStream;
use quote::quote;
use syn::{self, DeriveInput};

#[proc_macro_derive(Service, attributes(service_name, signal, state, peer))]
pub fn service(input: TokenStream) -> TokenStream {
    let service_struct: DeriveInput = syn::parse(input).unwrap();

    let struct_fields = match service_struct.data {
        syn::Data::Struct(structure) => match structure.fields {
            syn::Fields::Named(named) => named,
            _ => unreachable!("Can't have unnamed fields in a struct"),
        },
        syn::Data::Enum(..) => panic!("Invalid 'Service' usage. Expected struct, got enum"),
        syn::Data::Union(..) => panic!("Invalid 'Service' usage. Expected struct, got union"),
    };

    if service_struct.attrs.len() != 1 {
        panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.g. #[service_name(\"com.test.service\")]");
    }

    let service_name = service::parse_service_name(&service_struct.attrs[0]);
    let struct_ident = &service_struct.ident;

    let signals = service::parse_signals(&struct_fields);
    let states = service::parse_states(&struct_fields);

    quote! {
        #[async_trait]
        impl caro_service::Service for #struct_ident {
            async fn register_service(&mut self) -> caro_bus_lib::Result<()> {
                Self::register_bus(#service_name).await?;

                //self.peer.register().await?;
                //self.peer.register_callbacks().await?;

                #(#signals);*
                #(#states);*
                Ok(())
            }
        }
    }
    .into()
}

#[proc_macro_attribute]
pub fn method(_attr: TokenStream, input: TokenStream) -> TokenStream {
    input
}

#[proc_macro_attribute]
pub fn service_impl(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let imp: syn::ItemImpl = syn::parse(input.clone()).unwrap();

    let self_name = service::parse_impl_ident(&imp.self_ty);
    let methods = service::parse_methods(&imp.items);

    quote! {
        #imp

        #[async_trait]
        impl caro_service::service::ServiceMethods for Pin<Box<#self_name>> {
            async fn register_methods(&mut self) -> caro_bus_lib::Result<()> {
                let context = caro_service::this::This { pointer: self };

                #(#methods);*
                Ok(())
            }
        }
    }
    .into()
}
