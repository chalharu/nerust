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
            fs::copy(source.join(name).join(file), root.join(file)).unwrap();
        }
        fs::copy(source.join("support.rs"), root.join("support.rs")).unwrap();
        // New argument probes reuse the immutable pass manifest.
        let manifest_fixture = if name == "wrong_argument" {
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
        let output = self.cargo(&["check"]);
        assert!(!output.status.success(), "fixture unexpectedly compiled");
        let stderr = String::from_utf8(output.stderr).unwrap();
        for text in expected {
            assert!(stderr.contains(text), "missing {text:?}:\n{stderr}");
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
        fixture.rejects(&["ui.yaml", diagnostic]);
    }
}
