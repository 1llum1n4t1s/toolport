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

    def test_fix_keeps_lines_rustfmt_moves(self):
        # A stand-in formatter that sorts `use` lines, like rustfmt does.
        def sort_uses(text):
            lines = text.splitlines(keepends=True)
            uses = sorted(line for line in lines if line.startswith("use "))
            return "".join(uses + [line for line in lines if not line.startswith("use ")])

        source = "use b;\nuse a;\nfn f() {}\n"
        formatted = sort_uses(source)
        fixed, whole = check.fix_source(source, formatted, {1}, sort_uses)
        self.assertEqual(fixed, formatted)
        self.assertTrue(whole)
        self.assertEqual(sorted(fixed.splitlines()), sorted(source.splitlines()))

    def test_fix_stays_partial_when_safe(self):
        def squeeze(text):
            return text.replace("  ", " ")

        source = "old  bad\nkeep\nnew  bad\n"
        fixed, whole = check.fix_source(source, squeeze(source), {2}, squeeze)
        self.assertEqual(fixed, "old  bad\nkeep\nnew bad\n")
        self.assertFalse(whole)

    def test_renames_keep_both_paths(self):
        status = "M\0src-tauri/a.rs\0R087\0src-tauri/old.rs\0src-tauri/new.rs\0A\0src-tauri/b.rs\0"
        self.assertEqual(
            check.changed_files(status),
            [("src-tauri/a.rs",), ("src-tauri/old.rs", "src-tauri/new.rs"), ("src-tauri/b.rs",)],
        )


if __name__ == "__main__":
    unittest.main()
