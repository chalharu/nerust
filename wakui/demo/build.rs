use std::path::PathBuf;
use wakui_contract::Contract;

fn main() {
    let backend = wakui_build::Backend::select(
        std::env::var_os("CARGO_FEATURE_BACKEND_GTK").is_some(),
        std::env::var_os("CARGO_FEATURE_BACKEND_TAO").is_some(),
    )
    .expect("invalid backend selection");
    let mut registry = wakui_definition::Registry::default();
    wakui_definition::register(&mut registry).unwrap();
    demo_controls::register(&mut registry).unwrap();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let path =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("ui/settings.yaml");
    wakui_build::compile_file(
        &path,
        &out,
        &registry,
        &demo_contract::Settings::descriptor(),
        backend,
        "demo_contract::Settings",
        &std::env::var("TARGET").unwrap(),
    )
    .expect("UI compilation failed");
}
