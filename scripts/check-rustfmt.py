#!/usr/bin/env python3
"""Check changed Rust lines with rustfmt, leaving legacy formatting alone."""
import argparse
import difflib
import re
import subprocess
from pathlib import Path


def changed_lines(diff):
    lines = set()
    for start, count in re.findall(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", diff, re.M):
        start, count = int(start), int(count or "1")
        lines.update(range(start - 1, start - 1 + count))
    return lines


def format_edits(source, formatted, changed):
    before, after = source.splitlines(keepends=True), formatted.splitlines(keepends=True)
    edits = []
    for tag, first, last, new_first, new_last in difflib.SequenceMatcher(
        None, before, after, autojunk=False
    ).get_opcodes():
        if tag == "equal":
            continue
        # An insertion belongs to its following source line (or the final line).
        affected = range(first, last) if first < last else [min(first, len(before) - 1)]
        if any(line in changed for line in affected):
            edits.append((first, last, after[new_first:new_last]))
    return edits


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="Commit to compare against")
    parser.add_argument("--fix", action="store_true", help="Format only affected hunks")
    args = parser.parse_args()
    base = subprocess.check_output(
        ["git", "rev-parse", "--verify", args.base + "^{commit}"], text=True
    ).strip()
    files = subprocess.check_output(
        ["git", "diff", "--name-only", "--diff-filter=ACMR", "-z", base, "--", "src-tauri"],
        text=True,
    ).split("\0")
    failed = False
    checked = 0
    for name in files:
        if not name.endswith(".rs"):
            continue
        diff = subprocess.check_output(
            ["git", "diff", "--no-ext-diff", "--unified=0", base, "--", name], text=True
        )
        changed = changed_lines(diff)
        if not changed:
            continue
        checked += 1
        source = Path(name).read_text()
        formatted = subprocess.check_output(
            ["rustfmt", "--edition", "2021", "--config", "skip_children=true", "--emit", "stdout"],
            input=source,
            text=True,
        )
        edits = format_edits(source, formatted, changed)
        if not edits:
            continue
        if args.fix:
            lines = source.splitlines(keepends=True)
            for first, last, replacement in reversed(edits):
                lines[first:last] = replacement
            Path(name).write_text("".join(lines))
            print(f"Formatted changed hunks: {name}")
        else:
            failed = True
            for first, _, _ in edits:
                print(f"{name}:{first + 1}: changed lines need rustfmt")
    print(f"Checked changed lines in {checked} Rust files")
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
