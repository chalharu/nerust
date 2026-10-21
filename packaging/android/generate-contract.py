#!/usr/bin/env python3
"""Generate/check wire constants and Rust validation from bridge_events.v1.json.

Run with --write after changing the schema; CI uses --check. Generated files
are checked in so neither cargo nor Gradle needs Python during a normal build.
"""
import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUST = ROOT / "gui/frontends/android"
KOTLIN = ROOT / "packaging/android/app/src/main/kotlin/io/github/chalharu/nerust"


def event_definitions(schema):
    """Supported schema subset, fail closed when a new rule needs codegen."""
    require(schema.get("$schema") == "http://json-schema.org/draft-07/schema#")
    require(set(schema) <= {"$schema", "title", "description", "type", "oneOf", "definitions",
                            "x-settings-schema-version", "x-saf-chunk-bytes",
                            "x-menu-action-table", "x-kotlin-constants"})
    require(schema["type"] == "object")
    result = {}
    require(type(schema["oneOf"]) is list and schema["oneOf"])
    require(len(schema["oneOf"]) == len(schema["definitions"]))
    for ref in schema["oneOf"]:
        require(set(ref) == {"$ref"} and ref["$ref"].startswith("#/definitions/"))
        definition = schema["definitions"][ref["$ref"].removeprefix("#/definitions/")]
        require(set(definition) <= {"type", "required", "properties", "additionalProperties", "if", "then"})
        require(definition["type"] == "object" and definition["additionalProperties"] is False)
        props, required = definition["properties"], definition["required"]
        require(type(props) is dict and type(required) is list)
        require(all(type(name) is str for name in required))
        require(len(required) == len(set(required)) and set(required) <= props.keys())
        require({"v", "type"} <= set(required))
        for prop in props.values():
            require(set(prop) <= {"type", "const", "enum", "minimum", "maximum"})
            kind = field_type(prop)
            require(kind in {"integer", "string", "boolean"})
            if kind == "integer" and "const" not in prop:
                require(type(prop.get("minimum")) is int and type(prop.get("maximum")) is int)
                require(-(2**63) <= prop["minimum"] <= prop["maximum"] < 2**63)
            if "enum" in prop:
                require(kind == "string" and type(prop["enum"]) is list and prop["enum"]
                    and all(type(v) is str for v in prop["enum"]))
                require(len(prop["enum"]) == len(set(prop["enum"])))
            if "const" in prop:
                require(type(prop["const"]) in {int, str, bool})
                require(field_type({"const": prop["const"]}) == kind)
                require(not {"enum", "minimum", "maximum"} & prop.keys())
            require(not ({"minimum", "maximum"} & prop.keys()) or kind == "integer")
        if "if" in definition or "then" in definition:
            require("if" in definition and "then" in definition)
            condition, consequence = definition["if"], definition["then"]
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
        event = props["type"]["const"]
        require(set(props["type"]) <= {"type", "const"})
        require(field_type(props["type"]) == "string")
        require(set(props["v"]) <= {"type", "const"})
        require(field_type(props["v"]) == "integer" and props["v"]["const"] == 1)
        require(type(event) is str and event not in result)
        require(type(props["v"]["const"]) is int)
        result[event] = definition
    require(result and len({d["properties"]["v"]["const"] for d in result.values()}) == 1)
    return result


def field_type(prop):
    require(type(prop) is dict)
    if "type" in prop:
        require(type(prop["type"]) is str and prop["type"] in {"integer", "string", "boolean"})
        return prop["type"]
    require(type(prop.get("const")) in {int, str, bool})
    return {int: "integer", str: "string", bool: "boolean"}[type(prop["const"])]


def require(condition):
    # Unlike Python assert, contract checks must survive python -O.
    if not condition:
        raise AssertionError("unsupported or inconsistent bridge schema")


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
    return f'value.is_{ {"string": "string", "boolean": "boolean"}[kind]}()'


