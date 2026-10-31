//! Direct, typed schema-v2 expansion; no build script, registry or OUT_DIR.
mod commands;
mod generate;

use proc_macro::TokenStream;
use quote::quote;
use std::io::Read;
use syn::{
    Expr, LitStr, Path, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

struct ViewInput {
    file: LitStr,
    model: Expr,
    backend: Path,
}
impl Parse for ViewInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let file = input.parse()?;
        input.parse::<Token![,]>()?;
        let model = input.parse()?;
        input.parse::<Token![,]>()?;
        let backend = input.parse()?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        Ok(Self {
            file,
            model,
            backend,
        })
    }
}

/// Construct a shared-model View; closing it releases ownership without closing the Store.
#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ViewInput);
    expand_view(input, false)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}
/// Consume a non-Clone StoreOwner; the View closes it on closure or drop.
#[proc_macro]
pub fn owned_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ViewInput);
    expand_view(input, true)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}
#[proc_macro]
pub fn try_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ViewInput);
    expand_try_view(input, false)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}
#[proc_macro]
pub fn try_owned_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ViewInput);
    expand_try_view(input, true)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}
fn expand_try_view(input: ViewInput, owned: bool) -> syn::Result<proc_macro2::TokenStream> {
    let body = expand_view(input, owned)?;
    Ok(quote!({
        let __constructed = { #body };
        match __constructed.error() {
            ::std::option::Option::Some(__error) => {
                __constructed.close();
                ::std::result::Result::Err(__error)
            }
            ::std::option::Option::None => ::std::result::Result::Ok(__constructed),
        }
    }))
}
fn expand_view(input: ViewInput, owned: bool) -> syn::Result<proc_macro2::TokenStream> {
    let fail = |message: String| syn::Error::new(input.file.span(), message);
    let base = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| fail("CARGO_MANIFEST_DIR missing".into()))?;
    let path = std::path::PathBuf::from(base)
        .join(input.file.value())
        .canonicalize()
        .map_err(|e| fail(format!("{}: {e}", input.file.value())))?;
    let absolute = path
        .to_str()
        .ok_or_else(|| fail("UI path must be UTF-8".into()))?;
    let mut source = String::new();
    std::fs::File::open(&path)
        .and_then(|f| {
            f.take((wakui2_markup::MAX_BYTES + 1) as u64)
                .read_to_string(&mut source)
        })
        .map_err(|e| fail(format!("{absolute}: {e}")))?;
    let format = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let document = wakui2_markup::parse(&source, format, absolute).map_err(|e| {
        let marker = e
            .span
            .map(|span| generate::source_marker(absolute, span))
            .unwrap_or_default();
        fail(format!("{e}\n{marker}"))
    })?;
    let body = generate::generate(&document, &input.backend, owned)
        .map_err(|e| fail(format!("{absolute}: {e}")))?;
    let model = input.model;
    let root = if owned {
        quote!(
            let __wakui_owner: ::wakui2_runtime::StoreOwner<_> = #model;
            let __wakui_root = __wakui_owner.store().clone();
        )
    } else {
        quote!(let __wakui_root = (#model).clone();)
    };
    Ok(quote!({
        const _: &str = include_str!(#absolute);
        #root
        let __wakui_model = __wakui_root.clone();
        #body
    }))
}

/// Generate `<Model>Commands` for an inherent, non-generic model impl.
#[proc_macro_attribute]
pub fn commands(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "commands takes no arguments",
        )
        .to_compile_error()
        .into();
    }
    let item = parse_macro_input!(input as syn::ItemImpl);
    commands::expand(item)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_expansion_tracks_absolute_fixture() {
        let input = syn::parse_quote!("tests/ui/main.yaml", model, fake);
        let code = expand_view(input, false).unwrap();
        let text = code.to_string();
        assert!(text.contains("include_str !"));
        let absolute = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/ui/main.yaml")
            .canonicalize()
            .unwrap();
        assert!(text.contains(absolute.to_str().unwrap()));
        assert!(!text.contains("OUT_DIR"));
        assert!(text.contains("on_close"));
        syn::parse2::<syn::Expr>(code).unwrap();
    }
    #[test]
    fn invocation_syntax_is_strict() {
        for t in [
            quote!("ui.yaml", model),
            quote!("ui.yaml", model, fake, extra),
            quote!(42, model, fake),
        ] {
            assert!(syn::parse2::<ViewInput>(t).is_err());
        }
    }
}
