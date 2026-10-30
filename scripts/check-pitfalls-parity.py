#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check that LESSONS.md and the book's pitfall page describe the same pitfalls.

`LESSONS.md` (the long form, read on GitHub) and `book/src/reference/pitfalls.md`
(the condensed form, read at quack-rs.com) are two hand-written copies. They are
meant to differ in length, not in content, and they had drifted: a pitfall added
to one and not the other, a title reworded, a count updated in one place.

This script does not compare prose. It fails when the two files disagree on:

- the set and order of the pitfall ids (the `L` and `P` series);
- a pitfall's title, ignoring backticks and the `DuckDB` / `DuckDB` spelling;
- the "The N known pitfalls" count in the book page's introduction.

With `--identifiers` it also lists backticked identifiers (types, functions,
`duckdb_*` C functions, test names) that a pitfall's book section uses and its
LESSONS.md section does not. The condensed form should not make claims the long
form lacks. That check is informational (exit status unaffected).

Exit codes: 0 consistent, 1 inconsistent, 2 a file could not be read.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LESSONS = ROOT / "LESSONS.md"
BOOK = ROOT / "book" / "src" / "reference" / "pitfalls.md"

HEADING = re.compile(r"^## ((?:L|P)\d+): (.*?)(?: \{#[\w-]+\})?\s*$", re.M)


def sections(text: str) -> dict[str, tuple[str, str]]:
    """Map pitfall id -> (title, body)."""
    found = list(HEADING.finditer(text))
    out: dict[str, tuple[str, str]] = {}
    for i, m in enumerate(found):
        end = found[i + 1].start() if i + 1 < len(found) else len(text)
        # A following non-pitfall `## ` heading ends the last section.
        nxt = re.search(r"^## (?!(?:L|P)\d+:)", text[m.end():end], re.M)
        if nxt:
            end = m.end() + nxt.start()
        out[m.group(1)] = (m.group(2), text[m.end():end])
    return out


def norm_title(title: str) -> str:
    return re.sub(r"\s+", " ", title.replace("`", "")).strip().lower()


def idents(body: str) -> set[str]:
    # Hidden doctest lines (`# use ...`) are scaffolding, not claims.
    body = "\n".join(l for l in body.split("\n") if not l.startswith("# ") and l != "#")
    found = set()
    for tok in re.findall(r"`([^`\n]+)`", body):
        for w in re.findall(r"[A-Za-z_][A-Za-z0-9_]{3,}", tok):
            if "_" in w or any(c.isupper() for c in w[1:]):
                found.add(w)
    return found


def main() -> int:
    show_idents = "--identifiers" in sys.argv[1:]
    try:
        long_text = LESSONS.read_text(encoding="utf-8")
        book_text = BOOK.read_text(encoding="utf-8")
    except OSError as e:
        print(f"cannot read {e.filename}: {e.strerror}", file=sys.stderr)
        return 2

    a, b = sections(long_text), sections(book_text)
    problems: list[str] = []

    if list(a) != list(b):
        only_a = [k for k in a if k not in b]
        only_b = [k for k in b if k not in a]
        problems.append(
            f"pitfall ids differ: only in LESSONS.md {only_a or '-'}, "
            f"only in the book {only_b or '-'}"
            + ("" if sorted(a) != sorted(b) else " (same ids, different order)")
        )
    for k in a:
        if k in b and norm_title(a[k][0]) != norm_title(b[k][0]):
            problems.append(f"{k}: titles differ\n    LESSONS.md: {a[k][0]}\n    book:       {b[k][0]}")

    m = re.search(r"The (\d+) known pitfalls", book_text)
    if not m:
        problems.append("the book page no longer says 'The N known pitfalls'")
    elif int(m.group(1)) != len(b):
        problems.append(f"the book page says {m.group(1)} pitfalls but has {len(b)}")

    for p in problems:
        print(f"error: {p}")

    if show_idents:
        for k in b:
            if k in a:
                extra = sorted(idents(b[k][1]) - idents(a[k][1]))
                if extra:
                    print(f"note: {k}: in the book section but not in LESSONS.md: {', '.join(extra)}")

    if problems:
        return 1
    print(f"LESSONS.md and the book agree on {len(a)} pitfalls ({', '.join(a)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
