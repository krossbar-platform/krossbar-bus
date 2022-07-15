use proc_macro::TokenStream;
use quote::quote;
use syn::{self, DeriveInput, LitStr};

fn parse_service_name(attribute: &syn::Attribute) -> LitStr {
    match attribute.path.get_ident() {
        Some(ident) => {
            if ident != "service_name" {
                panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.q. #[service_name(\"com.test.service\")]. Got: {}", ident);
            }

            let exp = match syn::parse2::<syn::ExprParen>(attribute.tokens.clone()) {
                Ok(exp) => exp,
                Err(err) => {
                    panic!("Invalid service name attribute. Should be a parenthesized single `service_name` attribute with a service name. E.q. #[service_name(\"com.test.service\")]. Got: {}. {}", attribute.tokens, err.to_string())
                }
            };

            match *exp.expr {
                syn::Expr::Lit(syn::ExprLit{attrs: _, lit: syn::Lit::Str(name)}) => return name,
                _ => panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.q. #[service_name(\"com.test.service\")]. Got: {}", attribute.tokens)
            }
        }
        _ => {
            panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.q. #[service_name(\"com.test.service\")]");
        }
    }
}

#[proc_macro_derive(Service, attributes(service_name, signal, state, peer))]
pub fn service(input: TokenStream) -> TokenStream {
    let service_struct: DeriveInput = syn::parse(input).unwrap();

    if service_struct.attrs.len() != 1 {
        panic!("Invalid service name attribute. Should be a single `service_name` attribute with a service name. E.q. #[service_name(\"com.test.service\")]");
    }

    let service_name = parse_service_name(&service_struct.attrs[0]);
    let struct_ident = &service_struct.ident;

    quote! {
        #[async_trait]
        impl caro_service::Service for #struct_ident {
            async fn register_service(&mut self) -> caro_bus_lib::Result<()> {
                Self::register_bus(#service_name).await?;

                //self.peer.register().await?;
                //self.peer.register_callbacks().await?;

                //self.signal.register("signal").await?;
                //self.state.register("state", 0).await?;
                Ok(())
            }
        }
    }
    .into()
}
