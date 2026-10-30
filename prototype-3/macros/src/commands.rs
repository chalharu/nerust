use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    FnArg, ImplItem, ItemImpl, LitStr, ReturnType, Type, spanned::Spanned, visit_mut::VisitMut,
};

// Only rewrite the generated argument copy: Self in the source impl still
// refers to the model, whereas Self in the extension impl refers to Store<Model>.
fn argument_type(source: &Type, model: &syn::TypePath) -> syn::Result<Type> {
    struct ModelSelf<'a> {
        model: &'a syn::TypePath,
        error: Option<syn::Error>,
    }
    impl ModelSelf<'_> {
        fn replace_path(&self, qself: &mut Option<syn::QSelf>, path: &mut syn::Path) {
            if qself.is_some() || !path.segments.first().is_some_and(|s| s.ident == "Self") {
                return;
            }
            let suffix: syn::punctuated::Punctuated<_, syn::Token![::]> =
                path.segments.iter().skip(1).cloned().collect();
            let model = self.model;
            let replacement: syn::TypePath = if suffix.is_empty() {
                model.clone()
            } else {
                syn::parse_quote!(<#model>::#suffix)
            };
            *qself = replacement.qself;
            *path = replacement.path;
        }
    }
    impl VisitMut for ModelSelf<'_> {
        fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
            syn::visit_mut::visit_type_path_mut(self, ty);
            self.replace_path(&mut ty.qself, &mut ty.path);
        }
        fn visit_expr_path_mut(&mut self, expr: &mut syn::ExprPath) {
            syn::visit_mut::visit_expr_path_mut(self, expr);
            self.replace_path(&mut expr.qself, &mut expr.path);
        }
        fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
            fn contains_self(tokens: TokenStream) -> bool {
                tokens.into_iter().any(|token| match token {
                    proc_macro2::TokenTree::Ident(ident) => ident == "Self",
                    proc_macro2::TokenTree::Group(group) => contains_self(group.stream()),
                    _ => false,
                })
            }
            if contains_self(mac.tokens.clone()) {
                self.error = Some(syn::Error::new(
                    mac.span(),
                    "Self in command argument macros is unsupported; use the concrete model type",
                ));
            }
        }
    }
    let mut ty = source.clone();
    let mut visitor = ModelSelf { model, error: None };
    visitor.visit_type_mut(&mut ty);
    match visitor.error {
        Some(error) => Err(error),
        None => Ok(ty),
    }
}

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
            Some(FnArg::Typed(argument)) => argument_type(&argument.ty, &model)?,
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
    fn argument_self_is_replaced_recursively_without_changing_named_types() {
        let model = syn::parse_quote!(crate::models::Settings);
        for (source, expected) in [
            ("Self", "crate::models::Settings"),
            ("Option<Vec<Self>>", "Option<Vec<crate::models::Settings>>"),
            (
                "(&'static Self, [Self; 2], *const Self)",
                "(&'static crate::models::Settings, [crate::models::Settings; 2], *const crate::models::Settings)",
            ),
            (
                "fn(Self) -> Self",
                "fn(crate::models::Settings) -> crate::models::Settings",
            ),
            (
                "<Self as Payload>::Value",
                "<crate::models::Settings as Payload>::Value",
            ),
            ("Self::Value", "<crate::models::Settings>::Value"),
            (
                "<Vec<Self> as Payload<Self>>::Value",
                "<Vec<crate::models::Settings> as Payload<crate::models::Settings>>::Value",
            ),
            ("[u8; Self::SIZE]", "[u8; <crate::models::Settings>::SIZE]"),
            ("Option<Concrete>", "Option<Concrete>"),
            ("Concrete::Value", "Concrete::Value"),
        ] {
            let source: Type = syn::parse_str(source).unwrap();
            let original = source.clone();
            let expected: Type = syn::parse_str(expected).unwrap();
            assert_eq!(argument_type(&source, &model).unwrap(), expected);
            assert_eq!(source, original);
        }
    }

    #[test]
    fn generated_signatures_change_but_the_original_impl_does_not() {
        let source: ItemImpl = syn::parse_quote!(impl crate::models::Settings {
            #[command] fn replace(&mut self, other: Option<Vec<Self>>) {}
            #[command] fn associated(model: Store<Self>, other: <Self as Payload>::Value) {}
        });
        let generated: syn::File = syn::parse2(expand(source.clone()).unwrap()).unwrap();
        let mut original = source;
        for member in &mut original.items {
            if let ImplItem::Fn(method) = member {
                method.attrs.clear();
            }
        }
        assert_eq!(generated.items[0], syn::Item::Impl(original));
        for item in &generated.items[1..] {
            let tokens = quote!(#item).to_string();
            assert!(!tokens.contains("Self"), "{tokens}");
            assert!(tokens.contains("crate :: models :: Settings"), "{tokens}");
        }
    }

    #[test]
    fn opaque_argument_macros_with_self_have_an_explicit_diagnostic() {
        let model = syn::parse_quote!(Settings);
        let error = argument_type(&syn::parse_quote!(argument!(Option<Self>)), &model).unwrap_err();
        assert!(error.to_string().contains("use the concrete model type"));
        let concrete = syn::parse_quote!(argument!(Concrete));
        assert_eq!(argument_type(&concrete, &model).unwrap(), concrete);
    }

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
