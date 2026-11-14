#!/usr/bin/env python3
"""Generate/check wire constants and Rust validation from bridge_events.v1.json.

Run with --write after changing the schema; CI uses --check. Generated files
are checked in so neither cargo nor Gradle needs Python during a normal build.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bridge_schema import require

ROOT = Path(__file__).resolve().parents[2]
RUST = ROOT / "gui/frontends/android"
KOTLIN = ROOT / "packaging/android/app/src/main/kotlin/io/github/chalharu/nerust"


def _require_top_level_schema(schema):
    """Top-level schema shape this codegen understands."""
    require(schema.get("$schema") == "http://json-schema.org/draft-07/schema#")
    require(set(schema) <= {"$schema", "title", "description", "type", "oneOf", "definitions",
                            "x-settings-schema-version", "x-saf-chunk-bytes", "x-saf-summary-bytes",
                            "x-menu-action-table", "x-kotlin-constants"})
    require(schema["type"] == "object")
    require(type(schema["oneOf"]) is list and schema["oneOf"])
    require(len(schema["oneOf"]) == len(schema["definitions"]))


def _definition_props(definition):
    """Single event definition shape with its properties and required names."""
    require(set(definition) <= {"type", "required", "properties", "additionalProperties", "if", "then", "allOf"})
    require(definition["type"] == "object" and definition["additionalProperties"] is False)
    props, required = definition["properties"], definition["required"]
    require(type(props) is dict and type(required) is list)
    require(all(type(name) is str for name in required))
    require(len(required) == len(set(required)) and set(required) <= props.keys())
    require({"v", "type"} <= set(required))
    return props, required


def _require_integer_bounds(prop):
    """Integer range this codegen can emit."""
    require(type(prop.get("minimum")) is int and type(prop.get("maximum")) is int)
    require(-(2**63) <= prop["minimum"] <= prop["maximum"] < 2**63)


def _require_enum_values(prop, kind):
    """Closed string set this codegen can emit."""
    require(kind == "string" and type(prop["enum"]) is list and prop["enum"]
        and all(type(value) is str for value in prop["enum"]))
    require(len(prop["enum"]) == len(set(prop["enum"])))


def _require_const_value(prop, kind):
    """Pinned value consistent with its declared kind."""
    require(type(prop["const"]) in {int, str, bool})
    require(field_type({"const": prop["const"]}) == kind)
    require(not {"enum", "minimum", "maximum"} & prop.keys())


def _require_prop(prop):
    """Single property rule this codegen supports."""
    require(set(prop) <= {"type", "const", "enum", "minimum", "maximum"})
    kind = field_type(prop)
    require(kind in {"integer", "string", "boolean", "number"})
    if kind == "integer" and "const" not in prop:
        _require_integer_bounds(prop)
    if kind == "number":
        require(prop.get("minimum") is not None and prop.get("maximum") is not None)
        require(type(prop["minimum"]) in {int, float} and type(prop["maximum"]) in {int, float})
    if "enum" in prop:
        _require_enum_values(prop, kind)
    if "const" in prop:
        _require_const_value(prop, kind)
    require(not ({"minimum", "maximum"} & prop.keys()) or kind in {"integer", "number"})


def _require_gate(gate, props):
    """One enum value requiring extra fields."""
    require(set(gate) == {"if", "then"})
    condition, consequence = gate["if"], gate["then"]
    require(set(condition) == {"properties", "required"})
    require(set(consequence) == {"required"})
    require(len(condition["properties"]) == 1)
    require(set(condition["required"]) == condition["properties"].keys())
    for key, value in condition["properties"].items():
        require(key in props and set(value) == {"const"} and type(value["const"]) is str)
        require(value["const"] in props[key]["enum"])
    require(type(consequence["required"]) is list
            and len(consequence["required"]) == len(set(consequence["required"]))
            and set(consequence["required"]) <= props.keys())


def _require_conditional(definition, props):
    """Optional gates requiring extra fields for enum values."""
    gates = definition.get("allOf", [])
    require(type(gates) is list)
    for gate in gates:
        _require_gate(gate, props)


def _envelope_event(props, seen):
    """Shared type/version marker, returning the event name."""
    event = props["type"]["const"]
    require(set(props["type"]) <= {"type", "const"})
    require(field_type(props["type"]) == "string")
    require(set(props["v"]) <= {"type", "const"})
    require(field_type(props["v"]) == "integer" and props["v"]["const"] == 1)
    require(type(event) is str and event not in seen)
    require(type(props["v"]["const"]) is int)
    return event


def event_definitions(schema):
    """Supported schema subset, fail closed when a new rule needs codegen."""
    _require_top_level_schema(schema)
    result = {}
    for ref in schema["oneOf"]:
        require(set(ref) == {"$ref"} and ref["$ref"].startswith("#/definitions/"))
        definition = schema["definitions"][ref["$ref"].removeprefix("#/definitions/")]
        props, _ = _definition_props(definition)
        for prop in props.values():
            _require_prop(prop)
        _require_conditional(definition, props)
        event = _envelope_event(props, result)
        result[event] = definition
    require(result and len({d["properties"]["v"]["const"] for d in result.values()}) == 1)
    return result


def field_type(prop):
    require(type(prop) is dict)
    if "type" in prop:
        require(type(prop["type"]) is str and prop["type"] in {"integer", "string", "boolean", "number"})
        return prop["type"]
    require(type(prop.get("const")) in {int, str, bool})
    return {int: "integer", str: "string", bool: "boolean"}[type(prop["const"])]


def format_rust(content):
    """Use the repository config with stdin (no filename banner in stdout)."""
    return subprocess.run(
        ["rustfmt", "--config-path", str(ROOT / "rustfmt.toml"), "--emit", "stdout"],
        input=content, text=True, capture_output=True, check=True,
    ).stdout


def rust_predicate(prop):
    kind = field_type(prop)
    if "const" in prop:
        accessor = {"integer": "as_i64", "string": "as_str", "boolean": "as_bool"}[kind]
        return f'value.{accessor}() == Some({json.dumps(prop["const"])})'
    if "enum" in prop:
        choices = "\n            | ".join(json.dumps(value) for value in prop["enum"])
        return f'matches!(\n    value.as_str(),\n    Some(\n        {choices}\n    )\n)'
    if kind == "integer":
        return (f'value\n    .as_i64()\n    .is_some_and(|number| '
                f'({prop["minimum"]}..={prop["maximum"]}).contains(&number))')
    if kind == "number":
        return (f'value\n    .as_f64()\n    .is_some_and(|number| '
                f'({prop["minimum"]}..={prop["maximum"]}).contains(&number))')
    return f'value.is_{ {"string": "string", "boolean": "boolean"}[kind]}()'


def outputs(schema):
    definitions = event_definitions(schema)
    menu = schema["definitions"]["menuEvent"]["properties"]
    version = menu["v"]["const"]
    require(version == schema["definitions"]["errorDialogDismissedEvent"]["properties"]["v"]["const"])
    settings = schema["x-settings-schema-version"]
    chunk = schema["x-saf-chunk-bytes"]
    summary = schema["x-saf-summary-bytes"]
    require(all(type(number) is int and 0 < number < 2**31 for number in (version, settings, chunk, summary)))
    actions, keys = menu["action"]["enum"], menu["key"]["enum"]
    # Motion axes ride as optional per-axis number properties (one batched
    # event per motion callback); the property names are the wire ids.
    axes = ["left_x", "left_y", "right_x", "right_y", "trigger_l", "trigger_r"]
    for name in axes:
        prop = menu[name]
        require(field_type(prop) == "number")
        require(prop["minimum"] == -1.0 and prop["maximum"] == 1.0)
    for names in (actions, keys, axes):
        require(len(names) == len(set(names)))
        require(all(name.isascii() and name.replace("_", "").isalnum() for name in names))
        require(len({name.upper() for name in names}) == len(names))
    rs = ["// Generated by packaging/android/generate-contract.py; do not edit."]
    for name, kind, value in [("BRIDGE_EVENT_VERSION", "u32", version),
                              ("SETTINGS_SCHEMA_VERSION", "u32", settings),
                              ("SAF_CHUNK_BYTES", "usize", chunk),
                              ("SAF_SUMMARY_BYTES", "usize", summary)]:
        # SAF_SUMMARY_BYTES is live on the Android target (saf.rs sidecar
        # reads) but host test builds have no live caller, so the pin would
        # trip deny-warnings host clippy. The other pins are read on the host
        # too (bridge_events validation/tests), so they stay bare: an unused
        # pin must warn.
        if name == "SAF_SUMMARY_BYTES":
            rs.append(f"#[allow(dead_code)]\npub(crate) const {name}: {kind} = {value};")
        else:
            rs.append(f"pub(crate) const {name}: {kind} = {value};")
    for action in actions:
        rs.append(f'pub(crate) const ACTION_{action.upper()}: &str = "{action}";')
    # Required fields per menu action, from the schema `allOf` gates.
    # Dispatch checks presence through this table before decoding, so a
    # gate the decoder ignores fails the regeneration check instead of
    # silently widening. Sorted for a stable diff.
    gates = []
    for gate in schema["definitions"]["menuEvent"].get("allOf", []):
        action = gate["if"]["properties"]["action"]["const"]
        required = sorted(gate["then"]["required"])
        gates.append((action, required))
    gates.sort()
    rs.append("")
    rs.append("/// Required fields per menu action, from the schema gates.")
    rs.append("/// Dispatch rejects events missing any listed field first.")
    rs.append("pub(crate) const MENU_REQUIRED_FIELDS: &[(&str, &[&str])] = &[")
    for action, required in gates:
        fields = ", ".join(json.dumps(name) for name in required)
        rs.append(f'    ({json.dumps(action)}, &[{fields}]),')
    rs.append("];")
    # Inclusive bounds for integral bridge fields. The flexible int
    # decoders accept integral floats, then dispatch range-checks through
    # this table; a schema bump the decoder ignores fails regeneration.
    bounds = []
    for event in ("menuEvent", "errorDialogDismissedEvent"):
        for name, prop in schema["definitions"][event]["properties"].items():
            if field_type(prop) == "integer" and "minimum" in prop:
                bounds.append((name, prop["minimum"], prop["maximum"]))
    bounds.sort()
    rs.append("")
    rs.append("/// Inclusive bounds for integral bridge fields, from the schema.")
    rs.append("pub(crate) const INTEGER_FIELD_BOUNDS: &[(&str, i64, i64)] = &[")
    for name, minimum, maximum in bounds:
        rs.append(f'    ({json.dumps(name)}, {minimum}, {maximum}),')
    rs.append("];")
    # Analog range, shared by every motion axis property. Dispatch checks
    # axis values against this instead of a hardcoded literal.
    value_prop = menu["left_x"]
    rs.append("")
    rs.append("/// Inclusive analog axis range, from the schema.")
    rs.append(
        f'pub(crate) const ANALOG_VALUE_BOUNDS: (f32, f32) = '
        f'({float(value_prop["minimum"])}, {float(value_prop["maximum"])});'
    )
    kt = ["// Generated by packaging/android/generate-contract.py; do not edit.",
          "package io.github.chalharu.nerust", "",
          f"internal const val BRIDGE_EVENT_VERSION = {version}",
          f"internal const val SETTINGS_SCHEMA_VERSION = {settings}",
          f"internal const val SAF_CHUNK_BYTES = {chunk}",
          f"internal const val SAF_SUMMARY_BYTES = {summary}"]
    for obj, names in [("MenuAction", actions), ("ControllerKey", keys), ("ControllerAxis", axes)]:
        kt += ["", f"internal object {obj} {{"]
        kt += [f'    const val {name.upper()} = "{name}"' for name in names]
        # No ALL set: grep showed zero references, and an unused set would
        # only drift from the schema tables above. Re-add if a caller needs
        # enumeration rather than single constants.
        kt += ["}"]
    return {RUST / "src/bridge_contract.rs": format_rust("\n".join(rs) + "\n"),
            KOTLIN / "BridgeWire.kt": "\n".join(kt) + "\n"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    args = parser.parse_args()
    schema = json.loads((RUST / "bridge_events.v1.json").read_text())
    for path, content in outputs(schema).items():
        if args.write:
            path.write_text(content)
        elif not path.exists() or path.read_text() != content:
            parser.exit(1, f"stale generated contract: {path.relative_to(ROOT)}; run generate-contract.py --write\n")
    print("Android generated wire contract: OK")


if __name__ == "__main__":
    main()