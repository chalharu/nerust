#!/usr/bin/env python3
"""Check actual event producers/decoders and both directions of the JNI ABI.

This deliberately supports only the source syntax used by the bridge. New
syntax must teach this checker its meaning instead of silently skipping it.
No Android SDK, JVM, or third-party Python package is needed.
"""
import importlib.util
import json
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("generator", Path(__file__).with_name("generate-contract.py"))
GEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GEN)


class ContractError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise ContractError(message)


def uncomment(text):
    # Preserve strings containing comment-like characters.
    return re.sub(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/',
                  lambda m: m[0] if m[0].startswith('"') else " ", text, flags=re.DOTALL)


def sources():
    src = GEN.RUST / "src"
    return {
        "decoder": (src / "bridge_events.rs").read_text(),
        "generated": (src / "bridge_contract.rs").read_text(),
        "producer": (GEN.KOTLIN / "BridgeContract.kt").read_text(),
        "activity": (GEN.KOTLIN / "MainActivity.kt").read_text(),
        "table": (src / "android/bridge.rs").read_text(),
        "registration": (src / "android.rs").read_text(),
        "callbacks": [(src / f"android/{name}.rs").read_text()
                      for name in ("bridge", "menu", "picker", "settings_bridge")],
    }


KOTLIN_TYPES = {"String": "Ljava/lang/String;", "Int": "I", "Long": "J",
                "Boolean": "Z", "Float": "F", "ByteArray": "[B",
                "IntArray": "[I", "FloatArray": "[F", "Unit": "V"}
RUST_TYPES = {"JString": "Ljava/lang/String;", "jint": "I", "jlong": "J",
              "jboolean": "Z", "jfloat": "F", "JByteArray": "[B", "JIntArray": "[I"}


def kotlin_signature(arguments, result):
    args = []
    for arg in arguments.split(","):
        if not arg.strip():
            continue
        match = re.fullmatch(r"\s*\w+\s*:\s*(\w+)(\?)?\s*", arg)
        require(match and match[1] in KOTLIN_TYPES, f"unsupported Kotlin JNI argument: {arg}")
        require(not match[2] or match[1] in {"String", "ByteArray", "IntArray", "FloatArray"},
                "nullable primitives have a boxed JNI ABI")
        args.append(KOTLIN_TYPES[match[1]])
    require(result.rstrip("?") in KOTLIN_TYPES, f"unsupported Kotlin JNI return: {result}")
    require(not result.endswith("?") or result.rstrip("?") in {"String", "ByteArray", "IntArray", "FloatArray"},
            "nullable primitive return")
    return "(" + "".join(args) + ")" + KOTLIN_TYPES[result.rstrip("?")]


def _registration_entries(registration):
    """Declared native methods with their JNI signatures."""
    entries = re.findall(
        r'jni::NativeMethod::from_raw_parts\(\s*jni_str!\("(\w+)"\),\s*'
        r'jni_str!\("([^"\n]+)"\),\s*([\w:]+)\s+as\s+\*mut\s+c_void\s*,?\s*\)', registration)
    require(entries and len(entries) == registration.count("jni::NativeMethod::from_raw_parts("),
            "unparsed native registration")
    return entries


def _callback_symbols(callbacks):
    """Rust extern callbacks keyed by symbol with JNI signatures."""
    symbols = {}
    for text in callbacks:
        for match in re.finditer(
                r'pub\s+extern\s+"system"\s+fn\s+(Java_\w+)\s*\(([^()]*)\)\s*([^\{]*)\{',
                uncomment(text)):
            symbol, args, result = match.groups()
            require(not result.strip(), f"unsupported JNI callback return: {symbol}")
            args = [arg.strip() for arg in args.split(",") if arg.strip()]
            require(len(args) >= 2 and re.search(r":\s*jni::EnvUnowned<'_>$", args[0])
                    and re.search(r":\s*(?:jni::objects::)?JObject<'_>$", args[1]),
                    f"invalid JNI environment/receiver: {symbol}")
            descriptors = []
            for arg in args[2:]:
                kind = re.fullmatch(r"(?:mut\s+)?\w+:\s*(?:jni::(?:sys|objects)::)?(\w+)(?:<'_>)?", arg)
                require(kind and kind[1] in RUST_TYPES, f"unsupported Rust JNI argument: {arg}")
                descriptors.append(RUST_TYPES[kind[1]])
            require(symbol not in symbols, f"duplicate Rust callback: {symbol}")
            symbols[symbol] = "(" + "".join(descriptors) + ")V"
    return symbols


