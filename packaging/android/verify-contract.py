#!/usr/bin/env python3
"""Check actual event producers/decoders and both directions of the JNI ABI.

This deliberately supports only the source syntax used by the bridge. New
syntax must teach this checker its meaning instead of silently skipping it.
No Android SDK, JVM, or third-party Python package is needed.
"""
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bridge_schema import (
    ContractError,
    gates_by_action,
    load_sibling,
    require,
    uncomment,
)

GEN = load_sibling("generator", "generate-contract.py")


#: Shared rejection for a decoder without the typed event enum.
MISSING_DECODER = "missing typed event decoder"


def sources():
    src = GEN.RUST / "src"
    main = (GEN.KOTLIN / "MainActivity.kt").read_text()
    settings = (GEN.KOTLIN / "SettingsActivity.kt").read_text()
    activity = main + "\n" + settings
    callsites = (activity + "\n"
                 + (GEN.KOTLIN / "ChromeManager.kt").read_text() + "\n"
                 + (GEN.KOTLIN / "BridgeContract.kt").read_text())
    return {
        "decoder": (src / "bridge_events.rs").read_text(),
        "generated": (src / "bridge_contract.rs").read_text(),
        "producer": (GEN.KOTLIN / "BridgeContract.kt").read_text(),
        "activity": activity,
        "callsites": callsites,
        "table": (src / "android/bridge.rs").read_text(),
        "registration": (src / "android.rs").read_text(),
        "callbacks": [(src / f"android/{name}.rs").read_text()
                      for name in ("bridge", "menu", "picker", "settings_bridge")],
    }


_JAVA_STRING = "Ljava/lang/String;"
KOTLIN_TYPES = {"String": _JAVA_STRING, "Int": "I", "Long": "J",
                "Boolean": "Z", "Float": "F", "ByteArray": "[B",
                "IntArray": "[I", "FloatArray": "[F", "Unit": "V"}