def rust_validator(definitions):
    lines = ["", "/// Schema-derived validation before typed dispatch. Missing is not JSON null.",
             "pub(crate) fn validate_bridge_event(value: &mut serde_json::Value) -> Option<()> {",
             "    let object = value.as_object_mut()?;",
             '    match object.get("type")?.as_str()? {']
    for event, definition in definitions.items():
        props = definition["properties"]
        lines += [f'        {json.dumps(event)} => {{', "            for (name, value) in object.iter_mut() {",
                  "                let valid = match name.as_str() {"]
        for name, prop in props.items():
            if field_type(prop) != "integer" and "enum" not in prop:
                lines += [f'                    {json.dumps(name)} => {rust_predicate(prop)},']
                continue
            lines += [f'                    {json.dumps(name)} => {{']
            if field_type(prop) == "integer":
                lines += ["                        normalize_bridge_integer(value)?;"]
            lines += [f'                        {line}' for line in rust_predicate(prop).splitlines()]
            lines += ["                    }"]
        lines += ["                    _ => false,", "                };", "                if !valid {",
                  "                    return None;", "                }", "            }"]
        required = ", ".join(json.dumps(name) for name in definition["required"])
        lines += [f'            for name in [{required}] {{',
                  "                object.get(name)?;", "            }"]
        if "if" in definition:
            key, condition = next(iter(definition["if"]["properties"].items()))
            lines += [f'            if object.get({json.dumps(key)})?.as_str() == Some({json.dumps(condition["const"])}) {{']
            for name in definition["then"]["required"]:
                lines += [f'                object.get({json.dumps(name)})?;']
            lines += ["            }"]
        lines += ["            Some(())", "        }"]
    lines += ["        _ => None,", "    }", "}", "",
              "// JSON Schema integers include integral decimal/exponent numbers (e.g. 1.0).",
              "fn normalize_bridge_integer(value: &mut serde_json::Value) -> Option<()> {",
              "    if value.as_i64().is_some() {", "        return Some(());", "    }",
              "    // Never round an existing unsigned integer through f64.",
              "    if value.as_u64().is_some() {", "        return None;", "    }",
              "    let number = value.as_f64()?;",
              "    if number.fract() != 0.0 || number < i64::MIN as f64 || number >= -(i64::MIN as f64) {",
              "        return None;", "    }", "    *value = serde_json::Value::from(number as i64);",
              "    Some(())", "}"]
    return lines


def outputs(schema):
    definitions = event_definitions(schema)
    menu = schema["definitions"]["menuEvent"]["properties"]
    version = menu["v"]["const"]
    require(version == schema["definitions"]["errorDialogDismissedEvent"]["properties"]["v"]["const"])
    settings = schema["x-settings-schema-version"]
    chunk = schema["x-saf-chunk-bytes"]
    require(all(type(number) is int and 0 < number < 2**31 for number in (version, settings, chunk)))
    actions, keys = menu["action"]["enum"], menu["key"]["enum"]
    for names in (actions, keys):
        require(len(names) == len(set(names)))
        require(all(name.isascii() and name.replace("_", "").isalnum() for name in names))
        require(len({name.upper() for name in names}) == len(names))
    rs = ["// Generated by packaging/android/generate-contract.py; do not edit."]
    for name, kind, value in [("BRIDGE_EVENT_VERSION", "u32", version),
                              ("SETTINGS_SCHEMA_VERSION", "u32", settings),
                              ("SAF_CHUNK_BYTES", "usize", chunk)]:
        rs.append(f"pub(crate) const {name}: {kind} = {value};")
    for action in actions:
        rs.append(f'pub(crate) const ACTION_{action.upper()}: &str = "{action}";')
    rs += rust_validator(definitions)
    kt = ["// Generated by packaging/android/generate-contract.py; do not edit.",
          "package io.github.chalharu.nerust", "",
          f"internal const val BRIDGE_EVENT_VERSION = {version}",
          f"internal const val SETTINGS_SCHEMA_VERSION = {settings}",
          f"internal const val SAF_CHUNK_BYTES = {chunk}"]
    for obj, names in [("MenuAction", actions), ("ControllerKey", keys)]:
        kt += ["", f"internal object {obj} {{"]
        kt += [f'    const val {name.upper()} = "{name}"' for name in names]
        kt += ["", "    val ALL: Set<String> = setOf("]
        kt += [f"        {name.upper()}," for name in names]
        kt += ["    )", "}"]
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