def _checked_native(entries, symbols):
    """Registered names bound to matching Rust callbacks."""
    native = {}
    for name, signature, pointer in entries:
        symbol = pointer.split("::")[-1]
        require(symbol == "Java_io_github_chalharu_nerust_MainActivity_" + name,
                f"wrong registered function for {name}: {pointer}")
        require(symbols.get(symbol) == signature, f"Rust callback ABI mismatch: {name}")
        require(name not in native, f"duplicate registration: {name}")
        native[name] = signature
    require(set(symbols) == {pointer.split("::")[-1] for _, _, pointer in entries},
            "unregistered Rust MainActivity callback")
    return native


def _kotlin_methods(activity, call_names):
    """Kotlin bridge methods with the external native subset."""
    methods = {}
    kotlin_native = {}
    for match in re.finditer(r'\b(external\s+)?fun\s+(\w+)\s*\(([^()]*)\)\s*(?::\s*([\w?]+))?', activity):
        external, name, args, result = match.groups()
        if external or name in call_names:
            require(name not in methods, f"overloaded bridge method: {name}")
            methods[name] = kotlin_signature(args, result or "Unit")
            if external:
                kotlin_native[name] = methods[name]
    return methods, kotlin_native


def _checked_outbound(calls, methods):
    """Rust bridge table entries bound to Kotlin methods."""
    outbound = {}
    for name, signature in calls:
        require(name not in outbound, f"duplicate bridge method: {name}")
        # finish() is inherited from android.app.Activity, not a local method.
        actual = methods.get(name, "()V" if name == "finish" else None)
        require(signature == actual, f"Rust table/Kotlin ABI mismatch: {name}: {signature} != {actual}")
        outbound[name] = signature
    return outbound


def validate_jni(registration, table, activity, callbacks):
    registration, table, activity = map(uncomment, (registration, table, activity))
    entries = _registration_entries(registration)
    native = _checked_native(entries, _callback_symbols(callbacks))
    calls = re.findall(r'bridge_method!\(\s*\w+\s*,\s*"(\w+)"\s*,\s*"([^"]+)"\s*\)', table)
    require(calls and len(calls) == table.count("bridge_method!("), "unparsed Rust bridge table")
    methods, kotlin_native = _kotlin_methods(activity, {name for name, _ in calls})
    require(kotlin_native == native, f"native registration/Kotlin ABI mismatch: {native} != {kotlin_native}")
    require(len(kotlin_native) == len(re.findall(r'\bexternal\s+fun\b', activity)),
            "unparsed Kotlin native declaration")
    outbound = _checked_outbound(calls, methods)
    require("writeSafFileChunkBytes" not in outbound
            and not re.search(r'\bfun\s+writeSafFileChunkBytes\b', activity),
            "obsolete SAF chunk method")
    return {"native": native, "outbound": outbound}


def _enum_body(decoder):
    """Raw text inside the typed event enum braces."""
    match = re.search(r'enum BridgeEventV1\s*\{', decoder)
    require(match, "missing typed event decoder")
    depth = 0
    for index in range(match.end() - 1, len(decoder)):
        if decoder[index] == "{":
            depth += 1
        elif decoder[index] == "}":
            depth -= 1
            if depth == 0:
                require(decoder[index - 1] == "\n", "missing typed event decoder")
                return decoder[match.end():index - 1]
    raise ContractError("missing typed event decoder")


def validate_decoder(definitions, decoder):
    decoder = uncomment(decoder)
    require(re.search(r'#\[serde\(tag = "type", rename_all = "snake_case"\)\]\s*enum BridgeEventV1', decoder),
            "unsupported decoder enum representation")
    body = _enum_body(decoder)
    variant_pattern = r'(\w+)\s*\{([^{}]*)\}\s*,?'
    variants = re.findall(variant_pattern, body)
    require(not re.sub(variant_pattern, '', body).strip(), "unparsed decoder variant")
    expected = {}
    for variant, body in variants:
        event = re.sub(r'(?<!^)(?=[A-Z])', '_', variant).lower()
        field_pattern = (r'(?:#\[(?:allow\(dead_code\)|serde\(default\))\]\s*)*'
                         r'(\w+)\s*:\s*([\w:]+(?:<\w+>)?)\s*,')
        fields = re.findall(field_pattern, body)
        require(not re.sub(field_pattern, '', body).strip(), "unsupported decoder field/serde rule")
        require(event not in expected and fields, "unparsed event variant")
        props = {"type": ("string", True)}
        for name, kind in fields:
            optional = kind.startswith("Option<")
            if optional:
                kind = kind[7:-1]
            types = {"u32": "integer", "i32": "integer", "String": "string", "bool": "boolean",
                     "crate::error_dialog_controller::DialogRequestId": "integer"}
            require(kind in types, f"unsupported decoder field type: {kind}")
            props[name] = (types[kind], not optional)
        expected[event] = props
    actual = {event: {name: (GEN.field_type(prop), name in definition["required"])
                      for name, prop in definition["properties"].items()}
              for event, definition in definitions.items()}
    require(expected == actual, f"schema/typed decoder fields differ: {actual} != {expected}")
    require(re.search(r'if\s+version\s*!=\s*BRIDGE_EVENT_VERSION\s*\{.*?return None;', decoder, re.DOTALL),
            "decoder must reject unsupported versions")
    require(re.search(r'let mut value: serde_json::Value = serde_json::from_str\(raw\).ok\(\)\?;\s*'
                      r'validate_bridge_event\(&mut value\)\?;\s*'
                      r'let envelope: BridgeEventV1 = serde_json::from_value\(value\).ok\(\)\?;', decoder),
            "decoder must validate the same JSON value before typed dispatch")


