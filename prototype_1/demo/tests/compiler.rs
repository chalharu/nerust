use demo_contract::Settings;
use serde_json::{Value, json};
use wakui_build::{Backend, Compiler, parse};
use wakui_contract::Contract;
use wakui_definition::{Document, Registry};

const YAML: &str = include_str!("../ui/settings.yaml");

fn registry() -> Registry {
    let mut registry = Registry::default();
    wakui_definition::register(&mut registry).unwrap();
    demo_controls::register(&mut registry).unwrap();
    registry
}
fn document() -> Document {
    parse(YAML, "yaml", &registry()).unwrap()
}
fn value() -> Value {
    serde_json::to_value(document()).unwrap()
}
fn generate(document: &Document, backend: Backend) -> Result<String, wakui_build::Error> {
    Compiler::generate(
        document,
        &Settings::descriptor(),
        backend,
        "demo_contract::Settings",
    )
}
fn invalid(value: Value, expected: &str) {
    let source = serde_json::to_string(&value).unwrap();
    let error = parse(&source, "json", &registry())
        .and_then(|document| generate(&document, Backend::Gtk))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error}"
    );
}

#[test]
fn json_yaml_and_dto_roundtrip_generate_identical_native_code() {
    let yaml = document();
    let json = serde_json::to_string(&yaml).unwrap();
    let decoded = parse(&json, "json", &registry()).unwrap();
    assert_eq!(
        generate(&yaml, Backend::Gtk).unwrap(),
        generate(&decoded, Backend::Gtk).unwrap()
    );
    let code = generate(&yaml, Backend::Tao).unwrap();
    assert!(code.contains("use wakui_tao as backend"));
    assert!(code.contains("view.entry("));
    assert!(code.contains("view.items("));
    assert!(!code.contains("typetag"));
    assert!(!code.contains("serde_json"));
    assert!(!code.contains("sample.Heading")); // Extension was lowered at build time.
    assert_eq!(
        code.matches("model.command(&demo_contract::SAVE)").count(),
        2
    );
}

#[test]
fn external_control_requires_explicit_host_registration() {
    let mut only_standard = Registry::default();
    wakui_definition::register(&mut only_standard).unwrap();
    let error = parse(YAML, "yaml", &only_standard).unwrap_err();
    assert!(error.to_string().contains("sample.Heading"));
    let mut registry = registry();
    assert!(
        demo_controls::register(&mut registry)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
fn unknown_fields_and_control_tags_are_errors() {
    let mut v = value();
    v["root"]["content"]["children"][2]["typo"] = json!(true);
    invalid(v, "unknown field");
    let mut v = value();
    v["root"]["type"] = json!("missing.Control");
    invalid(v, "unregistered");
}

#[test]
fn bindings_are_typed_and_checked_before_native_codegen() {
    let mut v = value();
    v["root"]["content"]["children"][3]["text"]["bind"]["path"] = json!("busy");
    invalid(v, "expected Text, got Bool");
    let mut v = value();
    v["root"]["content"]["children"][2]["text"]["bind"]["path"] = json!("status");
    invalid(v, "TwoWay requires");
    let mut v = value();
    v["root"]["content"]["children"][3]["text"]["bind"]["path"] = json!("missing");
    invalid(v, "unknown Binding");
}

#[test]
fn readonly_attributes_cannot_request_two_way() {
    let mut v = value();
    v["root"]["content"]["children"][3]["text"]["bind"]["mode"] = json!("two_way");
    invalid(v, "TwoWay requires");
}

#[test]
fn one_way_entry_does_not_expose_writable_model_key() {
    let mut v = value();
    v["root"]["content"]["children"][2]["text"]["bind"]["mode"] = json!("one_way");
    let document = parse(&v.to_string(), "json", &registry()).unwrap();
    let code = generate(&document, Backend::Gtk).unwrap();
    assert!(code.contains(
        "view.entry(&panel0, \"name\", model.bind(&demo_contract::NAME).map(|value| value))"
    ));
}

#[test]
fn duplicate_node_ids_and_commands_are_diagnosed() {
    let mut v = value();
    v["root"]["content"]["children"][1]["common"]["id"] = json!("heading");
    invalid(v, "duplicate or empty node id");
    let mut v = value();
    v["root"]["menubar"]["items"][0]["items"][0]["command"]["path"] = json!("missing");
    invalid(v, "unknown Command");
}

#[test]
fn templates_are_checked_against_item_contract_and_capabilities() {
    let mut v = value();
    v["templates"]["row"]["data_type"] = json!("wrong.Contract");
    invalid(v, "wrong template data_type");
    let mut v = value();
    v["templates"]["row"]["root"]["text"]["bind"]["path"] = json!("missing");
    invalid(v, "invalid row binding");
    let mut v = value();
    v["root"]["content"]["children"][9]["item_key"] = json!("position");
    invalid(v, "invalid item key/mode");
    let mut v = value();
    v["templates"]["row"]["root"]["type"] = json!("wakui.TextBox");
    invalid(v, "supports Label row templates only");
}

#[test]
fn malformed_property_and_duplicate_map_keys_are_rejected() {
    let mut v = value();
    v["root"]["title"] = json!({"literal":"title","bind":{"path":"name"}});
    invalid(v, "map with a single key");
    assert!(
        parse("{\"a\":1,\"a\":2}", "json", &registry())
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    assert!(parse("a: 1\na: 2\n", "yaml", &registry()).is_err());
}

#[test]
fn schema_backend_and_parent_child_constraints_are_enforced() {
    assert!(Backend::select(false, false).is_err());
    assert!(Backend::select(true, true).is_err());
    let mut v = value();
    v["schema_version"] = json!(2);
    invalid(v, "unsupported schema");
    let mut v = value();
    v["root"]["content"] = v["root"].clone();
    invalid(v, "Window cannot be a child");
}

#[test]
fn generated_literals_are_escaped_as_rust_tokens() {
    let mut v = value();
    v["root"]["title"] = json!({"literal":"\"; panic!(\"injected\"); //\n"});
    let document = parse(&v.to_string(), "json", &registry()).unwrap();
    assert!(
        generate(&document, Backend::Gtk)
            .unwrap()
            .contains("\\\"; panic!(\\\"injected\\\"); //\\n")
    );
}
