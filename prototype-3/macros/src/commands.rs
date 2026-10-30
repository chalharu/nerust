use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ItemImpl, LitStr, ReturnType, Type, spanned::Spanned};

pub fn expand(mut item: ItemImpl) -> syn::Result<TokenStream> {
    if item.trait_.is_some()
        || !item.generics.params.is_empty()
        || item.generics.where_clause.is_some()
    {
        return Err(syn::Error::new(
            item.span(),
            "commands requires an inherent, non-generic impl",
        ));
    }
    let Type::Path(model) = item.self_ty.as_ref() else {
        return Err(syn::Error::new(
            item.self_ty.span(),
            "expected model type path",
        ));
    };
    let model = model.clone();
    if model.qself.is_some()
        || model
            .path
            .segments
            .iter()
            .any(|s| !matches!(s.arguments, syn::PathArguments::None))
    {
        return Err(syn::Error::new(
            model.span(),
            "generic model types are unsupported",
        ));
    }
    let trait_name = format_ident!("{}Commands", model.path.segments.last().unwrap().ident);
    let mut declarations = Vec::new();
    let mut methods = Vec::new();
    for member in &mut item.items {
        let ImplItem::Fn(method) = member else {
            continue;
        };
        let attrs: Vec<_> = method
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("command"))
            .cloned()
            .collect();
        if attrs.is_empty() {
            continue;
        }
        if attrs.len() != 1 {
            return Err(syn::Error::new(
                method.span(),
                "duplicate command attribute",
            ));
        }
        method.attrs.retain(|a| !a.path().is_ident("command"));
        let attr = &attrs[0];
        let mut helper: Option<syn::Ident> = None;
        if !matches!(attr.meta, syn::Meta::Path(_)) {
            attr.parse_nested_meta(|meta| {
                if !meta.path.is_ident("can_execute") || helper.is_some() {
                    return Err(meta.error("expected one can_execute=\"helper\""));
                }
                let value: LitStr = meta.value()?.parse()?;
                helper =
                    Some(syn::parse_str(&value.value()).map_err(|_| {
                        syn::Error::new(value.span(), "expected helper identifier")
                    })?);
                Ok(())
            })?;
        }
        let sig = &method.sig;
        if sig.asyncness.is_some()
            || sig.unsafety.is_some()
            || sig.constness.is_some()
            || sig.abi.is_some()
            || sig.variadic.is_some()
            || !sig.generics.params.is_empty()
            || sig.generics.where_clause.is_some()
            || !matches!(sig.output, ReturnType::Default)
                && !matches!(&sig.output,ReturnType::Type(_,t) if matches!(t.as_ref(),Type::Tuple(v) if v.elems.is_empty()))
        {
            return Err(syn::Error::new(
                sig.span(),
                "command must be a synchronous, safe, non-generic unit-returning method",
            ));
        }
        let name = &sig.ident;
        let generated = format_ident!("command_{}", name.to_string().trim_start_matches("r#"));
        let args: Vec<_> = sig.inputs.iter().collect();
        let receiver = matches!(args.first(), Some(FnArg::Receiver(_)));
        let argument_type = match args.get(1) {
            Some(FnArg::Typed(argument)) => (*argument.ty).clone(),
            _ => syn::parse_quote!(::std::string::String),
        };
        let action = if receiver {
            let FnArg::Receiver(r) = args[0] else {
                unreachable!()
            };
            if r.reference.is_none()
                || r.mutability.is_none()
                || r.colon_token.is_some()
                || args.len() > 2
            {
                return Err(syn::Error::new(
                    sig.span(),
                    "expected &mut self with zero arguments or one typed argument",
                ));
            }
            let call = if args.len() == 2 {
                quote!(__state.#name(__argument))
            } else {
                quote!(__state.#name())
            };
            quote!({ let _ = __store.update(move |__state| { #call; }); })
        } else {
            if args.len() != 2 {
                return Err(syn::Error::new(
                    sig.span(),
                    "static command requires (model: Store<Self>, argument: A)",
                ));
            }
            let FnArg::Typed(first) = args[0] else {
                unreachable!()
            };
            let valid = if let Type::Path(p) = first.ty.as_ref() {
                p.path.segments.last().is_some_and(|s| s.ident == "Store" && match &s.arguments {
                    syn::PathArguments::AngleBracketed(a) if a.args.len() == 1 => matches!(a.args.first(),Some(syn::GenericArgument::Type(Type::Path(t))) if t.path.is_ident("Self") || t == &model),
                    _ => false,
                })
            } else {
                false
            };
            if !valid {
                return Err(syn::Error::new(first.span(), "expected Store<Self>"));
            }
            quote!(<#model>::#name(__store.clone(),__argument))
        };
        let enabled = helper.map_or_else(
            || quote!(|_| true),
            |h| quote!(|__state| <#model>::#h(__state)),
        );
        // Preserve conditional compilation: never emit methods for a disabled source command.
        let cfg: Vec<_> = method
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("cfg"))
            .collect();
        declarations.push(quote!(#(#cfg)* fn #generated(&self, argument: ::wakui2_runtime::Binding<#argument_type>) -> ::wakui2_runtime::Command;));
        methods.push(quote!(#(#cfg)* fn #generated(&self, argument: ::wakui2_runtime::Binding<#argument_type>) -> ::wakui2_runtime::Command {
            self.command(#enabled, |__store,__argument| { #action; }, argument)
        }));
    }
    Ok(quote!(
        #item
        pub trait #trait_name { #(#declarations)* }
        impl #trait_name for ::wakui2_runtime::Store<#model> { #(#methods)* }
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_are_real_methods() {
        let t = expand(syn::parse_quote!(impl Settings {
            #[command] fn reset(&mut self) {}
            #[command] fn add(&mut self, value: String) {}
            #[command(can_execute="can_save")] fn save(model: Store<Self>, value: String) {}
            fn can_save(&self) -> bool { true }
        }))
        .unwrap()
        .to_string();
        assert!(t.contains("SettingsCommands"));
        assert!(t.contains("command_reset"));
        assert!(t.contains("command_add"));
        assert!(t.contains("command_save"));
        assert!(t.contains("update"));
        assert!(!t.contains("# [command"));
        syn::parse2::<syn::File>(t.parse().unwrap()).unwrap();
    }
    #[test]
    fn invalid_commands_are_rejected() {
        for source in [
            "impl S { #[command] fn x(&self) {} }",
            "impl S { #[command] fn x(&mut self, v:u32, extra:u32) {} }",
            "impl S { #[command] async fn x(&mut self) {} }",
            "impl S { #[command] fn x(&mut self)->bool {true} }",
            "impl S { #[command] fn x(s: Self, v:String) {} }",
            "impl<T> S<T> { #[command] fn x(&mut self) {} }",
            "impl S { #[command(foo=\"x\")] fn x(&mut self) {} }",
            "impl S { #[command] #[command] fn x(&mut self) {} }",
        ] {
            assert!(expand(syn::parse_str(source).unwrap()).is_err(), "{source}");
        }
    }
}
