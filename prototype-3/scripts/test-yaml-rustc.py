#!/usr/bin/env python3
"""Structured diagnostic regression tests; no compiler or desktop required."""
import copy
import importlib.util
import pathlib
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("wrapper", pathlib.Path(__file__).with_name("yaml-rustc.py"))
WRAPPER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(WRAPPER)


class Diagnostics(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.source = pathlib.Path(self.temporary.name) / "node.json"
        self.raw = "α:\r\n\tid: 日本\r\n".encode("utf-8")
        self.source.write_bytes(self.raw)
        self.marker = f"__wakui_yaml_line_2_column_6_file_{str(self.source).encode().hex()}"

    def diagnostic(self, marker=None):
        return {"level": "error", "message": "type mismatch", "spans": [{"is_primary": True,
                "expansion": {"macro_decl_name": (marker or self.marker) + "!", "span": {}}}]}

    def test_primary_expansion_preserves_crlf_unicode_and_tab_bytes(self):
        diagnostic = WRAPPER.decorate(self.diagnostic())
        span = diagnostic["spans"][0]
        self.assertEqual(span["byte_start"], self.raw.index("日".encode()))
        self.assertEqual(span["byte_end"] - span["byte_start"], 3)
        self.assertEqual((span["line_start"], span["column_start"]), (2, 6))
        self.assertTrue(span["is_primary"])
        self.assertFalse(diagnostic["spans"][1]["is_primary"])

    def test_unrelated_child_marker_cannot_hijack_primary(self):
        diagnostic = self.diagnostic()
        diagnostic["children"] = [{"message": self.marker.replace("line_2", "line_1")}]
        self.assertEqual(WRAPPER.decorate(diagnostic)["spans"][0]["line_start"], 2)

    def test_ambiguous_expansions_keep_original_diagnostic(self):
        diagnostic = self.diagnostic()
        diagnostic["spans"] += self.diagnostic(self.marker.replace("line_2", "line_1"))["spans"]
        original = copy.deepcopy(diagnostic)
        self.assertEqual(WRAPPER.decorate(diagnostic), original)

    def test_out_of_bounds_and_missing_files_keep_original(self):
        for marker in [self.marker.replace("line_2", "line_0"), self.marker.replace("column_6", "column_999")]:
            diagnostic = self.diagnostic(marker)
            self.assertEqual(WRAPPER.decorate(copy.deepcopy(diagnostic)), diagnostic)
        self.source.unlink()
        diagnostic = self.diagnostic()
        self.assertEqual(WRAPPER.decorate(copy.deepcopy(diagnostic)), diagnostic)

    def test_schema_error_message_fallback(self):
        diagnostic = {"level": "error", "message": f"unknown field\n{self.marker}", "spans": []}
        self.assertEqual(WRAPPER.decorate(diagnostic)["spans"][0]["line_start"], 2)

    def test_warning_is_not_rewritten(self):
        diagnostic = self.diagnostic()
        diagnostic["level"] = "warning"
        self.assertEqual(WRAPPER.decorate(copy.deepcopy(diagnostic)), diagnostic)


if __name__ == "__main__":
    unittest.main()