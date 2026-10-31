//! Direct, typed schema-v2 expansion; no build script, registry or OUT_DIR.
mod commands;
mod generate;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::quote;
use std::io::Read;
use syn::{
    Expr, LitStr, Path, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
    visit_mut::VisitMut,
};

struct RuntimePaths(Path);
impl RuntimePaths {
    fn resolve() -> syn::Result<Self> {
        let found = crate_name("panechord-runtime")
            .or_else(|_| crate_name("panechord"))
            .map_err(|error| syn::Error::new(proc_macro2::Span::call_site(), error))?;
        let path = match found {
            FoundCrate::Itself => syn::parse_quote!(crate),
            FoundCrate::Name(name) => {
                let name = syn::Ident::new(&name, proc_macro2::Span::call_site());
                syn::parse_quote!(::#name)
            }
        };
        Ok(Self(path))
    }
    fn macro_tokens(&self, stream: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        use proc_macro2::{Group, TokenTree};
        let tokens = stream.into_iter().collect::<Vec<_>>();
        let mut output = proc_macro2::TokenStream::new();
        let mut index = 0;
        while index < tokens.len() {
            if matches!(tokens.get(index), Some(TokenTree::Punct(mark)) if mark.as_char() == ':')
                && matches!(tokens.get(index + 1), Some(TokenTree::Punct(mark)) if mark.as_char() == ':')
                && matches!(tokens.get(index + 2), Some(TokenTree::Ident(name)) if name == "panechord_runtime")
                && (index == 0
                    || (!matches!(tokens.get(index - 1), Some(TokenTree::Ident(_)))
                        && !(matches!(tokens.get(index - 1), Some(TokenTree::Punct(mark)) if mark.as_char() == '>')
                            && !matches!(tokens.get(index.wrapping_sub(2)), Some(TokenTree::Punct(mark)) if mark.as_char() == '-'))))
            {
                let path = &self.0;
                output.extend(quote!(#path));
                index += 3;
                continue;
            }
            match &tokens[index] {
                TokenTree::Group(group) => {
                    let mut replacement =
                        Group::new(group.delimiter(), self.macro_tokens(group.stream()));
                    replacement.set_span(group.span());
                    output.extend([TokenTree::Group(replacement)]);
                }
                token => output.extend([token.clone()]),
            }
            index += 1;
        }
        output
    }
}
impl VisitMut for RuntimePaths {
    fn visit_macro_mut(&mut self, node: &mut syn::Macro) {
        self.visit_path_mut(&mut node.path);
        node.tokens = self.macro_tokens(node.tokens.clone());
    }
    fn visit_path_mut(&mut self, path: &mut Path) {
        if path.leading_colon.is_some()
            && path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == "panechord_runtime")
        {
            let suffix = path.segments.iter().skip(1).cloned().collect::<Vec<_>>();
            *path = self.0.clone();
            path.segments.extend(suffix);
        }
        syn::visit_mut::visit_path_mut(self, path);
    }
}

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
            f.take((panechord_markup::MAX_BYTES + 1) as u64)
                .read_to_string(&mut source)
        })
        .map_err(|e| fail(format!("{absolute}: {e}")))?;
    let format = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let document = panechord_markup::parse(&source, format, absolute).map_err(|e| {
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
            let __panechord_owner: ::panechord_runtime::StoreOwner<_> = #model;
            let __panechord_root = __panechord_owner.store().clone();
        )
    } else {
        quote!(let __panechord_root = (#model).clone();)
    };
    let mut expression: syn::Expr = syn::parse2(quote!({
        const _: &str = include_str!(#absolute);
        #root
        let __panechord_model = __panechord_root.clone();
        #body
    }))?;
    RuntimePaths::resolve()?.visit_expr_mut(&mut expression);
    Ok(quote!(#expression))
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
        .and_then(|code| {
            let mut file: syn::File = syn::parse2(code)?;
            RuntimePaths::resolve()?.visit_file_mut(&mut file);
            Ok(quote!(#file))
        })
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_alias_rewriting_preserves_local_paths_and_literals() {
        let mut expression: syn::Expr = syn::parse_quote!({
            panechord_runtime::local();
            crate::panechord_runtime::local();
            ::panechord_runtime::external();
            macro_rules! body {
                () => {
                    crate::panechord_runtime::local();
                    Local::<u8>::panechord_runtime();
                    let value: ::panechord_runtime::Binding<u8>;
                    fn command() -> ::panechord_runtime::Command {
                        unimplemented!()
                    }
                    ::panechord_runtime::external();
                    "::panechord_runtime";
                };
            }
        });
        RuntimePaths(syn::parse_quote!(::surface)).visit_expr_mut(&mut expression);
        let expected = quote!({
            panechord_runtime::local();
            crate::panechord_runtime::local();
            ::surface::external();
            macro_rules! body {
                () => {
                    crate::panechord_runtime::local();
                    Local::<u8>::panechord_runtime();
                    let value: ::surface::Binding<u8>;
                    fn command() -> ::surface::Command {
                        unimplemented!()
                    }
                    ::surface::external();
                    "::panechord_runtime";
                };
            }
        });
        assert_eq!(quote!(#expression).to_string(), expected.to_string());
    }
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
