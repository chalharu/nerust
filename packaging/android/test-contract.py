#!/usr/bin/env python3
"""Regression tests for single-source contract generation (stdlib only)."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True

SPEC = importlib.util.spec_from_file_location("contract", Path(__file__).with_name("generate-contract.py"))
CONTRACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTRACT)


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


if __name__ == "__main__":
    unittest.main()
