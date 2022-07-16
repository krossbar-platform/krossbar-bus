use quote::quote;
use syn::{self, LitStr};

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
