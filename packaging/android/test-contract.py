#!/usr/bin/env python3
"""Regression tests for single-source contract generation (stdlib only)."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True

SPEC = importlib.util.spec_from_file_location("contract", Path(__file__).with_name("generate-contract.py"))
CONTRACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTRACT)
VERIFY_SPEC = importlib.util.spec_from_file_location("verify_contract", Path(__file__).with_name("verify-contract.py"))
VERIFY = importlib.util.module_from_spec(VERIFY_SPEC)
VERIFY_SPEC.loader.exec_module(VERIFY)
DECODER_SPEC = importlib.util.spec_from_file_location("decoder_contract", Path(__file__).with_name("test-decoder-contract.py"))
DECODER = importlib.util.module_from_spec(DECODER_SPEC)
DECODER_SPEC.loader.exec_module(DECODER)


class ContractTests(unittest.TestCase):
    def setUp(self):
        self.schema = json.loads((CONTRACT.RUST / "bridge_events.v1.json").read_text())

    def test_checked_in_outputs_match_exactly(self):
        for path, content in CONTRACT.outputs(self.schema).items():
            self.assertEqual(path.read_text(), content, str(path))

    def test_schema_changes_update_both_languages(self):
        baseline = CONTRACT.outputs(self.schema)
        for key in ("x-settings-schema-version", "x-saf-chunk-bytes"):
            changed = copy.deepcopy(self.schema)
            changed[key] += 1
            for path, content in CONTRACT.outputs(changed).items():
                self.assertNotEqual(baseline[path], content)

    def test_mismatched_event_versions_are_rejected(self):
        self.schema["definitions"]["menuEvent"]["properties"]["v"]["const"] += 1
        with self.assertRaises(AssertionError):
            CONTRACT.outputs(self.schema)

    def test_invalid_chunks_and_duplicate_actions_are_rejected(self):
        changed = copy.deepcopy(self.schema)
        changed["x-saf-chunk-bytes"] = 0
        with self.assertRaises(AssertionError):
            CONTRACT.outputs(changed)
        actions = self.schema["definitions"]["menuEvent"]["properties"]["action"]["enum"]
        actions.append(actions[0])
        with self.assertRaises(AssertionError):
            CONTRACT.outputs(self.schema)

    def test_unsupported_schema_rules_are_not_ignored(self):
        original = copy.deepcopy(self.schema)
        for rule, value in (("pattern", "a+"), ("minLength", 1), ("format", "uuid")):
            with self.subTest(rule=rule):
                changed = copy.deepcopy(original)
                changed["definitions"]["menuEvent"]["properties"]["key"][rule] = value
                with self.assertRaises(AssertionError):
                    CONTRACT.outputs(changed)
        changed = copy.deepcopy(original)
        changed["not"] = {"type": "object"}
        with self.assertRaises(AssertionError):
            CONTRACT.outputs(changed)

    def test_unsupported_version_even_when_all_events_agree_is_rejected(self):
        for definition in self.schema["definitions"].values():
            definition["properties"]["v"]["const"] = 2
        with self.assertRaises(AssertionError):
            CONTRACT.outputs(self.schema)

    def test_rust_generated_formatting_is_stable_with_repository_config(self):
        generated = CONTRACT.outputs(self.schema)[CONTRACT.RUST / "src/bridge_contract.rs"]
        self.assertEqual(generated, CONTRACT.format_rust(generated))
        self.assertTrue(generated.startswith("// Generated"))


class StructureTests(unittest.TestCase):
    def setUp(self):
        self.schema = json.loads((CONTRACT.RUST / "bridge_events.v1.json").read_text())
        self.source = VERIFY.sources()

    def verify(self):
        return VERIFY.verify(self.schema, self.source)

    def test_actual_implementations_match(self):
        self.assertTrue(self.verify()["jni"]["native"])

    def test_schema_field_type_name_and_added_required_are_detected(self):
        original = copy.deepcopy(self.schema)
        for mutation in ("type", "name", "required", "new_required", "new_optional", "version_required"):
            with self.subTest(mutation=mutation):
                self.schema = copy.deepcopy(original)
                menu = self.schema["definitions"]["menuEvent"]
                if mutation == "type":
                    menu["properties"]["pressed"]["type"] = "string"
                elif mutation == "name":
                    menu["properties"]["device"] = menu["properties"].pop("device_id")
                elif mutation == "required":
                    menu["required"].append("pressed")
                elif mutation.startswith("new_"):
                    menu["properties"]["request_id"] = {"type": "integer"}
                    if mutation == "new_required":
                        menu["required"].append("request_id")
                else:
                    menu["required"].remove("v")
                with self.assertRaises(VERIFY.ContractError):
                    self.verify()

    def test_decoder_field_type_name_required_and_version_gate_are_detected(self):
        original = self.source["decoder"]
        for before, after in (("pressed: Option<bool>", "pressed: Option<String>"),
                              ("device_id: Option<i32>", "device: Option<i32>"),
                              ("v: u32", "v: Option<u32>"),
                              ("action: String", "action: Option<String>"),
                              ("pressed: Option<bool>", "pressed: Option<Vec<bool>>"),
                              ("pressed: Option<bool>", '#[serde(rename = "down")] pressed: Option<bool>'),
                              ("version != BRIDGE_EVENT_VERSION", "version == BRIDGE_EVENT_VERSION"),
                              ("validate_bridge_event(&mut value)?;", ""),
                              ("serde_json::from_value(value)", "serde_json::from_str(raw)")):
            with self.subTest(before=before):
                self.assertIn(before, original)
                self.source["decoder"] = original.replace(before, after)
                with self.assertRaises(VERIFY.ContractError):
                    self.verify()

    def test_regeneration_cannot_hide_schema_name_type_or_mandatory_drift(self):
        original = copy.deepcopy(self.schema)
        for event, mutation in (("menuEvent", "type"), ("menuEvent", "mandatory"),
                                ("errorDialogDismissedEvent", "name"),
                                ("errorDialogDismissedEvent", "type"),
                                ("errorDialogDismissedEvent", "mandatory")):
            with self.subTest(event=event, mutation=mutation):
                schema = copy.deepcopy(original)
                definition = schema["definitions"][event]
                field = "pressed" if event == "menuEvent" else "request_id"
                if mutation == "type":
                    definition["properties"][field] = {"type": "string"}
                elif mutation == "name":
                    definition["properties"]["ack_id"] = definition["properties"].pop(field)
                    definition["required"] = ["ack_id" if name == field else name
                                              for name in definition["required"]]
                elif field in definition["required"]:
                    definition["required"].remove(field)
                else:
                    definition["required"].append(field)
                source = copy.deepcopy(self.source)
                source["generated"] = CONTRACT.outputs(schema)[CONTRACT.RUST / "src/bridge_contract.rs"]
                with self.assertRaises(VERIFY.ContractError):
                    VERIFY.verify(schema, source)

    def test_schema_null_unknown_enum_bounds_and_controller_rules_cannot_drift(self):
        original = copy.deepcopy(self.schema)
        for mutation in ("null", "unknown", "enum", "minimum", "maximum", "conditional"):
            with self.subTest(mutation=mutation):
                self.schema = copy.deepcopy(original)
                menu = self.schema["definitions"]["menuEvent"]
                if mutation == "null":
                    menu["properties"]["pressed"]["type"] = ["boolean", "null"]
                elif mutation == "unknown":
                    menu["additionalProperties"] = True
                elif mutation == "enum":
                    menu["properties"]["key"]["enum"].append("probe")
                elif mutation in {"minimum", "maximum"}:
                    menu["properties"]["device_id"][mutation] += 1
                else:
                    menu["then"]["required"].remove("pressed")
                with self.assertRaises(VERIFY.ContractError):
                    self.verify()

    def test_generated_validation_changes_for_every_schema_rule(self):
        original = copy.deepcopy(self.schema)
        path = CONTRACT.RUST / "src/bridge_contract.rs"
        baseline = CONTRACT.outputs(original)[path]
        for mutation in ("optional", "minimum", "maximum", "enum", "conditional"):
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(original)
                menu = changed["definitions"]["menuEvent"]
                if mutation == "optional":
                    menu["properties"]["probe"] = {"type": "boolean"}
                elif mutation in {"minimum", "maximum"}:
                    menu["properties"]["device_id"][mutation] += 1
                elif mutation == "enum":
                    menu["properties"]["key"]["enum"].append("probe")
                else:
                    menu["then"]["required"].remove("pressed")
                self.assertNotEqual(CONTRACT.outputs(changed)[path], baseline)

    def test_modified_generated_validation_is_rejected(self):
        self.source["generated"] = self.source["generated"].replace("_ => false", "_ => true")
        self.assertNotEqual(self.source["generated"], VERIFY.sources()["generated"])
        with self.assertRaises(VERIFY.ContractError):
            self.verify()

    def test_producer_field_type_name_required_and_version_are_detected(self):
        original = self.source["producer"]
        for before, after in (("pressed: Boolean", "pressed: String"),
                              ('"device_id"', '"device"'),
                              ('.put("action", action)', ""),
                              ('.put("v", BRIDGE_EVENT_VERSION)', '.put("v", "1")'),
                              ('.put("pressed", pressed)', '.put("extra", pressed)')):
            with self.subTest(before=before):
                self.assertIn(before, original)
                self.source["producer"] = original.replace(before, after)
                with self.assertRaises(VERIFY.ContractError):
                    self.verify()

    def test_jni_signature_registration_symbol_and_callback_abi_drift(self):
        original = copy.deepcopy(self.source)
        mutations = [("table", '"(JI)[B"', '"(II)[B"'),
                     ("table", '"(J[B)Z"', '"(I[B)Z"'),
                     ("table", '"(JZ)Z"', '"(J)Z"'),
                     ("table", '"(JLjava/lang/String;)V"', '"(Ljava/lang/String;)V"'),
                     ("registration", '"(J)V"', '"()V"'),
                     ("activity", "sessionId: Long", "sessionId: Int"),
                     ("activity", "requestId: Long", "requestId: Int"),
                     ("activity", "commit: Boolean", "commit: Long"),
                     ("activity", "bytes: ByteArray", "bytes: IntArray"),
                     ("activity", "): Long = safRepository.openSafWriteSession",
                      "): Int = safRepository.openSafWriteSession"),
                     ("registration", '"([I)V"', '"([J)V"'),
                     ("registration", "menu::Java_io_github_chalharu_nerust_MainActivity_onMenuAction as",
                      "menu::Java_io_github_chalharu_nerust_MainActivity_onActivityDestroyed as"),
                     ("activity", "private external fun onMenuAction(action: String)", "")]
        for key, before, after in mutations:
            with self.subTest(key=key, before=before):
                self.source = copy.deepcopy(original)
                self.assertIn(before, self.source[key])
                self.source[key] = self.source[key].replace(before, after)
                with self.assertRaises(VERIFY.ContractError):
                    self.verify()
        self.source = copy.deepcopy(original)
        self.source["callbacks"] = [text.replace("x_g: jni::sys::jfloat", "x_g: jni::sys::jlong")
                                    for text in self.source["callbacks"]]
        self.assertNotEqual(self.source["callbacks"], original["callbacks"])
        with self.assertRaises(VERIFY.ContractError):
            self.verify()

    def test_ack_signature_is_derived_not_pinned_to_old_no_args_abi(self):
        # A coordinated ACK-ID migration must pass; one-sided migrations fail.
        # Derive the synthetic new method name so this stays valid after the
        # Android agent changes the real ACK's signature concurrently.
        registration = '''jni::NativeMethod::from_raw_parts(
            jni_str!("onProbeAck"), jni_str!("(J)V"),
            menu::Java_io_github_chalharu_nerust_MainActivity_onProbeAck as *mut c_void,
        )'''
        table = 'bridge_method!(PROBE, "probe", "(J)V");'
        activity = 'private external fun onProbeAck(id: Long)\nfun probe(id: Long) {}'
        callbacks = ['''pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onProbeAck(
            env: jni::EnvUnowned<'_>, activity: JObject<'_>, id: jni::sys::jlong,
        ) {}''']
        self.assertEqual(VERIFY.validate_jni(registration, table, activity, callbacks)["native"],
                         {"onProbeAck": "(J)V"})
        with self.assertRaises(VERIFY.ContractError):
            VERIFY.validate_jni(registration.replace("(J)V", "()V"), table, activity, callbacks)

    def test_unsupported_bridge_syntax_does_not_silently_pass(self):
        self.source["producer"] = self.source["producer"].replace(
            '.put("pressed", pressed)', '.put("pressed", pressed.toString())')
        with self.assertRaises(VERIFY.ContractError):
            self.verify()

    def test_runtime_probes_derive_fields_and_required_from_schema(self):
        before = DECODER.harness(self.schema)
        event = self.schema["definitions"]["menuEvent"]
        event["properties"]["probe_id"] = {"type": "integer", "minimum": 1, "maximum": 100}
        event["required"].append("probe_id")
        after = DECODER.harness(self.schema)
        self.assertNotEqual(before, after)
        cases = DECODER.probes(self.schema)
        self.assertIn(("menu: missing required probe_id", True),
                      [(label, not accepted and isinstance(payload, dict) and "probe_id" not in payload)
                       for label, payload, accepted in cases])
        self.assertTrue(any(label == "menu: wrong type probe_id" and
                            payload["probe_id"] == "not an integer" and not accepted
                            for label, payload, accepted in cases))

    def test_runtime_probes_cover_optional_null_unknown_bounds_and_conditionals(self):
        cases = {label: accepted for label, _, accepted in DECODER.probes(self.schema)}
        for field in ("device_id", "key", "pressed"):
            self.assertTrue(cases[f"menu: missing optional {field}"])
            self.assertFalse(cases[f"menu: {field}=None"])
            self.assertFalse(cases[f"menu: action=controller missing {field}"])
        self.assertFalse(cases["menu: unknown field extra"])
        self.assertFalse(cases["menu: key='__unknown__'"])
        for event, field in (("menuEvent", "device_id"), ("errorDialogDismissedEvent", "request_id")):
            definition = self.schema["definitions"][event]
            event_type = definition["properties"]["type"]["const"]
            prop = definition["properties"][field]
            for value in (prop["minimum"], prop["maximum"]):
                self.assertTrue(cases[f"{event_type}: boundary {field}={value}"])
            for value in (prop["minimum"] - 1, prop["maximum"] + 1):
                self.assertFalse(cases[f"{event_type}: boundary {field}={value}"])
        self.assertTrue(cases["menu: v=1.0"])
        self.assertFalse(cases["menu: v=1.5"])

    def test_runtime_manifest_is_valid_toml_and_uses_workspace_dependency_versions(self):
        import tomllib
        manifest = tomllib.loads(DECODER.manifest())
        workspace = tomllib.loads((CONTRACT.ROOT / "Cargo.toml").read_text())
        for name in ("serde", "serde_json", "log"):
            self.assertEqual(manifest["dependencies"][name]["version"],
                             workspace["workspace"]["dependencies"][name]["version"])
        for name in ("serde", "serde_json"):
            self.assertIn("std", manifest["dependencies"][name]["features"])
        self.assertIn("derive", manifest["dependencies"]["serde"]["features"])

    def test_probe_labels_are_data_not_rust_format_strings(self):
        self.assertIn('false, "{}", "menu: v={}"', DECODER.harness(self.schema))


class PackagingTests(unittest.TestCase):
    def shell(self, script, *args, cwd=None):
        helpers = Path(__file__).with_name("package-helpers.sh").resolve()
        return subprocess.run(["bash", "-c", 'set -euo pipefail; source "$1"; shift; ' + script,
                               "test-package", str(helpers), *map(str, args)],
                              cwd=cwd, text=True, capture_output=True)

    def test_checksum_basename_for_relative_absolute_and_spaces_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "input.apk"
            source.write_bytes(b"apk bytes")
            for output in ("relative/dist", "directory with spaces", str(root / "absolute/dist")):
                with self.subTest(output=output):
                    result = self.shell('publish_apk "$1" "$2" "$3"', source, output, "nerust-test.apk", cwd=root)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    checksum = root / output / "nerust-test.apk.sha256"
                    self.assertEqual(checksum.read_text().split()[1], "nerust-test.apk")
                    moved = root / "downloaded"
                    checksum.parent.rename(moved)
                    check = subprocess.run(["sha256sum", "-c", checksum.name], cwd=moved, capture_output=True)
                    self.assertEqual(check.returncode, 0, check.stderr)
                    for item in moved.iterdir():
                        item.unlink()
                    moved.rmdir()

    def test_reader_failure_is_closed_even_if_output_looks_clean(self):
        with tempfile.TemporaryDirectory() as directory:
            library = Path(directory) / "libmain.so"
            library.write_bytes(b"native")
            result = self.shell('reader() { printf "clean sections\\n"; return 7; }; '
                                'verify_native_library reader "$1" 40', library)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("cannot inspect", result.stderr)

    def test_debug_compressed_debug_and_oversize_fail_clean_library_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            library = Path(directory) / "libmain.so"
            library.write_bytes(b"native")
            for sections, budget, success in ((".text .rodata", 6, True), (".debug_info", 40, False),
                                              (".debug_line_str", 40, False), (".zdebug_info", 40, False),
                                              (".text", 5, False)):
                with self.subTest(sections=sections, budget=budget):
                    result = self.shell('READER_SECTIONS="$2"; reader() { printf "%s\\n" "$READER_SECTIONS"; }; '
                                        'verify_native_library reader "$1" "$3"', library, sections, budget)
                    self.assertEqual(result.returncode == 0, success, result.stderr)

    def test_missing_reader_and_missing_artifact_fail(self):
        result = self.shell('verify_native_library /nonexistent/reader /nonexistent/libmain.so 40')
        self.assertNotEqual(result.returncode, 0)
        with tempfile.TemporaryDirectory() as directory:
            result = self.shell('publish_apk /nonexistent/source "$1" test.apk', directory)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((Path(directory) / "test.apk.sha256").exists())


if __name__ == "__main__":
    unittest.main()
