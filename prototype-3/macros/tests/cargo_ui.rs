//! Real downstream compiler/rebuild probes, without trybuild or native backends.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Mutex, MutexGuard, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
static CARGO_SESSION: Mutex<()> = Mutex::new(());

fn session() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Never use the outer cargo target: it is locked while libtest runs.
        let root =
            PathBuf::from("/tmp/wakui3-ui-check").join(format!("{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    })
}

struct Fixture {
    root: PathBuf,
    _session: MutexGuard<'static, ()>,
}

impl Fixture {
    fn copy(name: &str) -> Self {
        // Hold across check/build/run, not just individual Cargo invocations:
        // all fixture binaries are named probe and share a dependency cache.
        let guard = CARGO_SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let package = Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = package.join("tests/compile_fail");
        let root = session().join(format!("{name}-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&root).unwrap();
        for file in ["main.rs", "ui.yaml"] {
            let fixture = if name == "self_arguments" && file == "ui.yaml" {
                "pass"
            } else {
                name
            };
            let destination = root.join(file);
            fs::copy(source.join(fixture).join(file), &destination).unwrap();
            fs::File::options()
                .write(true)
                .open(destination)
                .unwrap()
                .set_modified(SystemTime::now())
                .unwrap();
        }
        fs::copy(source.join("support.rs"), root.join("support.rs")).unwrap();
        fs::File::options()
            .write(true)
            .open(root.join("support.rs"))
            .unwrap()
            .set_modified(SystemTime::now())
            .unwrap();
        // New argument probes reuse the immutable pass manifest.
        let manifest_fixture = if matches!(name, "wrong_argument" | "self_arguments") {
            "pass"
        } else {
            name
        };
        let mut manifest =
            fs::read_to_string(source.join(manifest_fixture).join("Cargo.toml")).unwrap();
        for dependency in ["runtime", "macros"] {
            let path = package
                .parent()
                .unwrap()
                .join(dependency)
                .canonicalize()
                .unwrap();
            let quoted = serde_json::to_string(path.to_str().unwrap()).unwrap();
            manifest = manifest.replace(&format!("\"../../../../{dependency}\""), &quoted);
        }
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        assert!(!root.join("build.rs").exists());
        Self {
            root,
            _session: guard,
        }
    }

    fn cargo(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO"))
            .current_dir(&self.root)
            .env("CARGO_TARGET_DIR", session().join("target"))
            .env("CARGO_TERM_COLOR", "never")
            .env(
                "RUSTC_WRAPPER",
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("scripts/yaml-rustc.py"),
            )
            .args(args)
            .arg("--offline")
            .output()
            .expect("start downstream cargo");
        println!(
            "{}: cargo {} --offline => {}\n{}",
            self.root.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn rejects(&self, expected: &[&str]) {
        self.rejects_at(expected, Some(8));
    }
    fn rejects_at(&self, expected: &[&str], expected_line: Option<u64>) {
        let output = self.cargo(&["check", "--message-format=json"]);
        assert!(!output.status.success(), "fixture unexpectedly compiled");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = format!("{}\n{stdout}", String::from_utf8(output.stderr).unwrap());
        for text in expected {
            assert!(stderr.contains(text), "missing {text:?}:\n{stderr}");
        }
        let yaml_spans: Vec<_> = stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|message| message["reason"] == "compiler-message")
            .flat_map(|message| {
                message["message"]["spans"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter(|span| {
                span["is_primary"] == true
                    && span["file_name"]
                        .as_str()
                        .is_some_and(|path| path.ends_with("/ui.yaml"))
            })
            .collect();
        assert!(
            expected_line.is_none() || !yaml_spans.is_empty(),
            "missing YAML primary span: {stderr}"
        );
        for span in yaml_spans {
            assert!(span["line_start"].as_u64().unwrap() > 0);
            if let Some(line) = expected_line {
                assert_eq!(span["line_start"], line, "incorrect YAML node: {stderr}");
            }
            println!(
                "YAML PRIMARY {}:{}:{}",
                span["file_name"], span["line_start"], span["column_start"]
            );
        }
    }

    fn build_fresh(&self) -> bool {
        let output = self.cargo(&["build", "--message-format=json"]);
        assert!(output.status.success(), "downstream build failed");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let artifact = stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|v| v["reason"] == "compiler-artifact" && v["target"]["name"] == "probe")
            .unwrap_or_else(|| panic!("missing probe artifact: {stdout}"));
        let fresh = artifact["fresh"].as_bool().unwrap();
        println!("probe compiler-artifact fresh={fresh}");
        fresh
    }

    fn run(&self) -> String {
        let output = self.cargo(&["run", "--quiet", "--bin", "probe"]);
        assert!(output.status.success(), "downstream executable failed");
        let stdout = String::from_utf8(output.stdout).unwrap();
        println!("probe stdout={stdout:?}");
        stdout
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Keep failed inputs and build artifacts for diagnosis; never touch sources.
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
fn owned_view_requires_a_non_clone_lifecycle_owner() {
    let fixture = Fixture::copy("pass");
    let main = fs::read_to_string(fixture.root.join("main.rs")).unwrap();
    fs::write(
        fixture.root.join("main.rs"),
        main.replace("::view!", "::owned_view!"),
    )
    .unwrap();
    fixture.rejects_at(&["StoreOwner", "mismatched types"], None);
}

#[test]
fn rustc_rejects_nonexistent_binding_field() {
    Fixture::copy("missing_field").rejects(&["E0609", "no field", "nonexistent_field"]);
}

#[test]
fn rustc_rejects_wrong_binding_type() {
    Fixture::copy("wrong_type").rejects(&[
        "E0308",
        "mismatched types",
        "Binding<String>",
        "Binding<bool>",
    ]);
}

#[test]
fn rustc_rejects_unknown_command() {
    Fixture::copy("unknown_command").rejects(&[
        "E0599",
        "no method",
        "command_nonexistent_command",
    ]);
}

#[test]
fn rustc_rejects_wrong_typed_command_argument() {
    Fixture::copy("wrong_argument").rejects(&[
        "E0308",
        "mismatched types",
        "expected `i32`",
        "found `String`",
    ]);
}

fn document(content: &str) -> String {
    format!(
        "schema_version: 2\nroot:\n  type: Window\n  id: window\n  title: {{literal: Probe}}\n  content:\n{}\n",
        content
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[test]
fn yaml_schema_typo_has_a_primary_source_span() {
    let fixture = Fixture::copy("pass");
    fs::write(
        fixture.root.join("ui.yaml"),
        document("type: Label\nid: label\ntexte: {literal: typo}"),
    )
    .unwrap();
    fixture.rejects_at(&["unknown field", "texte"], Some(9));
}

#[test]
fn rust_reference_signatures_are_checked_at_the_yaml_node() {
    let fixture = Fixture::copy("pass");
    let support = fixture.root.join("support.rs");
    let original = fs::read_to_string(&support).unwrap();
    fs::write(support, format!("{original}\npub fn wrong_factory(_: &fake::View, _: &fake::Parent, _: &str, _: Binding<bool>) {{}}\npub fn wrong_factory_result(_: &fake::View, _: &fake::Parent, _: &str, _: Binding<String>) -> bool {{ true }}\npub async fn wrong_validator(_: bool) -> Result<(), String> {{ Ok(()) }}\npub mod wrong_converter {{ pub fn forward(_: bool) -> String {{ String::new() }} pub fn backward(_: String) -> Result<bool, String> {{ Ok(true) }} }}\n")).unwrap();
    for (content, expected) in [
        (
            "type: Native\nid: native\nfactory: support::wrong_factory\nvalue: {bind: {path: name}}",
            "expected `bool`, found `String`",
        ),
        (
            "type: Native\nid: native\nfactory: support::wrong_factory_result\nvalue: {bind: {path: name}}",
            "expected `()`, found `bool`",
        ),
        (
            "type: TextBox\nid: entry\ntext: {bind: {path: name}}\nvalidation: {validator: support::wrong_validator, error: name, pending: flag}",
            "expected `bool`, found `String`",
        ),
        (
            "type: TextBox\nid: entry\ntext: {bind: {path: name, converter: support::wrong_converter}}",
            "expected",
        ),
    ] {
        fs::write(fixture.root.join("ui.yaml"), document(content)).unwrap();
        fixture.rejects(&[expected]);
    }
}

#[test]
fn typed_menu_arguments_compile_execute_and_reject_wrong_commands() {
    let fixture = Fixture::copy("pass");
    let yaml = "schema_version: 2\nroot:\n  type: Window\n  id: window\n  title: {literal: Menu}\n  content: {type: Label, id: label, text: {literal: menu}}\n  menus:\n    - id: file\n      title: {literal: File}\n      items: {bind: {path: items}}\n      command: typed\n";
    fs::write(fixture.root.join("ui.yaml"), yaml).unwrap();
    fs::write(fixture.root.join("main.rs"), "mod support;\nuse support::{fake, ModelCommands};\nfn main() { let model = support::model(); let _ = wakui2_macros::view!(\"ui.yaml\", model, fake); assert_eq!(model.snapshot().name, \"42\"); println!(\"typed-menu-ok\"); }\n").unwrap();
    assert_eq!(fixture.run(), "typed-menu-ok\n");
    fs::write(
        fixture.root.join("ui.yaml"),
        yaml.replace("command: typed", "command: reset"),
    )
    .unwrap();
    fixture.rejects(&["E0308", "mismatched types"]);
}

#[test]
fn self_arguments_compile_and_execute_downstream() {
    assert_eq!(Fixture::copy("self_arguments").run(), "self-arguments-ok\n");
}

#[test]
fn yaml_only_edit_rebuilds_and_changes_executable_output() {
    let fixture = Fixture::copy("pass");
    assert!(
        !fixture.build_fresh(),
        "first build must compile the fixture"
    );
    assert_eq!(fixture.run(), "before-yaml-edit\n");
    assert!(fixture.build_fresh(), "unchanged inputs must be fresh");

    let rust_before = fs::read(fixture.root.join("main.rs")).unwrap();
    let support_before = fs::read(fixture.root.join("support.rs")).unwrap();
    let manifest_before = fs::read(fixture.root.join("Cargo.toml")).unwrap();
    let yaml = fixture.root.join("ui.yaml");
    let before = fs::read_to_string(&yaml).unwrap();
    let after = before.replace("before-yaml-edit", "after-yaml-edit-with-different-length");
    assert_ne!(
        wakui2_markup::fingerprint(&before),
        wakui2_markup::fingerprint(&after)
    );
    // Both content and length change; no sleep, touch of Rust, clean or build.rs.
    fs::write(&yaml, after).unwrap();

    assert!(
        !fixture.build_fresh(),
        "include_str! must invalidate the artifact"
    );
    assert_eq!(fixture.run(), "after-yaml-edit-with-different-length\n");
    assert!(
        fixture.build_fresh(),
        "rebuilt YAML input must become fresh again"
    );
    for (file, before) in [
        ("main.rs", rust_before),
        ("support.rs", support_before),
        ("Cargo.toml", manifest_before),
    ] {
        assert_eq!(
            fs::read(fixture.root.join(file)).unwrap(),
            before,
            "{file} changed"
        );
    }
}

#[test]
fn macro_rejects_resource_exhaustion_and_yaml_aliases() {
    let fixture = Fixture::copy("pass");
    let yaml = fixture.root.join("ui.yaml");
    let before = fs::read_to_string(&yaml).unwrap();
    let deep = format!(
        "{}0{}",
        "[".repeat(wakui2_markup::MAX_DEPTH + 2),
        "]".repeat(wakui2_markup::MAX_DEPTH + 2)
    );
    let wide = format!("[{}]", vec!["0"; wakui2_markup::MAX_NODES + 1].join(","));
    for (source, diagnostic) in [
        (
            " ".repeat(wakui2_markup::MAX_BYTES + 1),
            "input exceeds 1 MiB",
        ),
        (deep, "maximum depth 64 exceeded"),
        (wide, "node budget exceeded"),
        (
            before.replace("before-yaml-edit", "*undefined_alias"),
            "aliases, anchors and tags are forbidden",
        ),
        (
            before.replace("before-yaml-edit", "&anchor anchored"),
            "aliases, anchors and tags are forbidden",
        ),
    ] {
        fs::write(&yaml, source).unwrap();
        fixture.rejects_at(&["ui.yaml", diagnostic], None);
    }
}
