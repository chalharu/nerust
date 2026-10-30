use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Path};
use wakui2_markup::{Control, Document, Edges, Mode, Orientation, Property, Style};

pub fn generate(doc: &Document, backend: &Path, owned: bool) -> syn::Result<TokenStream> {
    doc.validate()
        .map_err(|(p, e)| error(format!("{p}: {e}")))?;
    let Control::Window {
        title,
        content,
        menus,
        ..
    } = &doc.root
    else {
        unreachable!()
    };
    let title = binding(title, false)?;
    let body = control(doc, content, backend, false)?;
    let style = style(&doc.root.style());
    let mut menu_code = Vec::new();
    for menu in menus {
        let id = &menu.id;
        let title = binding(&menu.title, false)?;
        let items = binding(&menu.items, false)?;
        let command = command(&menu.command)?;
        menu_code.push(quote!({
            let __dispatch_model = __wakui_model.clone();
            __wakui_view.menu(#id,#title,#items,::std::rc::Rc::new(move |__arg| {
                __dispatch_model.#command(::wakui2_runtime::Binding::constant(__arg)).invoke()
            }));
        }));
    }
    let close = if owned {
        quote!(__close_model.close();)
    } else {
        quote!(let _ = &__close_model;)
    };
    Ok(quote!(
        let __wakui_view = #backend::View::new(#title);
        let __wakui_parent = __wakui_view.root();
        __wakui_view.style("__root",#style);
        #body
        #(#menu_code)*
        let __close_model = __wakui_root.clone();
        __wakui_view.on_close(::std::rc::Rc::new(move || { #close }));
        __wakui_view
    ))
}

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}
fn path(value: &str) -> syn::Result<Path> {
    let p: Path =
        syn::parse_str(value).map_err(|_| error(format!("invalid Rust path `{value}`")))?;
    if p.segments
        .iter()
        .any(|s| !matches!(s.arguments, syn::PathArguments::None))
    {
        return Err(error("generic path arguments are not supported in markup"));
    }
    Ok(p)
}
fn command(value: &str) -> syn::Result<Ident> {
    let name = value.strip_prefix("root.").unwrap_or(value);
    let ident: Ident =
        syn::parse_str(name).map_err(|_| error(format!("invalid command name `{value}`")))?;
    Ok(format_ident!(
        "command_{}",
        ident.to_string().trim_start_matches("r#")
    ))
}
fn command_model(value: &str) -> TokenStream {
    if value.starts_with("root.") {
        quote!(__wakui_root)
    } else {
        quote!(__wakui_model)
    }
}
fn binding(prop: &Property, writable: bool) -> syn::Result<TokenStream> {
    Ok(match prop {
        Property::Literal(v) => {
            let v = literal(&v.literal)?;
            quote!(::wakui2_runtime::Binding::constant(#v))
        }
        Property::Bind(v) => {
            let b = &v.bind;
            let (model, fields) = if let Some(s) = b.path.strip_prefix("root.") {
                (quote!(__wakui_root), s)
            } else {
                (quote!(__wakui_model), b.path.as_str())
            };
            let fields: Vec<Ident> = fields
                .split('.')
                .map(|s| {
                    syn::parse_str::<Ident>(s)
                        .map_err(|_| error(format!("invalid binding field path `{}`", b.path)))
                })
                .collect::<Result<_, _>>()?;
            let writer = if writable && b.mode == Mode::TwoWay {
                quote!(::std::option::Option::Some(|s,v| s.#(#fields).* = v))
            } else {
                quote!(::std::option::Option::None)
            };
            let base = quote!(#model.bind(|s| s.#(#fields).*.clone(),#writer));
            if let Some(converter) = &b.converter {
                let converter = path(converter)?;
                quote!((#base).convert(#converter::forward,#converter::backward))
            } else {
                base
            }
        }
    })
}
fn field_writer(value: &str) -> syn::Result<TokenStream> {
    binding(
        &Property::Bind(wakui2_markup::Bound {
            bind: wakui2_markup::Binding {
                path: value.into(),
                mode: Mode::TwoWay,
                converter: None,
            },
        }),
        true,
    )
}
fn literal(value: &serde_json::Value) -> syn::Result<TokenStream> {
    use serde_json::Value;
    Ok(match value {
        Value::String(s) => quote!(::std::string::String::from(#s)),
        Value::Bool(v) => quote!(#v),
        Value::Number(v) => {
            // Numeric primitive properties in the frozen View API are f64.
            let n = v
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| error("invalid numeric literal"))?;
            quote!(#n)
        }
        Value::Object(v)
            if v.keys()
                .all(|k| ["start", "count", "row_height"].contains(&k.as_str())) =>
        {
            let integer = |key: &str, default: usize| -> syn::Result<usize> {
                v.get(key).map_or(Ok(default), |n| {
                    n.as_u64().and_then(|n| n.try_into().ok()).ok_or_else(|| {
                        error(format!("viewport {key} must be a nonnegative integer"))
                    })
                })
            };
            let start = integer("start", 0)?;
            let count = integer("count", 20)?;
            let height = v
                .get("row_height")
                .map_or(Some(48.0), |n| n.as_f64())
                .ok_or_else(|| error("invalid row_height"))? as f32;
            wakui2_runtime::Viewport {
                start,
                count,
                row_height: height,
            }
            .validate()
            .map_err(error)?;
            quote!(::wakui2_runtime::Viewport { start:#start,count:#count,row_height:#height })
        }
        _ => {
            return Err(error(
                "literal must be string, boolean, number or Viewport object; bind collection fields instead",
            ));
        }
    })
}
#[test]
fn viewport_literals_follow_runtime_validation() {
    for (count, row_height) in [(0, 32.0), (257, 32.0), (1, 0.5), (1, 4097.0)] {
        let value = serde_json::json!({"count": count, "row_height": row_height});
        assert!(
            literal(&value)
                .unwrap_err()
                .to_string()
                .contains("invalid viewport")
        );
    }
    for (count, row_height) in [(1, 1.0), (256, 4096.0)] {
        assert!(literal(&serde_json::json!({"count": count, "row_height": row_height})).is_ok());
    }
    assert!(literal(&serde_json::json!({})).is_ok());
    assert!(literal(&serde_json::json!({"row_height": 1e100})).is_err());
}

fn edges(e: &Edges) -> TokenStream {
    let (top, right, bottom, left) = match e {
        Edges::Scalar(v) => (*v, *v, *v, *v),
        Edges::Sides(s) => (s.top, s.right, s.bottom, s.left),
    };
    quote!(::wakui2_runtime::Edges {top:#top,right:#right,bottom:#bottom,left:#left})
}
fn style(s: &Style) -> TokenStream {
    let margin = edges(&s.margin);
    let padding = edges(&s.padding);
    let p = &s.placement;
    let (row, column, row_span, column_span, x, y) =
        (p.row, p.column, p.row_span, p.column_span, p.x, p.y);
    let option = |v: Option<f32>| {
        v.map_or_else(
            || quote!(::std::option::Option::None),
            |v| quote!(::std::option::Option::Some(#v)),
        )
    };
    let width = option(p.width);
    let height = option(p.height);
    quote!(::wakui2_runtime::Style {margin:#margin,padding:#padding,placement: ::wakui2_runtime::Placement {
        row:#row,column:#column,row_span:#row_span,column_span:#column_span,x:#x,y:#y,width:#width,height:#height
    }})
}

fn template(doc: &Document, name: &str, backend: &Path) -> syn::Result<TokenStream> {
    let t = doc
        .templates
        .get(name)
        .ok_or_else(|| error(format!("unknown template `{name}`")))?;
    let body = control(doc, &t.root, backend, true)?;
    Ok(quote!({
        let __template_root = __wakui_root.clone();
        ::std::rc::Rc::new(move |__view: &#backend::View,__parent: &#backend::Parent,__scope: &str,__wakui_model: ::wakui2_runtime::Store<_>| {
            let __wakui_root = __template_root.clone();
            let __wakui_view = __view;
            let __wakui_parent = __parent.clone();
            #body
        })
    }))
}
fn control(doc: &Document, c: &Control, backend: &Path, scoped: bool) -> syn::Result<TokenStream> {
    control_inner(doc, c, backend, scoped).map_err(|e| error(format!("control `{}`: {e}", c.id())))
}
fn control_inner(
    doc: &Document,
    c: &Control,
    backend: &Path,
    scoped: bool,
) -> syn::Result<TokenStream> {
    let local_id = c.id();
    let id = if scoped {
        quote!(::std::format!("{}/{}",__scope,#local_id))
    } else {
        quote!(::std::string::String::from(#local_id))
    };
    let s = style(&c.style());
    let code = match c {
        Control::Window { .. } => {
            return Err(error("Window is only supported at the document root"));
        }
        Control::StackLayout {
            orientation,
            gap,
            children,
            ..
        } => {
            let horizontal = matches!(orientation, Orientation::Horizontal);
            layout(
                doc,
                children,
                backend,
                scoped,
                quote!(::wakui2_runtime::Layout::Stack {horizontal:#horizontal,gap:#gap}),
            )?
        }
        Control::Grid {
            columns,
            gap,
            children,
            ..
        } => layout(
            doc,
            children,
            backend,
            scoped,
            quote!(::wakui2_runtime::Layout::Grid {columns:#columns,gap:#gap}),
        )?,
        Control::AbsoluteLayout { children, .. } => layout(
            doc,
            children,
            backend,
            scoped,
            quote!(::wakui2_runtime::Layout::Absolute),
        )?,
        Control::FlexLayout { gap, children, .. } => layout(
            doc,
            children,
            backend,
            scoped,
            quote!(::wakui2_runtime::Layout::Flex {gap:#gap}),
        )?,
        Control::Label { text, .. } => {
            let text = binding(text, false)?;
            quote!(__wakui_view.label(&__wakui_parent,&__id,#text);)
        }
        Control::TextBox {
            text, validation, ..
        } => {
            let text = binding(text, true)?;
            let validation = if let Some(v) = validation {
                let validator = path(&v.validator)?;
                let error = field_writer(&v.error)?;
                let pending = field_writer(&v.pending)?;
                quote!(let __token = ::wakui2_runtime::validation(&__wakui_model,__text.clone(),#error,#pending,
                    |__value| ::std::boxed::Box::pin(#validator(__value)));
                    __wakui_view.keep(__token);)
            } else {
                quote!()
            };
            quote!(let __text = #text; #validation __wakui_view.entry(&__wakui_parent,&__id,__text);)
        }
        Control::Button {
            text,
            command: cmd,
            argument,
            ..
        } => {
            let text = binding(text, false)?;
            let name = command(cmd)?;
            let model = command_model(cmd);
            let argument = argument
                .as_ref()
                .map(|p| binding(p, false))
                .transpose()?
                .unwrap_or_else(|| {
                    quote!(::wakui2_runtime::Binding::constant(
                        ::std::string::String::new()
                    ))
                });
            quote!(__wakui_view.button(&__wakui_parent,&__id,#text,#model.#name(#argument));)
        }
        Control::Native { factory, value, .. } => {
            let factory = path(factory)?;
            let value = binding(value, true)?;
            quote!(#factory(&__wakui_view,&__wakui_parent,&__id,#value);)
        }
        Control::ItemsControl {
            items,
            item_template,
            viewport,
            ..
        }
        | Control::BindableLayout {
            items,
            item_template,
            viewport,
            ..
        } => {
            let items = binding(items, false)?;
            let viewport = binding(viewport, true)?;
            let template = template(doc, item_template, backend)?;
            quote!(__wakui_view.items(&__wakui_parent,&__id,#items,#viewport,#template);)
        }
        Control::ContentControl {
            content,
            content_template,
            ..
        } => {
            let item = binding(content, false)?;
            let template = template(doc, content_template, backend)?;
            quote!(__wakui_view.content(&__wakui_parent,&__id,#item,#template);)
        }
    };
    Ok(quote!({let __id = #id; #code __wakui_view.style(&__id,#s);}))
}
fn layout(
    doc: &Document,
    children: &[Control],
    backend: &Path,
    scoped: bool,
    value: TokenStream,
) -> syn::Result<TokenStream> {
    let children: Vec<_> = children
        .iter()
        .map(|c| control(doc, c, backend, scoped))
        .collect::<Result<_, _>>()?;
    Ok(
        quote!(let __wakui_parent = __wakui_view.layout(&__wakui_parent,&__id,#value); #(#children)*),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(content: &str) -> Document {
        wakui2_markup::parse(&format!("schema_version: 2\nroot:\n  type: Window\n  id: w\n  title: {{literal: Title}}\n  content: {content}\n"),"yaml","test.yaml").unwrap()
    }
    #[test]
    fn window_user_id_does_not_identify_the_backend_root() {
        let d = document("{type: Label, id: label, text: {literal: hello}}");
        for id in ["w", "arbitrary-window-id"] {
            let mut d = d.clone();
            let Control::Window { id: window_id, .. } = &mut d.root else {
                unreachable!()
            };
            *window_id = id.into();
            let code = generate(&d, &syn::parse_quote!(backend), false)
                .unwrap()
                .to_string();
            assert!(code.contains(". style (\"__root\" ,"), "{code}");
            assert!(!code.contains(&format!("\"{id}\"")), "{code}");
        }
    }
    #[test]
    fn every_feature_emits_runtime_calls() {
        let d = wakui2_markup::parse(include_str!("../tests/ui/main.yaml"), "yaml", "test.yaml")
            .unwrap();
        let code = generate(&d, &syn::parse_quote!(backend), false).unwrap();
        syn::parse2::<syn::Expr>(quote!({#code})).unwrap();
        let code = code.to_string();
        for word in [
            "layout",
            "style",
            "items",
            "content",
            "label",
            "entry",
            "button",
            "meter",
            "validation",
            "keep",
            "convert",
            "menu",
            "command_choose",
            "command_add",
            "__wakui_root",
            "__scope",
        ] {
            assert!(code.contains(word), "{word}");
        }
        assert!(code.contains("forward"));
        assert!(code.contains("backward"));
        assert!(code.contains("Some (| s , v |"));
    }
    #[test]
    fn no_raw_rust_is_evaluated() {
        for content in [
            "{type: Native, id: n, factory: 'crate::meter(); panic!()', value: {literal: 1}}",
            "{type: TextBox, id: n, text: {bind: {path: name, converter: 'crate::converter()'}}}",
            "{type: Label, id: n, text: {bind: {path: 'name.clone()'}}}",
            "{type: Button, id: n, text: {literal: x}, command: 'add(); panic!()'}",
            "{type: TextBox, id: n, text: {bind: {path: name}}, validation: {validator: 'run()', error: error, pending: pending}}",
            "{type: Native, id: n, factory: 'crate::meter::<String>', value: {literal: 1}}",
        ] {
            let e = generate(&document(content), &syn::parse_quote!(backend), false).unwrap_err();
            assert!(e.to_string().contains("control `n`"), "{e}");
        }
    }
    #[test]
    fn literal_conversion_is_typed_not_token_injection() {
        let code = literal(&serde_json::json!("x\"); panic!()"))
            .unwrap()
            .to_string();
        syn::parse_str::<syn::Expr>(&code).unwrap();
        assert!(code.starts_with(":: std :: string :: String :: from"));
        assert!(
            literal(&serde_json::json!(1))
                .unwrap()
                .to_string()
                .contains("f64")
        );
        assert!(literal(&serde_json::json!({"count":3,"row_height":0})).is_err());
        assert!(literal(&serde_json::json!({"count":-1})).is_err());
        assert!(literal(&serde_json::json!({"unknown":1})).is_err());
    }
}