RUST_TYPES = {"JString": _JAVA_STRING, "jstring": _JAVA_STRING,
              "jint": "I", "jlong": "J",
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
    """Declared native methods with their JNI signatures, keyed by host class.

    Entries after a `find_class` belong to that class; entries before any
    `find_class` (as in unit-test fixtures) default to MainActivity.
    """
    classes = [
        (match.start(), match[1].replace("/", "."))
        for match in re.finditer(r'find_class\(jni_str!\("([\w/]+)"\)\)', registration)
    ]
    entries = []
    for match in re.finditer(
            r'jni::NativeMethod::from_raw_parts\(\s*jni_str!\("(\w+)"\),\s*'
            r'jni_str!\("([^"\n]+)"\),\s*([\w:]+)\s+as\s+\*mut\s+c_void\s*,?\s*\)', registration):
        owner = "io.github.chalharu.nerust.MainActivity"
        for start, class_name in classes:
            if start < match.start():
                owner = class_name
        entries.append((owner, match[1], match[2], match[3]))
    require(entries and len(entries) == registration.count("jni::NativeMethod::from_raw_parts("),
            "unparsed native registration")
    return entries


def _rust_arg_descriptor(arg):
    """Single Rust JNI argument to its descriptor letter."""
    kind = re.fullmatch(r"(?:mut\s+)?\w+:\s*(?:jni::(?:sys|objects)::)?(\w+)(?:<'_>)?", arg)
    require(kind and kind[1] in RUST_TYPES, f"unsupported Rust JNI argument: {arg}")
    return RUST_TYPES[kind[1]]


def _callback_signature(symbol, args_text, result_text):
    """One Rust extern callback to its JNI signature; env/receiver are fixed."""
    args = [arg.strip() for arg in args_text.split(",") if arg.strip()]
    require(len(args) >= 2 and re.search(r":\s*jni::EnvUnowned<'_>$", args[0])
            and re.search(r":\s*(?:jni::objects::)?JObject<'_>$", args[1]),
            f"invalid JNI environment/receiver: {symbol}")
    result = result_text.strip()
    if not result:
        descriptor = "V"
    else:
        # Callbacks stay one-way; only native queries return values, from
        # either host activity (dialog-era MainActivity or SettingsActivity).
        require(symbol.startswith((
            "Java_io_github_chalharu_nerust_MainActivity_native",
            "Java_io_github_chalharu_nerust_SettingsActivity_native",
        )), f"only native queries may return values: {symbol}")
        match = re.fullmatch(r"->\s*jni::sys::(\w+)", result)
        require(match and match[1] in RUST_TYPES,
                f"unsupported JNI query return: {symbol}: {result}")
        descriptor = RUST_TYPES[match[1]]
    return "(" + "".join(_rust_arg_descriptor(arg) for arg in args[2:]) + ")" + descriptor


def _callback_symbols(callbacks):
    """Rust extern callbacks keyed by symbol with JNI signatures."""
    symbols = {}
    for text in callbacks:
        for match in re.finditer(
                r'pub\s+extern\s+"system"\s+fn\s+(Java_\w+)\s*\(([^()]*)\)([^\{]*)\{',
                uncomment(text)):
            symbol, args, result = match.groups()
            require(symbol not in symbols, f"duplicate Rust callback: {symbol}")
            symbols[symbol] = _callback_signature(symbol, args, result)
    return symbols


def _checked_native(entries, symbols):
    """Registered names bound to matching Rust callbacks, per host class.

    Names stay unique across host activities: a second host binding the same
    name would make the Kotlin ABI ambiguous.
    """
    native = {}
    for owner, name, signature, pointer in entries:
        symbol = pointer.split("::")[-1]
        require(symbol == "Java_" + owner.replace(".", "_") + "_" + name,
                f"wrong registered function for {owner}#{name}: {pointer}")
        require(symbols.get(symbol) == signature, f"Rust callback ABI mismatch: {owner}#{name}")
        require(name not in native, f"duplicate registration across host activities: {name}")
        native[name] = signature
    require(set(symbols) == {pointer.split("::")[-1] for _, _, _, pointer in entries},
            "unregistered Rust activity callback")
    return native


def _kotlin_methods(activity, call_names):
    """Kotlin bridge methods with the external native subset.

    The activity source concatenates MainActivity and SettingsActivity; extern
    names must stay unique across both so registration stays unambiguous.
    """
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
    require(kotlin_native == native,
            f"native registration/Kotlin ABI mismatch: {native} != {kotlin_native}")
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
    require(match, MISSING_DECODER)
    depth = 0
    for index in range(match.end() - 1, len(decoder)):
        if decoder[index] == "{":
            depth += 1
        elif decoder[index] == "}":
            depth -= 1
            if depth == 0:
                require(decoder[index - 1] == "\n", MISSING_DECODER)
                return decoder[match.end():index - 1]
    raise ContractError(MISSING_DECODER)


def _variant_gaps(body):
    """Decoder body split into gaps and variant pairs; commas stay in gaps."""
    gaps = []
    variants = []
    pos = 0
    for match in re.finditer(r'(\w+)\s*\{([^{}]*)\}', body):
        gaps.append(body[pos:match.start()])
        variants.append(match.groups())
        pos = match.end()
    gaps.append(body[pos:])
    return gaps, variants


def _gap_commas_ok(gap, allow_comma):
    """Gap holds only whitespace plus at most one separator comma."""
    if gap.replace(",", "").strip():
        return False
    return gap.count(",") <= (1 if allow_comma else 0)


def validate_decoder(definitions, decoder):
    decoder = uncomment(decoder)
    require(re.search(r'#\[serde\(tag = "type", rename_all = "snake_case"\)\]\s*enum BridgeEventV1', decoder),
            "unsupported decoder enum representation")
    body = _enum_body(decoder)
    gaps, variants = _variant_gaps(body)
    require(_gap_commas_ok(gaps[0], False), "unparsed decoder variant")
    require(all(_gap_commas_ok(gap, True) for gap in gaps[1:]), "unparsed decoder variant")
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
            types = {"u32": "integer", "i32": "integer", "i64": "integer",
                     "u64": "integer", "String": "string", "bool": "boolean",
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


def _producer_puts(body):
    """Producer field assignments emitted into the JSON object."""
    puts = re.findall(r'\.put\("(\w+)", ([^()\n]*)\)', body)
    rest = re.sub(r'\.put\("(\w+)", ([^()\n]*)\)', '', body).strip()
    require(rest == '' or rest == '.apply(extras)', "unsupported producer expression")
    return puts


def _balanced(text, start, brackets):
    """Span from the opening bracket at `start` through its match."""
    opening, closing = brackets
    require(text[start] == opening, f"block must open with {opening}")
    depth = 0
    in_string = None
    index = start
    while index < len(text):
        char = text[index]
        if in_string:
            if char == in_string and text[index - 1] != "\\":
                in_string = None
        elif char in ('"', "'"):
            in_string = char
        elif char == opening:
            depth += 1
        elif char == closing:
            depth -= 1
            if depth == 0:
                return text[start:index + 1]
        index += 1
    raise ContractError("unbalanced block")


def _balanced_call(text, start):
    """Call text from the opening paren at `start` through its match."""
    return _balanced(text, start, "()")


def _balanced_braces(text, start):
    """Brace block from the opening brace at `start` through its match."""
    return _balanced(text, start, "{}")


def _menu_event_calls(kotlin):
    """Every `menuEvent(...)` call text across the Kotlin sources, trailing
    extras lambdas included."""
    calls = []
    for match in re.finditer(r'menuEvent\s*\(', kotlin):
        paren = match.end() - 1
        call = _balanced_call(kotlin, paren)
        rest = kotlin[paren + len(call):]
        gap = re.match(r'\s*', rest).group(0)
        if kotlin[paren + len(call) + len(gap):paren + len(call) + len(gap) + 1] == "{":
            call += gap + _balanced_braces(kotlin, paren + len(call) + len(gap))
        calls.append(call)
    require(calls, "no menuEvent producers found")
    return calls


def validate_producer(definitions, producer, kotlin):
    producer = uncomment(producer)
    match = re.search(r'internal fun menuEvent\s*\(', producer)
    require(match, "unparsed event producers")
    call = _balanced_call(producer, match.end() - 1)
    args = call[1:-1]
    after = producer[match.end() - 1 + len(call):]
    body_match = re.match(r'\s*:\s*String\s*=\s*JSONObject\(\)(.*?)\.toString\(\)', after, re.DOTALL)
    require(body_match, "unparsed event producers")
    body = body_match.group(1)
    require(re.fullmatch(r'\s*action:\s*String\s*,\s*extras:\s*JSONObject\.\(\)\s*->\s*Unit\s*=\s*\{\}\s*',
                         args),
            "unified producer must take (action, extras)")
    puts = _producer_puts(body)
    require(dict(puts).get("v") == "BRIDGE_EVENT_VERSION"
            and dict(puts).get("type") == '"menu"'
            and dict(puts).get("action") == "action",
            "producer envelope/version drift")
    require(re.search(r'\.apply\(extras\)\s*$', body.strip()), "producer must apply extras last")
    menu = definitions["menu"]
    code = uncomment(kotlin)
    gated = gates_by_action(menu)
    gated_fields = set().union(*(required for required, _ in gated.values())) if gated else set()
    for call in _menu_event_calls(code):
        pairs = re.findall(r'(?<![\w.])put\("(\w+)", ([^()\n]*)\)', call)
        for key, value in pairs:
            require(re.fullmatch(r'"[^"\n]*"|[\w.]+', value) is not None,
                    f"unsupported extras value: {value}")
        puts_in_call = {key for key, _ in pairs}
        action = next(
            (name for name in list(gated) + ["CREATE_SLOT"]
             if f"MenuAction.{name}" in call),
            None,
        )
        if action is None:
            require(not (puts_in_call & gated_fields),
                    "plain menuEvent must not carry gated fields")
            continue
        if action == "CREATE_SLOT":
            # Slot creation takes no payload by design (the schema has no
            # gate const for create_slot); fail loud on any extras.
            require(not puts_in_call, "create_slot takes no extras")
            continue
        required, forbidden = gated[action]
        require(set(required) <= puts_in_call,
                f"{action.lower()} call misses gated fields")
        require(not (puts_in_call & set(forbidden)),
                f"{action.lower()} call carries foreign gated fields")
    require("menuEventJson" not in code and "touchGateEventJson" not in code
            and "controllerEventJson" not in code, "obsolete typed builders remain")


def verify(schema, source):
    try:
        definitions = GEN.event_definitions(schema)
        generated = GEN.outputs(schema)[GEN.RUST / "src/bridge_contract.rs"]
        require(source["generated"] == generated, "generated structural validation is stale or modified")
        validate_decoder(definitions, source["decoder"])
        validate_producer(definitions, source["producer"], source["callsites"])
        require("register_native_methods(class, &methods)" in source["registration"],
                "registration table is not registered")
        require("register_native_methods(settings_class, &settings_methods)" in source["registration"],
                "settings registration table is not registered")
        return {"jni": validate_jni(source["registration"], source["table"], source["activity"], source["callbacks"])}
    except (AssertionError, KeyError, TypeError, IndexError) as error:
        raise ContractError(f"unsupported bridge schema/source: {error}") from error


if __name__ == "__main__":
    try:
        result = verify(json.loads((GEN.RUST / "bridge_events.v1.json").read_text()), sources())
    except ContractError as error:
        sys.exit(str(error))
    print(f'Android event/JNI structures: OK ({len(result["jni"]["native"])} registered natives)')