use wakui2_markup::*;

fn doc(content: &str) -> String {
    format!(
        "schema_version: 2\nroot:\n  type: Window\n  id: main\n  title: {{literal: Title}}\n  content:\n{}",
        content
            .lines()
            .map(|l| format!("    {l}\n"))
            .collect::<String>()
    )
}
fn parse_yaml(source: &str) -> Result<Document, Error> {
    parse(source, "yaml", "ui/test.yaml")
}
#[test]
fn controls_spacing_placement_and_json() {
    let source = doc(
        "type: Grid\nid: grid\ncolumns: 2\ngap: 4\nmargin: 8\npadding: {top: 1, left: 3}\nstyle:\n  placement: {row: 2, column_span: 2, width: 120}\nchildren:\n  - {type: Label, id: label, text: {bind: {path: name}}}",
    );
    let d = parse_yaml(&source).unwrap();
    let Control::Window { content, .. } = d.root else {
        panic!()
    };
    assert_eq!(content.style().placement.column_span, 2);
    assert!(matches!(content.style().margin, Edges::Scalar(8.0)));
    parse(r#"{"schema_version":2,"root":{"type":"Window","id":"w","title":{"literal":"T"},"content":{"type":"Label","id":"l","text":{"literal":"Hello"}}}}"#,"json","test.json").unwrap();
}
#[test]
fn strict_fields_and_duplicates() {
    for content in [
        "type: Label\nid: l\ntext: {literal: hi}\nunknown: true",
        "type: Label\nid: l\nid: again\ntext: {literal: hi}",
        "type: Label\nid: l\ntext: {bind: {path: name, typo: true}}",
        "type: Label\nid: l\ntext: {literal: hi, bind: {path: name}}",
        "type: Label\nid: l\ntext: {literal: hi}\nstyle: {placement: {typo: 2}}",
        "type: Grid\nid: l\ncolumns: 0",
        "type: Grid\nid: l\ncolumns: 1\ngap: -1",
        "type: Label\nid: l\ntext: {literal: hi}\nmargin: {typo: 1}",
    ] {
        let e = parse_yaml(&doc(content)).unwrap_err();
        assert!(e.to_string().contains("ui/test.yaml"), "{e}");
    }
    let e = parse(
        r#"{"schema_version":2,"schema_version":2}"#,
        "json",
        "duplicate.json",
    )
    .unwrap_err();
    assert!(e.message.contains("duplicate"));
    assert!(parse_yaml(&doc("type: StackLayout\nid: s\nchildren:\n  - {type: Label, id: x, text: {literal: a}}\n  - {type: Label, id: x, text: {literal: b}}")).unwrap_err().message.contains("duplicate id"));
}
#[test]
fn backend_root_id_is_reserved_in_all_markup_id_scopes() {
    let source = doc("type: Label\nid: label\ntext: {literal: hi}");
    for (source, path) in [
        (source.replace("id: main", "id: __root"), "root"),
        (source.replace("id: label", "id: __root"), "root.content"),
        (
            format!(
                "{source}  menus:\n    - {{id: __root, title: {{literal: Actions}}, items: {{bind: {{path: menu_items}}}}, command: choose}}\n"
            ),
            "root",
        ),
        (
            format!(
                "{source}templates:\n  unused:\n    root: {{type: Label, id: __root, text: {{literal: hi}}}}\n"
            ),
            "templates.unused.root",
        ),
    ] {
        let e = parse_yaml(&source).unwrap_err();
        assert_eq!(e.path, path);
        assert!(
            e.message.contains("__root") && e.message.contains("reserved"),
            "{e}"
        );
    }
    parse_yaml(&source.replace("id: main", "id: arbitrary-window-id")).unwrap();
    parse_yaml(&source.replace("id: label", "id: __root_suffix")).unwrap();
}
#[test]
fn lexical_yaml_rejects_features_not_literals() {
    for value in ["&anchor hello", "*alias", "!custom hello", "!!str hello"] {
        let e = parse_yaml(&doc(&format!(
            "type: Label\nid: l\ntext: {{literal: {value}}}"
        )))
        .unwrap_err();
        assert!(e.message.contains("forbidden"), "{e}");
        assert!(e.message.contains("line"));
    }
    for value in [
        "'&anchor *alias !tag <<: x'",
        r#""&a \" *b !c""#,
        r#"'it''s !literal'"#,
    ] {
        parse_yaml(&doc(&format!(
            "type: Label\nid: l\ntext: {{literal: {value}}}"
        )))
        .unwrap();
    }
    parse_yaml(&doc("type: Label\nid: l\ntext:\n  literal: |\n    &anchor *alias !tag <<: text\n# &anchor *alias !tag")).unwrap();
    assert!(
        parse_yaml(&doc(
            "type: Label\nid: l\ntext: {literal: x}\n<<: {id: merged}"
        ))
        .unwrap_err()
        .message
        .contains("merge")
    );
}

#[test]
fn lexer_understands_yaml_not_just_quote_characters() {
    for text in [
        "rock &roll *stars !wow",
        "Don't open quotes",
        "abc:'not a quote",
        "hello,world",
        "<<",
    ] {
        parse_yaml(&doc(&format!(
            "type: Label\nid: n\ntext:\n  literal: {text}"
        )))
        .unwrap();
    }
    let d = parse_yaml(&doc("type: Label\nid: n\ntext: {literal: yes}")).unwrap();
    let Control::Window { content, .. } = d.root else {
        panic!()
    };
    assert_eq!(content.id(), "n");
    parse_yaml(&doc(
        "type: Label\nid: l\ntext:\n  literal: \"a multiline\n    &anchor *alias !tag\"\n# !tag",
    ))
    .unwrap();
    // A quote within a continued plain scalar cannot hide a subsequent anchor.
    let source =
        doc("type: Label\nid: l\ntext:\n  literal: some\n    'not a quoted scalar\nmargin: &bad 1");
    assert!(
        parse_yaml(&source)
            .unwrap_err()
            .message
            .contains("forbidden")
    );
    for prefix in [
        "--- &bad\n",
        "--- !custom\n",
        "%TAG !app! tag:example.com,2026:\n---\n",
    ] {
        assert!(
            parse_yaml(&format!(
                "{prefix}{}",
                doc("type: Label\nid: l\ntext: {literal: hi}")
            ))
            .unwrap_err()
            .message
            .contains("forbidden")
        );
    }
}
#[test]
fn bounds_are_checked_before_typed_deserialization() {
    assert!(
        parse_yaml(&" ".repeat(MAX_BYTES + 1))
            .unwrap_err()
            .message
            .contains("1 MiB")
    );
    let deep = format!("{}0{}", "[".repeat(70), "]".repeat(70));
    assert!(
        parse(&deep, "json", "depth.json")
            .unwrap_err()
            .message
            .contains("depth")
    );
    assert!(parse_yaml(&deep).unwrap_err().message.contains("depth"));
    let wide = format!("[{}]", vec!["0"; MAX_NODES + 1].join(","));
    assert!(
        parse(&wide, "json", "wide.json")
            .unwrap_err()
            .message
            .contains("budget")
    );
    assert!(parse_yaml(&wide).unwrap_err().message.contains("budget"));
}
#[test]
fn recursive_and_missing_templates_are_diagnosed() {
    let source = "schema_version: 2\ntemplates:\n  a:\n    root: {type: ContentControl, id: a, content: {bind: {path: selected}}, content_template: b}\n  b:\n    root: {type: ItemsControl, id: b, items: {bind: {path: rows}}, viewport: {bind: {path: viewport}}, item_template: a}\nroot:\n  type: Window\n  id: w\n  title: {literal: title}\n  content: {type: Label, id: l, text: {literal: x}}\n";
    assert!(
        parse_yaml(source)
            .unwrap_err()
            .message
            .contains("recursive template")
    );
    let missing = doc(
        "type: ContentControl\nid: c\ncontent: {bind: {path: selected}}\ncontent_template: missing",
    );
    assert!(
        parse_yaml(&missing)
            .unwrap_err()
            .message
            .contains("unknown template")
    );
}
#[test]
fn source_diagnostics_schema_and_fingerprint() {
    let e = parse_yaml("root: [unterminated").unwrap_err();
    assert!(e.filename == "ui/test.yaml");
    assert!(
        e.message.contains("line") || e.message.contains("Line"),
        "{e}"
    );
    let e = parse_yaml(&doc("type: Grid\nid: g\ncolumns: nope")).unwrap_err();
    assert!(e.path.contains("root"), "{e}");
    let schema = schema().to_string();
    for name in [
        "Window",
        "StackLayout",
        "Grid",
        "AbsoluteLayout",
        "FlexLayout",
        "BindableLayout",
        "ItemsControl",
        "ContentControl",
        "Label",
        "TextBox",
        "Button",
        "Native",
        "TwoWay",
        "converter",
        "placement",
    ] {
        assert!(schema.contains(name), "{name}");
    }
    assert_eq!(fingerprint(""), "cbf29ce484222325");
    assert_eq!(fingerprint("hello"), "a430d84680aabd0b");
    assert_ne!(fingerprint("hello"), fingerprint("Hello"));
}

#[test]
fn template_expansion_is_bounded_even_for_acyclic_graphs() {
    let mut source = String::from(
        "schema_version: 2\ntemplates:\n  t0:\n    root: {type: Label, id: leaf, text: {literal: leaf}}\n",
    );
    for i in 1..16 {
        source.push_str(&format!("  t{i}:\n    root:\n      type: StackLayout\n      id: stack\n      children:\n        - {{type: ContentControl, id: a, content: {{bind: {{path: selected}}}}, content_template: t{prev}}}\n        - {{type: ContentControl, id: b, content: {{bind: {{path: selected}}}}, content_template: t{prev}}}\n",prev=i-1));
    }
    source.push_str("root:\n  type: Window\n  id: w\n  title: {literal: title}\n  content: {type: ContentControl, id: c, content: {bind: {path: selected}}, content_template: t15}\n");
    assert!(parse_yaml(&source).unwrap_err().message.contains("budget"));
}

#[test]
fn mixed_layout_and_template_depth_is_bounded() {
    let mut source = String::from(
        "schema_version: 2\ntemplates:\n  t0:\n    root: {type: Label, id: leaf, text: {literal: leaf}}\n",
    );
    for i in 1..36 {
        source.push_str(&format!("  t{i}:\n    root:\n      type: StackLayout\n      id: stack\n      children:\n        - {{type: ContentControl, id: content, content: {{bind: {{path: selected}}}}, content_template: t{prev}}}\n",prev=i-1));
    }
    source.push_str("root:\n  type: Window\n  id: w\n  title: {literal: title}\n  content: {type: ContentControl, id: c, content: {bind: {path: selected}}, content_template: t35}\n");
    assert!(parse_yaml(&source).unwrap_err().message.contains("depth"));
}

#[test]
fn unsupported_or_invalid_structures_have_explicit_errors() {
    for content in [
        "type: Native\nid: native\nfactory: crate::meter\nvalue: {literal: 1}\nchildren: []",
        "type: TextBox\nid: entry\ntext: {literal: hi}\nvalidation: {validator: crate::validate, error: error, pending: pending, unknown: true}",
        "type: Label\nid: label\ntext: {literal: hi}\nstyle: {placement: {row_span: 0}}",
        "type: Label\nid: label\ntext: {literal: hi}\nstyle: {placement: {width: -1}}",
        "type: Label\nid: label\ntext: {literal: hi}\nmargin: .inf",
        "type: Label\nid: label\ntext: {literal: hi}\nmargin: -1",
        "type: Label\nid: bad/id\ntext: {literal: hi}",
        "type: Label\nid: ''\ntext: {literal: hi}",
        "type: Window\nid: nested\ntitle: {literal: title}\ncontent: {type: Label, id: label, text: {literal: hi}}",
        "type: UnknownControl\nid: unknown",
    ] {
        assert!(parse_yaml(&doc(content)).is_err(), "{content}");
    }
    let source = doc("type: Label\nid: label\ntext: {literal: hi}");
    assert!(
        parse_yaml(&source.replace("schema_version: 2", "schema_version: 1"))
            .unwrap_err()
            .path
            .contains("schema_version")
    );
    assert!(parse(&source, "xml", "unsupported.xml").is_err());
    assert!(parse_yaml(&format!("{source}\n---\n{source}")).is_err());
}
