#!/usr/bin/env python3
"""Run schema-derived probes through the real Rust decoder, without an SDK.

The isolated crate avoids building unrelated GUI/GPU backends. Only the input
key enum and request-id wrapper are fixtures; codec and validator are real files.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("generator", Path(__file__).with_name("generate-contract.py"))
GEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GEN)


def sample(prop):
    if "const" in prop:
        return prop["const"]
    if "enum" in prop:
        return prop["enum"][0]
    return {"integer": max(1, prop.get("minimum", 1)), "string": "probe", "boolean": True}[GEN.field_type(prop)]


def probes(schema):
    cases = []
    for event, definition in GEN.event_definitions(schema).items():
        props = definition["properties"]
        base = {name: sample(prop) for name, prop in props.items()}
        cases.append((f"{event}: valid", base, True))
        for name, prop in props.items():
            missing = dict(base)
            del missing[name]
            required = name in definition["required"]
            cases.append((f"{event}: missing {'required' if required else 'optional'} {name}", missing, not required))
            kind = GEN.field_type(prop)
            wrong = {"integer": "not an integer", "boolean": "not a boolean", "string": 7}[kind]
            for value in (None, {}, [], wrong):
                cases.append((f"{event}: {name}={value!r}", dict(base, **{name: value}), False))
            cases.append((f"{event}: wrong type {name}", dict(base, **{name: wrong}), False))
            if "enum" in prop:
                for value in prop["enum"]:
                    payload = dict(base, **{name: value})
                    cases.append((f"{event}: {name}={value!r}", payload, True))
                cases.append((f"{event}: {name}='__unknown__'", dict(base, **{name: "__unknown__"}), False))
            if kind == "integer":
                if "const" in prop:
                    value = prop["const"]
                    for number, accepted in ((float(value), True), (value + 0.5, False), (value + 1, False)):
                        cases.append((f"{event}: {name}={number!r}", dict(base, **{name: number}), accepted))
                else:
                    for number, accepted in ((prop["minimum"], True), (prop["maximum"], True),
                                             (prop["minimum"] - 1, False), (prop["maximum"] + 1, False)):
                        cases.append((f"{event}: boundary {name}={number}", dict(base, **{name: number}), accepted))
        cases.append((f"{event}: unknown field extra", dict(base, extra=True), False))
        if "if" in definition:
            key, condition = next(iter(definition["if"]["properties"].items()))
            conditional = dict(base, **{key: condition["const"]})
            for name in definition["then"]["required"]:
                payload = dict(conditional)
                del payload[name]
                cases.append((f"{event}: {key}={condition['const']} missing {name}", payload, False))
    for payload in (None, [], True, "legacy", {"v": 1, "type": "unknown"}):
        cases.append((f"unknown envelope {payload!r}", payload, False))
    return cases


def manifest():
    workspace = tomllib.loads((GEN.ROOT / "Cargo.toml").read_text())
    lines = ['[package]', 'name = "android_decoder_contract_probe"', 'version = "0.0.0"',
             'edition = "2024"', '[workspace]', '[dependencies]']
    for name in ("serde", "serde_json", "log"):
        version = workspace["workspace"]["dependencies"][name]["version"]
        features = ["std", "derive"] if name == "serde" else ["std"]
        lines.append(f'{name} = {{ version = {json.dumps(version)}, features = {json.dumps(features)} }}')
    return "\n".join(lines) + "\n"


def harness(schema):
    lines = [
        'extern crate self as nerust_input_traits;',
        '#[derive(Debug, Clone, Copy, PartialEq, Eq)]',
        'pub enum AbstractKey { Button1, Button2, Button5, Button6, Start, Select, DpadUp, DpadDown, DpadLeft, DpadRight }',
        'mod error_dialog_controller {',
        '    #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]',
        '    #[serde(transparent)]',
        '    pub(crate) struct DialogRequestId(pub(crate) u64);',
        '}',
        f'#[path = {json.dumps(str(GEN.RUST / "src/bridge_contract.rs"))}] mod bridge_contract;',
        f'#[path = {json.dumps(str(GEN.RUST / "src/bridge_events.rs"))}] mod bridge_events;',
        '#[test] fn schema_decoder_probes() {',
    ]
    for label, payload, accepted in probes(schema):
        raw = json.dumps(json.dumps(payload, separators=(",", ":")))
        lines.append(f'    assert_eq!(bridge_events::decode_frontend_event({raw}).is_some(), '
                     f'{str(accepted).lower()}, "{{}}", {json.dumps(label)});')
    lines.append('}')
    return "\n".join(lines) + "\n"


def main():
    schema = json.loads((GEN.RUST / "bridge_events.v1.json").read_text())
    with tempfile.TemporaryDirectory(prefix="nerust-android-decoder-") as directory:
        root = Path(directory)
        (root / "src").mkdir()
        (root / "Cargo.toml").write_text(manifest())
        (root / "src/lib.rs").write_text(harness(schema))
        env = dict(os.environ, RUSTC_WRAPPER="")
        env.setdefault("CARGO_BUILD_JOBS", "2")
        env["CARGO_TARGET_DIR"] = str(Path(env.get("CARGO_TARGET_DIR", "/tmp/nerust-android-contract-target")).resolve() / "decoder-probes")
        subprocess.run(["cargo", "test", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--lib"],
                       env=env, check=True)
    print(f"Android real decoder schema probes: OK ({len(probes(schema))} cases)")


if __name__ == "__main__":
    main()