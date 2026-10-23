#!/usr/bin/env python3
"""Self-tests for check-test-inventory.py: the e2e gate must ignore comments.

Run directly (no Android SDK needed); also wired into check-contract.sh.
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bridge_schema import load_sibling

inventory = load_sibling("check_test_inventory", "check-test-inventory.py")
selected_e2e_tests = inventory.selected_e2e_tests
diff_names = inventory.diff_names


class SelectedE2eTests(unittest.TestCase):
    def test_normal_selections_count(self):
        text = (
            'run_instrumentation "${TEST_CLASS}#alpha"\n'
            'expect_process_termination "${TEST_CLASS}#beta"\n'
            'run_instrumentation "${TEST_CLASS}#gamma,${TEST_CLASS}#delta"\n'
        )
        self.assertEqual(
            selected_e2e_tests(text), {"alpha", "beta", "gamma", "delta"}
        )

    def test_full_line_comment_is_not_selected(self):
        text = (
            'run_instrumentation "${TEST_CLASS}#alpha"\n'
            '# run_instrumentation "${TEST_CLASS}#beta"\n'
            '   # expect_process_termination "${TEST_CLASS}#gamma"\n'
        )
        self.assertEqual(selected_e2e_tests(text), {"alpha"})

    def test_all_commented_selection_is_empty(self):
        text = '# run_instrumentation "${TEST_CLASS}#alpha"\n'
        self.assertEqual(selected_e2e_tests(text), set())

    def test_trailing_comment_is_not_selected(self):
        text = 'run_instrumentation "${TEST_CLASS}#alpha" # "${TEST_CLASS}#beta"\n'
        self.assertEqual(selected_e2e_tests(text), {"alpha"})

    def test_hash_inside_quotes_is_kept(self):
        # A literal '#' inside quotes is part of the argument, not a comment:
        # the rest of the line is still scanned.
        text = 'run_instrumentation "prefix#${TEST_CLASS}#alpha"\n'
        self.assertEqual(selected_e2e_tests(text), {"alpha"})


class DiffNames(unittest.TestCase):
    def test_unknown_method_is_extra(self):
        missing, extra = diff_names({"alpha"}, {"alpha", "ghost"})
        self.assertEqual(missing, [])
        self.assertEqual(extra, ["ghost"])

    def test_unselected_method_is_missing(self):
        missing, extra = diff_names({"alpha", "skipped"}, {"alpha"})
        self.assertEqual(missing, ["skipped"])
        self.assertEqual(extra, [])

    def test_matching_sets_are_clean(self):
        self.assertEqual(diff_names({"alpha"}, {"alpha"}), ([], []))


if __name__ == "__main__":
    unittest.main()
