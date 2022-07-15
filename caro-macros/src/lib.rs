use proc_macro::TokenStream;
use quote::quote;
use syn::{self, DeriveInput, LitStr};

fn parse_signals(fields: &syn::FieldsNamed) -> Vec<proc_macro2::TokenStream> {
    let mut result = vec![];

    for field in fields.named.iter() {
        for attribute in field.attrs.iter() {
            if attribute.path.is_ident("signal") {
                let signal_field = field.ident.as_ref().unwrap();
                let signal_name = syn::LitStr::new(&signal_field.to_string(), signal_field.span());

                result.push(quote! {
                    self.#signal_field.register(#signal_name).await?;
                })
            }
        }
    }

    result
}

fn parse_states(fields: &syn::FieldsNamed) -> Vec<proc_macro2::TokenStream> {
    let mut result = vec![];

    for field in fields.named.iter() {
        for attribute in field.attrs.iter() {
            if attribute.path.is_ident("state") {
                let state_field = field.ident.as_ref().unwrap();
                let state_name = syn::LitStr::new(&state_field.to_string(), state_field.span());

                let state_initial_value = match syn::parse2::<syn::ExprParen>(
                    attribute.tokens.clone(),
                ) {
                    Ok(exp) => exp.expr,
                    Err(err) => {
                        panic!("Invalid service state attribute. Should be a parenthesized expression. E.g. #[state(42)]. Got: '{}'. Err: {}", attribute.tokens, err.to_string())
                    }
                };

                result.push(quote! {
                    self.#state_field.register(#state_name, #state_initial_value).await?;
                })
            }
        }
    }

    result
}

fn parse_service_name(attribute: &syn::Attribute) -> LitStr {
    match attribute.path.get_ident() {
        Some(ident) => {
            if ident != "service_name" {
                panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.g. #[service_name(\"com.test.service\")]. Got: '{}'", ident);
            }

            let exp = match syn::parse2::<syn::ExprParen>(attribute.tokens.clone()) {
                Ok(exp) => exp,
                Err(err) => {
                    panic!("Invalid service name attribute. Should be a parenthesized single `service_name` attribute with a service name. E.g. #[service_name(\"com.test.service\")]. Got: '{}'. Err: {}", attribute.tokens, err.to_string())
                }
            };

            match *exp.expr {
                syn::Expr::Lit(syn::ExprLit{attrs: _, lit: syn::Lit::Str(name)}) => return name,
                _ => panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.g. #[service_name(\"com.test.service\")]. Got: '{}'", attribute.tokens)
            }
        }
        _ => {
            panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.g. #[service_name(\"com.test.service\")]");
        }
    }
}

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

    let service_name = parse_service_name(&service_struct.attrs[0]);
    let struct_ident = &service_struct.ident;

    let signals = parse_signals(&struct_fields);
    let states = parse_states(&struct_fields);

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
