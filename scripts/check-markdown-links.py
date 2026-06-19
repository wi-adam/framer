#!/usr/bin/env python3
"""Verify that every relative link in tracked markdown resolves to a file.

This guards against doc-internal link rot — moved, renamed, or typo'd paths,
e.g. when a spec moves between docs/specs/ and docs/plans/. It validates
LOCAL/relative links only: external URLs (http/https/mailto/tel) are
intentionally NOT fetched, because liveness checks are flaky and rate-limited
and this is about the deterministic, high-value part of doc maintenance.

Stdlib only; deterministic. Run locally:

    python3 scripts/check-markdown-links.py

Exits non-zero (and lists offenders) if any relative link points at a missing
file. Same-file `#anchor` links and `<placeholder>` paths in templates are
skipped; fenced code blocks are ignored.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys

# Strip ``` fenced code blocks so example link syntax inside them isn't checked.
FENCE = re.compile(r"```.*?```", re.DOTALL)
# Inline links/images: [text](target) and ![alt](target). Ignores escaped \[.
LINK = re.compile(r"(?<!\\)!?\[[^\]]*\]\(([^)]+)\)")
EXTERNAL = ("http://", "https://", "mailto:", "tel:")


def tracked_markdown() -> list[str]:
    out = subprocess.check_output(["git", "ls-files", "*.md"], text=True)
    return [line for line in out.splitlines() if line]


def link_targets(text: str):
    for match in LINK.finditer(FENCE.sub("", text)):
        raw = match.group(1).strip()
        # Drop any "title" after a space, then any #anchor fragment.
        path = raw.split()[0].split("#")[0] if raw else ""
        yield raw, path


def main() -> int:
    files = tracked_markdown()
    broken: list[tuple[str, str, str]] = []
    checked = 0

    for path in files:
        base = os.path.dirname(path)
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
        for raw, target in link_targets(text):
            if not target:
                continue  # same-file #anchor — not validated
            if target.startswith(EXTERNAL):
                continue
            if "<" in target or ">" in target:
                continue  # template placeholder, e.g. ../specs/<feature>.md
            checked += 1
            resolved = os.path.normpath(os.path.join(base, target))
            if not os.path.exists(resolved):
                broken.append((path, raw, resolved))

    print(f"Checked {checked} local markdown links across {len(files)} files.")
    if broken:
        print("\nBroken links:")
        for path, raw, resolved in broken:
            print(f"  {path}: ({raw}) -> {resolved}")
        return 1
    print("OK: no broken local links.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
