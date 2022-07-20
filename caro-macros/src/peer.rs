use quote::{quote, ToTokens};
use syn::{self, LitStr};

#[derive(Default)]
pub(crate) struct Features {
    methods: bool,
}

pub(crate) fn parse_fetures(attribute: &syn::Attribute) -> Features {
    Features::default()
}

pub(crate) fn parse_signals(fields: &syn::FieldsNamed) -> Vec<proc_macro2::TokenStream> {
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

pub(crate) fn parse_states(fields: &syn::FieldsNamed) -> Vec<proc_macro2::TokenStream> {
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

pub(crate) fn parse_service_name(attribute: &syn::Attribute) -> LitStr {
    match attribute.path.get_ident() {
        Some(ident) => {
            if ident != "service" {
                panic!("Invalid service attribute. Should be a single `service` attribute with a service name and optional features. #[service(\"com.test.service\")] or #[service(name = \"com.test.service\", features = [])]. Got: '{}'", ident);
            }

            let exp = match syn::parse2::<syn::Expr>(attribute.tokens.clone()) {
                Ok(syn::Expr::Paren(exp)) => *exp.expr,
                Ok(e) => {
                    panic!("Invalid service attribute. Should be a single `service` attribute with a service name and optional features. #[service(\"com.test.service\")] or #[service(name = \"com.test.service\", features = [])]. Got: '{:?}'", e)
                }
                Err(err) => {
                    panic!("Invalid service attribute. Should be a single `service` attribute with a service name and optional features. #[service(\"com.test.service\")] or #[service(name = \"com.test.service\", features = [])]. Got: '{}'. Err: {}", attribute.tokens, err.to_string())
                }
            };

            match exp {
                syn::Expr::Lit(syn::ExprLit{attrs: _, lit: syn::Lit::Str(name)}) => return name,
                e => panic!("Invalid service attribute. Should be a single `service` attribute with a service name and optional features. #[service(\"com.test.service\")] or #[service(name = \"com.test.service\", features = [])]. Got: '{:?}'", e)
            }
        }
        _ => {
            panic!("Invalid service attribute. Should be a single `service` attribute with a service name and optional features. #[service(\"com.test.service\")] or #[service(name = \"com.test.service\", features = [])]");
        }
    }
}

pub(crate) fn parse_impl_ident(impl_type: &syn::Type) -> syn::Ident {
    let path = match impl_type {
        syn::Type::Path(syn::TypePath { qself: _, path }) => path,
        _ => panic!("Failed to parse service methods. Unexpected impl struct"),
    };

    path.get_ident().cloned().unwrap()
}

fn check_method_params(params: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>) {
    if params.len() != 2 {
        panic!("Invalid service method signature. Should be a single-argument method. E.g 'async fn hello(&mut self, param: String) -> i32. Got: {}", params.to_token_stream().to_string());
    }

    if !matches!(params.first().unwrap(), syn::FnArg::Receiver(_)) {
        panic!("Invalid first service method argument. Should be '&self'");
    }
}

pub(crate) fn parse_methods(methods: &Vec<syn::ImplItem>) -> Vec<proc_macro2::TokenStream> {
    let mut result = vec![];

    for item in methods {
        if let syn::ImplItem::Method(method) = item {
            for attr in &method.attrs {
                if attr.path.is_ident("method") {
                    check_method_params(&method.sig.inputs);

                    let method_ident = &method.sig.ident;
                    let method_name =
                        syn::LitStr::new(&method_ident.to_string(), method_ident.span());

                    result.push(quote! {
                        Self::register_method(#method_name, move |p| async move {
                                context.get().#method_ident(p).await
                            })
                            .await?;
                    })
                }
            }
        }
    }

    result
}
