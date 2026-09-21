#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reject GitHub Actions workflow files containing an invalid `${{ }}` expression.

Why this exists
---------------
On 2026-09-21 `.github/workflows/mutants.yml` was rejected by GitHub with:

    Invalid workflow file: .github/workflows/mutants.yml
    (Line: 203, Col: 14): An expression was expected

The cause was a *shell comment* inside a `run:` block that mentioned an empty
expression literally, while explaining that a value should NOT be passed that
way. GitHub evaluates expressions anywhere in the file, including inside shell
comments, so an empty one is a syntax error that invalidates the whole
workflow.

An invalid workflow file does not fail loudly. GitHub records a run with **zero
jobs**, and the workflow simply stops running -- here the mutation-testing gate
silently stopped executing on pull requests, which is exactly the class of
"a gate that is not actually running" this repository has been cleaning up.

Neither `yaml.safe_load` nor a JSON-Schema check catches it: both treat the
`run:` block as an opaque string. `actionlint` does catch it, but it ships as a
Go binary from GitHub releases and cannot be checksum-pinned the way
`osv-scanner` is in `ci.yml`, so this dependency-free check covers the specific
class instead. Adding `actionlint` as well would be a strict improvement if a
pinned, verifiable install is ever available.

Usage:
    scripts/check-workflow-expressions.py [paths...]   # default: .github/workflows/*.yml
"""

from __future__ import annotations

import pathlib
import re
import sys

OPEN = "${{"
CLOSE = "}}"


def check(path: pathlib.Path) -> list[str]:
    """Returns a list of human-readable problems found in `path`."""
    text = path.read_text(encoding="utf-8")
    problems: list[str] = []
    pos = 0
    while True:
        start = text.find(OPEN, pos)
        if start == -1:
            break
        line = text.count("\n", 0, start) + 1
        end = text.find(CLOSE, start + len(OPEN))
        if end == -1:
            problems.append(f"{path}:{line}: `{OPEN}` is never closed")
            break
        inner = text[start + len(OPEN) : end]
        if not inner.strip():
            problems.append(
                f"{path}:{line}: empty expression `{OPEN}{inner}{CLOSE}` -- "
                f"GitHub rejects the whole file with 'An expression was expected'. "
                f"Reword to avoid the literal `{OPEN}` sequence."
            )
        pos = end + len(CLOSE)
    return problems


def main(argv: list[str]) -> int:
    paths = [pathlib.Path(a) for a in argv[1:]]
    if not paths:
        paths = sorted(pathlib.Path(".github/workflows").glob("*.yml"))
    if not paths:
        print("no workflow files found", file=sys.stderr)
        return 1

    problems: list[str] = []
    expressions = 0
    for p in paths:
        problems.extend(check(p))
        expressions += len(re.findall(re.escape(OPEN), p.read_text(encoding="utf-8")))

    if problems:
        print("Invalid workflow expression(s):\n", file=sys.stderr)
        for pr in problems:
            print(f"  {pr}", file=sys.stderr)
        return 1

    print(f"{len(paths)} workflow file(s), {expressions} expressions, all well-formed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
