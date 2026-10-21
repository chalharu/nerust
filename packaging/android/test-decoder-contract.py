#!/usr/bin/env python3
"""Run schema-derived probes through the real Rust decoder, without an SDK.

The isolated crate avoids building unrelated GUI/GPU backends. Only the input
key enum and request-id wrapper are fixtures; codec and validator are real files.
"""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

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


def _enum_cases(event, name, prop, base):
    """Accepted members plus unknown-marker rejection."""
    if "enum" not in prop:
        return []
    cases = [(f"{event}: {name}={value!r}", dict(base, **{name: value}), True)
             for value in prop["enum"]]
    cases.append((f"{event}: {name}='__unknown__'", dict(base, **{name: "__unknown__"}), False))
    return cases


def _integer_cases(event, name, prop, base):
    """Decimal-form and boundary probes for integers."""
    if GEN.field_type(prop) != "integer":
        return []
    if "const" in prop:
        value = prop["const"]
        return [(f"{event}: {name}={number!r}", dict(base, **{name: number}), accepted)
                for number, accepted in ((float(value), True), (value + 0.5, False), (value + 1, False))]
    bounds = ((prop["minimum"], True), (prop["maximum"], True),
              (prop["minimum"] - 1, False), (prop["maximum"] + 1, False))
    return [(f"{event}: boundary {name}={number}", dict(base, **{name: number}), accepted)
            for number, accepted in bounds]


def _prop_cases(event, name, prop, base, required):
    """Missing, mistyped, enum, and bounds probes for one property."""
    kind = GEN.field_type(prop)
    wrong = {"integer": "not an integer", "boolean": "not a boolean", "string": 7}[kind]
    missing = dict(base)
    del missing[name]
    cases = [(f"{event}: missing {'required' if required else 'optional'} {name}", missing, not required)]
    for value in (None, {}, [], wrong):
        cases.append((f"{event}: {name}={value!r}", dict(base, **{name: value}), False))
    cases.append((f"{event}: wrong type {name}", dict(base, **{name: wrong}), False))
    cases += _enum_cases(event, name, prop, base)
    return cases + _integer_cases(event, name, prop, base)


def _conditional_cases(event, definition, base):
    """Conditional-required probes for a gated event."""
    if "if" not in definition:
        return []
    key, condition = next(iter(definition["if"]["properties"].items()))
    conditional = dict(base, **{key: condition["const"]})
    cases = []
    for name in definition["then"]["required"]:
        payload = dict(conditional)
        del payload[name]
        cases.append((f"{event}: {key}={condition['const']} missing {name}", payload, False))
    return cases


def _unknown_envelope_cases():
    """Non-object and unknown-type envelopes."""
    return [(f"unknown envelope {payload!r}", payload, False)
            for payload in (None, [], True, "legacy", {"v": 1, "type": "unknown"})]


def probes(schema):
    cases = []
    for event, definition in GEN.event_definitions(schema).items():
        props = definition["properties"]
        base = {name: sample(prop) for name, prop in props.items()}
        cases.append((f"{event}: valid", base, True))
        for name, prop in props.items():
            cases += _prop_cases(event, name, prop, base, name in definition["required"])
        cases.append((f"{event}: unknown field extra", dict(base, extra=True), False))
        cases += _conditional_cases(event, definition, base)
    return cases + _unknown_envelope_cases()


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


def prepare_probe(root, schema):
    (root / "src").mkdir()
    (root / "Cargo.toml").write_text(manifest())
    (root / "src/lib.rs").write_text(harness(schema))
    # Keep transitive versions aligned with the workspace instead of resolving
    # a second, newer dependency graph in this temporary project.
    (root / "Cargo.lock").write_bytes((GEN.ROOT / "Cargo.lock").read_bytes())


def main():
    schema = json.loads((GEN.RUST / "bridge_events.v1.json").read_text())
    with tempfile.TemporaryDirectory(prefix="nerust-android-decoder-") as directory:
        root = Path(directory)
        prepare_probe(root, schema)
        env = dict(os.environ, RUSTC_WRAPPER="")
        env.setdefault("CARGO_BUILD_JOBS", "2")
        # Share the build cache without hardcoding a public directory.
        cache = Path(env.get("CARGO_TARGET_DIR",
                             Path(tempfile.gettempdir()) / "nerust-android-contract-target"))
        env["CARGO_TARGET_DIR"] = str(cache.resolve() / "decoder-probes")
        # A cold CI cache must be allowed to fetch locked dependencies. Explicit
        # caller policy (e.g. CARGO_NET_OFFLINE) is still inherited via env.
        subprocess.run(["cargo", "test", "--manifest-path", str(root / "Cargo.toml"), "--lib"],
                       env=env, check=True)
    print(f"Android real decoder schema probes: OK ({len(probes(schema))} cases)")


if __name__ == "__main__":
    main()