def _producer_arg_types(args):
    """Producer parameters mapped to schema kinds."""
    arg_types = {}
    for arg in args.split(","):
        if arg.strip():
            match = re.fullmatch(r'\s*(\w+):\s*(String|Int|Boolean)\s*', arg)
            require(match, "unsupported producer parameter")
            arg_types[match[1]] = {"String": "string", "Int": "integer", "Boolean": "boolean"}[match[2]]
    return arg_types


def _producer_puts(body):
    """Producer field assignments emitted into the JSON object."""
    puts = re.findall(r'\.put\("(\w+)",\s*("[^"\n]*"|[\w.]+)\)', body)
    require(re.sub(r'\.put\("(\w+)",\s*("[^"\n]*"|[\w.]+)\)', '', body).strip() == '',
            "unsupported producer expression")
    return puts


def _check_producer_event(name, puts, arg_types, definitions):
    """Producer fields, types, and envelope against the schema."""
    fields = {}
    for key, value in puts:
        require(key not in fields, "duplicate producer field")
        kind = arg_types.get(value)
        if value == "BRIDGE_EVENT_VERSION":
            kind = "integer"
        elif value.startswith(('"', "MenuAction.")):
            kind = "string"
        require(kind is not None, f"unsupported producer value: {value}")
        fields[key] = kind
    menu = definitions["menu"]
    required = set(menu["required"])
    if name == "controllerEventJson":
        required |= set(menu["then"]["required"])
    require(required <= fields.keys() <= menu["properties"].keys(), "producer fields differ from schema")
    require(all(GEN.field_type(menu["properties"][key]) == kind for key, kind in fields.items()),
            "producer field type differs from schema")
    require(dict(puts).get("v") == "BRIDGE_EVENT_VERSION" and dict(puts).get("type") == '"menu"',
            "producer envelope/version drift")
    require(dict(puts).get("action") == ("action" if name == "menuEventJson" else "MenuAction.CONTROLLER"),
            "producer action drift")


def validate_producer(definitions, producer):
    producer = uncomment(producer)
    builders = re.findall(r'internal fun (\w+)\(([^()]*)\): String\s*=\s*JSONObject\(\)(.*?)\.toString\(\)',
                          producer, re.DOTALL)
    require({name for name, _, _ in builders} == {"menuEventJson", "controllerEventJson"},
            "unparsed event producers")
    for name, args, body in builders:
        _check_producer_event(name, _producer_puts(body), _producer_arg_types(args), definitions)


def verify(schema, source):
    try:
        definitions = GEN.event_definitions(schema)
        generated = GEN.outputs(schema)[GEN.RUST / "src/bridge_contract.rs"]
        require(source["generated"] == generated, "generated structural validation is stale or modified")
        validate_decoder(definitions, source["decoder"])
        validate_producer(definitions, source["producer"])
        require("register_native_methods(class, &methods)" in source["registration"],
                "registration table is not registered")
        return {"jni": validate_jni(source["registration"], source["table"], source["activity"], source["callbacks"])}
    except (AssertionError, KeyError, TypeError, IndexError) as error:
        raise ContractError(f"unsupported bridge schema/source: {error}") from error


if __name__ == "__main__":
    try:
        result = verify(json.loads((GEN.RUST / "bridge_events.v1.json").read_text()), sources())
    except ContractError as error:
        sys.exit(str(error))
    print(f'Android event/JNI structures: OK ({len(result["jni"]["native"])} registered natives)')