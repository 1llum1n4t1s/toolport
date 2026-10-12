import importlib.util
import unittest
import sys

sys.dont_write_bytecode = True
from pathlib import Path

spec = importlib.util.spec_from_file_location("check_rustfmt", Path(__file__).with_name("check-rustfmt.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class ChangedFormatting(unittest.TestCase):
    def test_added_and_deleted_lines(self):
        self.assertEqual(check.changed_lines("@@ -1,2 +1,0 @@\n@@ -9 +8,2 @@\n@@ -20 +21 @@"), {7, 8, 20})

    def test_legacy_hunks_are_ignored(self):
        self.assertEqual(check.format_edits("old  bad\nnew good\n", "old bad\nnew good\n", {1}), [])

    def test_new_violation_is_rejected(self):
        self.assertEqual(check.format_edits("old good\nnew  bad\n", "old good\nnew bad\n", {1}), [(1, 2, ["new bad\n"])])

    def test_expanded_hunk_keeps_full_replacement(self):
        self.assertEqual(check.format_edits("fn f(){x();}\n", "fn f() {\n    x();\n}\n", {0}), [(0, 1, ["fn f() {\n", "    x();\n", "}\n"])])

    def test_formatter_insertion_at_eof(self):
        self.assertEqual(check.format_edits("line\n", "line\n\n", {0}), [(1, 1, ["\n"])])


if __name__ == "__main__":
    unittest.main()
