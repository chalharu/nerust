use panechord_markup::{
    BindingSource, Control, Document, Edges, Mode, Orientation, Property, Style,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Path};

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
    let title = located(doc, None, doc.root.id(), binding(title, false)?);
    let body = control(doc, content, backend, None)?;
    let style = style(&doc.root.style());
    let appearance = appearance(&doc.root, quote!("__root"))?;
    let mut menu_code = Vec::new();
    for menu in menus {
        let id = &menu.id;
        let title = binding(&menu.title, false)?;
        let items = match &menu.items {
            panechord_markup::MenuItems::Binding(items) => binding(items, false)?,
            panechord_markup::MenuItems::Entries(entries) => {
                let entries = entries.iter().map(|entry| {
                    let id = &entry.id;
                    let label = binding(&entry.label, false)?;
                    let argument = binding(&entry.argument, false)?;
                    let enabled = entry.enabled.as_ref().map(|value| binding(value, false)).transpose()?.unwrap_or_else(|| quote!(::panechord_runtime::Binding::constant(true)));
                    let role = format_ident!("{}", format!("{:?}", entry.role));
                    let shortcut = match &entry.shortcut {
                        Some(value) => quote!(Some(::std::string::String::from(#value))),
                        None => quote!(None),
                    };
                    Ok(quote!((#label).zip(&(#argument)).zip(&(#enabled)).map(|((label, argument), enabled)| ::panechord_runtime::MenuItem {
                        id: ::std::string::String::from(#id), label, argument, shortcut: #shortcut,
                        role: ::panechord_runtime::MenuRole::#role, enabled,
                    })))
                }).collect::<syn::Result<Vec<_>>>()?;
                quote!(::panechord_runtime::Binding::sequence(vec![#(#entries),*]).distinct())
            }
        };
        let command = command(&menu.command)?;
        let method = if menu.nested {
            format_ident!("hierarchical_menu")
        } else {
            format_ident!("typed_menu")
        };
        menu_code.push(located(
            doc,
            None,
            id,
            quote!({
                let __dispatch_model = __panechord_model.clone();
                __panechord_view.#method(#id,#title,#items,::std::rc::Rc::new(move |__arg| {
                    __dispatch_model.#command(::panechord_runtime::Binding::constant(__arg)).invoke()
                }));
            }),
        ));
    }
    let close = if owned {
        quote!(__panechord_owner.close();)
    } else {
        quote!(let _ = &__close_model;)
    };
    Ok(quote!(
        let __panechord_view = #backend::View::new(#title);
        let __panechord_parent = __panechord_view.root();
        __panechord_view.style("__root",#style);
        #body
        #appearance
        #(#menu_code)*
        let __close_model = __panechord_root.clone();
        __panechord_view.on_close(::std::rc::Rc::new(move || { #close }));
        __panechord_view
    ))
}

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}
fn appearance(control: &Control, id: TokenStream) -> syn::Result<TokenStream> {
    let mut calls = Vec::new();
    for (name, property) in control.appearance() {
        if let Some(property) = property {
            let value = if name == "style_binding" {
                if let Property::Literal(value) = property {
                    let literal =
                        serde_json::from_value::<panechord_markup::Style>(value.literal.clone())
                            .map_err(|error_value| {
                                error(format!("invalid style literal: {error_value}"))
                            })?;
                    let value = style(&literal);
                    quote!(::panechord_runtime::Binding::constant(#value))
                } else {
                    binding(property, false)?
                }
            } else {
                binding(property, false)?
            };
            let method = format_ident!("{name}");
            calls.push(quote!(__panechord_view.#method(#id, #value);));
        }
    }
    Ok(quote!(#(#calls)*))
}
fn located(doc: &Document, scope: Option<&str>, id: &str, body: TokenStream) -> TokenStream {
    let Some(span) = doc.source.node(scope, id) else {
        return body;
    };
    let name = format_ident!("{}", source_marker(&doc.source.filename, span));
    quote!({ macro_rules! #name { () => {{ #body }}; } #name!() })
}
pub fn source_marker(filename: &str, span: panechord_markup::SourceSpan) -> String {
    let encoded: String = filename
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "__panechord_yaml_line_{}_column_{}_file_{}",
        span.line, span.column, encoded
    )
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
        quote!(__panechord_root)
    } else {
        quote!(__panechord_model)
    }
}
fn binding(prop: &Property, writable: bool) -> syn::Result<TokenStream> {
    Ok(match prop {
        Property::Literal(v) => {
            let v = literal(&v.literal)?;
            quote!(::panechord_runtime::Binding::constant(#v))
        }
        Property::Bind(v) => {
            let b = &v.bind;
            let (model, fields) = if let Some(s) = b.path.strip_prefix("root.") {
                (quote!(__panechord_root), s)
            } else {
                (quote!(__panechord_model), b.path.as_str())
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
            let dependency = fields[0].to_string();
            let base = if b.source == BindingSource::External {
                let external = quote!(#model.#(#fields).*.clone());
                if writable && b.mode == Mode::TwoWay {
                    external
                } else {
                    quote!((#external).map(|value| value))
                }
            } else {
                quote!(#model.bind_scoped_explicit(#dependency,|s| s.#(#fields).*.clone(),#writer))
            };
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
        &Property::Bind(panechord_markup::Bound {
            bind: panechord_markup::Binding {
                path: value.into(),
                mode: Mode::TwoWay,
                source: BindingSource::Store,
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
        Value::Object(value) if value.len() == 1 && value.contains_key("u64") => {
            let value = value["u64"]
                .as_u64()
                .ok_or_else(|| error("u64 literal must be an unsigned 64-bit integer"))?;
            let value = proc_macro2::Literal::u64_suffixed(value);
            quote!(#value)
        }
        Value::Object(value) if value.len() == 1 && value.contains_key("i64") => {
            let value = value["i64"]
                .as_i64()
                .ok_or_else(|| error("i64 literal must be a signed 64-bit integer"))?;
            let value = proc_macro2::Literal::i64_suffixed(value);
            quote!(#value)
        }
        Value::Object(value) if value.len() == 1 && value.contains_key("i32") => {
            let value: i32 = value["i32"]
                .as_i64()
                .and_then(|value| value.try_into().ok())
                .ok_or_else(|| error("i32 literal must be a signed 32-bit integer"))?;
            let value = proc_macro2::Literal::i32_suffixed(value);
            quote!(#value)
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
            panechord_runtime::Viewport {
                start,
                count,
                row_height: height,
            }
            .validate()
            .map_err(error)?;
            quote!(::panechord_runtime::Viewport { start:#start,count:#count,row_height:#height })
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
    quote!(::panechord_runtime::Edges {top:#top,right:#right,bottom:#bottom,left:#left})
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
    let alignment = |value: Option<panechord_markup::Alignment>| {
        value.map_or_else(
            || quote!(::std::option::Option::None),
            |value| {
                let name = format_ident!("{}", format!("{value:?}"));
                quote!(::std::option::Option::Some(::panechord_runtime::Alignment::#name))
            },
        )
    };
    let horizontal_alignment = alignment(s.horizontal_alignment);
    let vertical_alignment = alignment(s.vertical_alignment);
    let text = s.text.map_or_else(|| quote!(::std::option::Option::None), |text| {
        let role = format_ident!("{}", format!("{:?}", text.role));
        let scale = text.scale;
        let foreground = text.foreground.map_or_else(|| quote!(::std::option::Option::None), |[red, green, blue]| quote!(::std::option::Option::Some([#red,#green,#blue])));
        quote!(::std::option::Option::Some(::panechord_runtime::TextStyle {role: ::panechord_runtime::TextRole::#role,scale:#scale,foreground:#foreground}))
    });
    quote!(::panechord_runtime::Style {margin:#margin,padding:#padding,horizontal_alignment:#horizontal_alignment,vertical_alignment:#vertical_alignment,text:#text,placement: ::panechord_runtime::Placement {
        row:#row,column:#column,row_span:#row_span,column_span:#column_span,x:#x,y:#y,width:#width,height:#height
    }})
}

fn template(doc: &Document, name: &str, backend: &Path) -> syn::Result<TokenStream> {
    let t = doc
        .templates
        .get(name)
        .ok_or_else(|| error(format!("unknown template `{name}`")))?;
    let body = control(doc, &t.root, backend, Some(name))?;
    Ok(quote!({
        let __template_root = __panechord_root.clone();
        ::std::rc::Rc::new(move |__view: &#backend::View,__parent: &#backend::Parent,__scope: &str,__panechord_model: ::panechord_runtime::Store<_>| {
            let __panechord_root = __template_root.clone();
            let __panechord_view = __view;
            let __panechord_parent = __parent.clone();
            #body
        })
    }))
}
fn control(
    doc: &Document,
    c: &Control,
    backend: &Path,
    scope: Option<&str>,
) -> syn::Result<TokenStream> {
    control_inner(doc, c, backend, scope)
        .map(|body| located(doc, scope, c.id(), body))
        .map_err(|e| {
            let location = doc
                .source
                .node(scope, c.id())
                .map(|span| format!("{}:{}:{}: ", doc.source.filename, span.line, span.column))
                .unwrap_or_default();
            error(format!("{location}control `{}`: {e}", c.id()))
        })
}
fn control_inner(
    doc: &Document,
    c: &Control,
    backend: &Path,
    scoped: Option<&str>,
) -> syn::Result<TokenStream> {
    let local_id = c.id();
    let id = if scoped.is_some() {
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
                quote!(::panechord_runtime::Layout::Stack {horizontal:#horizontal,gap:#gap}),
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
            quote!(::panechord_runtime::Layout::Grid {columns:#columns,gap:#gap}),
        )?,
        Control::AbsoluteLayout { children, .. } => layout(
            doc,
            children,
            backend,
            scoped,
            quote!(::panechord_runtime::Layout::Absolute),
        )?,
        Control::FlexLayout { gap, children, .. } => layout(
            doc,
            children,
            backend,
            scoped,
            quote!(::panechord_runtime::Layout::Flex {gap:#gap}),
        )?,
        Control::Scroll {
            orientation,
            content,
            ..
        } => {
            let horizontal = matches!(orientation, panechord_markup::Orientation::Horizontal);
            let body = control(doc, content, backend, scoped)?;
            quote!(match __panechord_view.try_scroll(&__panechord_parent, &__id, #horizontal) {
                ::std::result::Result::Ok(__panechord_parent) => { #body }
                ::std::result::Result::Err(__error) => __panechord_view.report_error(__error),
            })
        }
        Control::Label { text, .. } => {
            let text = binding(text, false)?;
            quote!(__panechord_view.label(&__panechord_parent,&__id,#text);)
        }
        Control::Separator { orientation, .. } => {
            let vertical = matches!(orientation, Orientation::Vertical);
            quote!(__panechord_view.separator(&__panechord_parent,&__id,#vertical);)
        }
        Control::Table {
            source, selection, ..
        } => {
            let source = binding(source, false)?;
            if let Some(selection) = selection {
                let selection = binding(selection, true)?;
                quote!(__panechord_view.table_with_selection(&__panechord_parent,&__id,#source,#selection);)
            } else {
                quote!(__panechord_view.table(&__panechord_parent,&__id,#source);)
            }
        }
        Control::TextBox {
            text,
            placeholder,
            validation,
            ..
        } => {
            let text = binding(text, true)?;
            let validation = if let Some(v) = validation {
                let validator = path(&v.validator)?;
                let error = field_writer(&v.error)?;
                let pending = field_writer(&v.pending)?;
                quote!(let __token = ::panechord_runtime::validation(&__panechord_model,__text.clone(),#error,#pending,
                    |__value| ::std::boxed::Box::pin(#validator(__value)));
                    __panechord_view.keep(__token);)
            } else {
                quote!()
            };
            let entry = if let Some(placeholder) = placeholder {
                let placeholder = binding(placeholder, false)?;
                quote!(__panechord_view.entry_with_placeholder(&__panechord_parent,&__id,__text,#placeholder);)
            } else {
                quote!(__panechord_view.entry(&__panechord_parent,&__id,__text);)
            };
            quote!(let __text = #text; #validation #entry)
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
                .unwrap_or_else(|| quote!(::panechord_runtime::omitted_command_argument()));
            quote!(__panechord_view.button(&__panechord_parent,&__id,#text,#model.#name(#argument));)
        }
        Control::Native { factory, value, .. } => {
            let factory = path(factory)?;
            let value = binding(value, true)?;
            quote!(let _: () = #factory(&__panechord_view,&__panechord_parent,&__id,#value);)
        }
        Control::CheckBox { text, checked, .. } => {
            let text = binding(text, false)?;
            let checked = binding(checked, true)?;
            quote!(__panechord_view.checkbox(&__panechord_parent, &__id, #text, #checked);)
        }
        Control::Radio {
            text,
            selected,
            value,
            ..
        } => {
            let text = binding(text, false)?;
            let selected = binding(selected, true)?;
            quote!(__panechord_view.radio(&__panechord_parent, &__id, #text, #selected, #value);)
        }
        Control::Slider {
            value,
            min,
            max,
            step,
            ..
        } => {
            let value = binding(value, true)?;
            quote!(__panechord_view.slider(&__panechord_parent, &__id, #value, ::panechord_runtime::SliderRange { min: #min, max: #max, step: #step });)
        }
        Control::Combo {
            items, selected, ..
        } => {
            let items = binding(items, false)?;
            let selected = binding(selected, true)?;
            quote!(__panechord_view.combo(&__panechord_parent, &__id, #items, #selected);)
        }
        Control::Image {
            source, pointer, ..
        } => {
            let source = binding(source, false)?;
            if let Some(pointer) = pointer {
                let pointer = binding(pointer, true)?;
                quote!(__panechord_view.image_with_pointer(&__panechord_parent, &__id, #source, #pointer);)
            } else {
                quote!(__panechord_view.image(&__panechord_parent, &__id, #source);)
            }
        }
        Control::Repeat {
            items,
            item_template,
            gap,
            ..
        } => {
            let items = binding(items, false)?;
            let template = template(doc, item_template, backend)?;
            quote!(__panechord_view.repeat(&__panechord_parent, &__id, #items, #gap, #template);)
        }
        Control::Tab {
            selected,
            pages,
            position,
            ..
        } => {
            let selected = binding(selected, true)?;
            let position = format_ident!(
                "{}",
                match position {
                    panechord_markup::TabPosition::Top => "Top",
                    panechord_markup::TabPosition::Bottom => "Bottom",
                    panechord_markup::TabPosition::Left => "Left",
                    panechord_markup::TabPosition::Right => "Right",
                }
            );
            let mut generated = Vec::new();
            for page in pages {
                let key = &page.id;
                let id = if scoped.is_some() {
                    quote!(::std::format!("{}/{}", __scope, #key))
                } else {
                    quote!(::std::string::String::from(#key))
                };
                let title = binding(&page.title, false)?;
                let body = control(doc, &page.content, backend, scoped)?;
                generated.push(quote!({
                    match __panechord_view.try_tab_page(&__tab, &#id, #title, #key) {
                        ::std::result::Result::Ok(__panechord_parent) => { #body }
                        ::std::result::Result::Err(__error) => __panechord_view.report_error(__error),
                    }
                }));
            }
            quote!(
                let __tab = __panechord_view.tab(&__panechord_parent, &__id, #selected);
                if let ::std::result::Result::Err(__error) = __panechord_view.tab_position(&__id, ::panechord_runtime::TabPosition::#position) {
                    __panechord_view.report_error(__error);
                }
                #(#generated)*
                if let ::std::result::Result::Err(__error) = __panechord_view.finish_tab(&__tab) {
                    __panechord_view.report_error(__error);
                }
            )
        }
        Control::TabItems {
            selected,
            items,
            title,
            item_template,
            position,
            ..
        } => {
            let selected = binding(selected, true)?;
            let items = binding(items, false)?;
            let title = binding(title, false)?;
            let template = template(doc, item_template, backend)?;
            let position = format_ident!("{}", format!("{position:?}"));
            quote!({
                let __title_root = __panechord_root.clone();
                __panechord_view.tab_items(&__panechord_parent, &__id, #items, #selected, ::std::rc::Rc::new(move |__panechord_model: ::panechord_runtime::Store<_>| {
                    let __panechord_root = __title_root.clone();
                    #title
                }), #template);
                if let ::std::result::Result::Err(__error) = __panechord_view.tab_position(&__id, ::panechord_runtime::TabPosition::#position) {
                    __panechord_view.report_error(__error);
                }
            })
        }
        Control::Dialog {
            title,
            open,
            content,
            ..
        } => {
            let title = binding(title, false)?;
            let open = binding(open, true)?;
            let body = control(doc, content, backend, scoped)?;
            quote!(let __panechord_parent = __panechord_view.dialog(&__panechord_parent, &__id, #title, #open); #body)
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
            quote!(__panechord_view.items(&__panechord_parent,&__id,#items,#viewport,#template);)
        }
        Control::ContentControl {
            content,
            content_template,
            ..
        } => {
            let item = binding(content, false)?;
            let template = template(doc, content_template, backend)?;
            quote!(__panechord_view.content(&__panechord_parent,&__id,#item,#template);)
        }
    };
    let appearance = appearance(c, quote!(&__id))?;
    Ok(quote!({let __id = #id; #code __panechord_view.style(&__id,#s); #appearance}))
}
fn layout(
    doc: &Document,
    children: &[Control],
    backend: &Path,
    scoped: Option<&str>,
    value: TokenStream,
) -> syn::Result<TokenStream> {
    let children: Vec<_> = children
        .iter()
        .map(|c| control(doc, c, backend, scoped))
        .collect::<Result<_, _>>()?;
    Ok(
        quote!(match __panechord_view.try_layout(&__panechord_parent,&__id,#value) {
            ::std::result::Result::Ok(__panechord_parent) => { #(#children)* }
            ::std::result::Result::Err(__error) => __panechord_view.report_error(__error),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(content: &str) -> Document {
        panechord_markup::parse(&format!("schema_version: 2\nroot:\n  type: Window\n  id: w\n  title: {{literal: Title}}\n  content: {content}\n"),"yaml","test.yaml").unwrap()
    }
    #[test]
    fn dynamic_tabs_generate_item_scoped_titles_and_typed_template_calls() {
        let mut document = document("{type: Label, id: placeholder, text: {literal: placeholder}}");
        document.templates.insert("page".into(), panechord_markup::Template {
            root: serde_json::from_value(serde_json::json!({"type":"Label", "id":"text", "text":{"bind":{"path":"label"}}})).unwrap(),
        });
        document.root = serde_json::from_value(serde_json::json!({
            "type":"Window", "id":"window", "title":{"literal":"Tabs"},
            "content":{"type":"TabItems", "id":"tabs", "items":{"bind":{"path":"pages"}},
                "selected":{"bind":{"path":"selected","mode":"TwoWay"}}, "position":"Left",
                "title":{"bind":{"path":"label"}}, "item_template":"page"}
        }))
        .unwrap();
        let generated = generate(&document, &syn::parse_str("backend").unwrap(), false)
            .unwrap()
            .to_string();
        assert!(generated.contains("tab_items"));
        assert!(generated.contains("TabPosition :: Left"));
        assert!(generated.contains("__title_root"));
        assert!(generated.contains("s . label . clone"));
        document.templates.clear();
        assert!(generate(&document, &syn::parse_str("backend").unwrap(), false).is_err());
    }

    #[test]
    fn external_bindings_keep_the_original_writer_only_for_two_way_controls() {
        let document = document(
            "{type: StackLayout, id: fields, children: [{type: TextBox, id: edit, text: {bind: {path: name, source: External, mode: TwoWay}}}, {type: Label, id: read, text: {bind: {path: name, source: External}}}]}",
        );
        let code = generate(&document, &syn::parse_quote!(backend), false)
            .unwrap()
            .to_string();
        assert!(code.contains("__panechord_model . name . clone ()"));
        assert!(code.contains(". map (| value | value)"));
        assert!(!code.contains("bind_scoped_explicit"));
    }
    #[test]
    fn static_menu_entries_emit_reactive_bindings_and_native_roles() {
        let document = panechord_markup::parse("schema_version: 2\nroot:\n  type: Window\n  id: main\n  title: {literal: Main}\n  menus:\n    - id: file\n      title: {literal: File}\n      command: choose\n      items:\n        - id: quit\n          label: {bind: {path: label, source: External}}\n          enabled: {bind: {path: enabled, source: External}}\n          argument: {literal: quit}\n          role: Quit\n          shortcut: Primary+Q\n  content: {type: Label, id: content, text: {literal: Main}}", "yaml", "menu.yaml").unwrap();
        let code = generate(&document, &syn::parse_quote!(backend), false)
            .unwrap()
            .to_string();
        assert!(code.contains("Binding :: sequence"));
        assert!(code.contains("MenuRole :: Quit"));
        assert!(code.contains("command_choose"));
    }
    #[test]
    fn required_controls_emit_typed_backend_calls_and_nested_content() {
        let d = document(
            "{type: StackLayout, id: screen, children: [\
            {type: CheckBox, id: check, text: {literal: Enabled}, checked: {bind: {path: enabled, mode: TwoWay}}},\
            {type: Radio, id: radio, text: {literal: Video}, selected: {bind: {path: choice, mode: TwoWay}}, value: video},\
            {type: Slider, id: slider, value: {bind: {path: volume, mode: TwoWay}}, min: 0, max: 100, step: 1},\
            {type: Combo, id: combo, items: {bind: {path: choices}}, selected: {bind: {path: choice, mode: TwoWay}}},\
            {type: Image, id: image, source: {bind: {path: frame}}},\
            {type: Scroll, id: viewport, content: {type: Label, id: viewport_content, text: {literal: Content}}},\
            {type: Tab, id: tabs, selected: {bind: {path: tab, mode: TwoWay}}, pages: [{id: video, title: {literal: Video}, content: {type: Label, id: video_content, text: {literal: Video}}}]},\
            {type: Dialog, id: dialog, title: {literal: Settings}, open: {bind: {path: open, mode: TwoWay}}, content: {type: Label, id: dialog_content, text: {literal: Dialog}}}\
        ]}",
        );
        let code = generate(&d, &syn::parse_quote!(backend), false).unwrap();
        syn::parse2::<syn::Expr>(quote!({ #code })).unwrap();
        for method in [
            "checkbox",
            "radio",
            "slider",
            "combo",
            "image",
            "try_scroll",
            "viewport_content",
            "tab_page",
            "dialog",
            "dialog_content",
        ] {
            assert!(code.to_string().contains(method), "{method}");
        }
    }

    #[test]
    fn tab_positions_emit_runtime_enums() {
        for position in ["Top", "Bottom", "Left", "Right"] {
            let document = document(&format!(
                "{{type: Tab, id: tabs, position: {position}, selected: {{literal: video}}, pages: [{{id: video, title: {{literal: Video}}, content: {{type: Label, id: label, text: {{literal: Video}}}}}}]}}"
            ));
            let code = generate(&document, &syn::parse_quote!(backend), false).unwrap();
            syn::parse2::<syn::Expr>(quote!({ #code })).unwrap();
            assert!(
                code.to_string()
                    .contains(&format!("TabPosition :: {position}"))
            );
        }
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
        let d = panechord_markup::parse(include_str!("../tests/ui/main.yaml"), "yaml", "test.yaml")
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
            "__panechord_root",
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
