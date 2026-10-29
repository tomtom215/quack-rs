#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check every link in the book and README that can be checked offline.

mdBook reports neither a link to a missing `#anchor` nor a broken docs.rs link,
so both used to go stale unnoticed. This checks, in `book/src/**/*.md` (except
the changelog, which quotes history) and `README.md`, outside code fences:

- a relative link: the target file exists, and for a book page, the `#anchor`
  is an `id` in the page mdBook rendered (`book/book/`);
- a `https://docs.rs/quack-rs/latest/quack_rs/...` link: the page, and its
  `#anchor`, exist in the rustdoc output of this checkout (`target/doc/`).

Inline links (`[text](url)`) and reference definitions (`[label]: url`) are
both checked. A docs.rs link is checked against *this checkout*: an item added
since the last release resolves here but 404s on docs.rs until the next one.

Build both first:
    mdbook build
    cargo doc --no-deps --features duckdb-1-5-4   # the docs.rs feature set

Exit codes:
    0  every link resolves
    1  a link does not resolve
    2  the rendered book or rustdoc output is missing
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BOOK_SRC = REPO_ROOT / "book" / "src"
BOOK_OUT = REPO_ROOT / "book" / "book"
RUSTDOC = REPO_ROOT / "target" / "doc" / "quack_rs"
DOCS_RS = "https://docs.rs/quack-rs/latest/quack_rs/"

INLINE = re.compile(r"\]\(([^)\s]+)\)")
REFERENCE = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*(\S+)", re.M)
FENCE = re.compile(r"^```.*?^```", re.M | re.S)

_ids: dict[Path, set[str]] = {}


def ids(page: Path) -> set[str]:
    if page not in _ids:
        text = page.read_text(encoding="utf-8", errors="replace")
        _ids[page] = set(re.findall(r'\s(?:id|name)="([^"]+)"', text))
    return _ids[page]


def links(md: Path) -> list[tuple[int, str]]:
    text = md.read_text(encoding="utf-8")
    # Blank out code fences, keeping line numbers.
    text = FENCE.sub(lambda m: "\n" * m.group(0).count("\n"), text)
    found = []
    for pattern in (INLINE, REFERENCE):
        for m in pattern.finditer(text):
            found.append((text.count("\n", 0, m.start()) + 1, m.group(1)))
    return sorted(found)


def check_docs_rs(url: str) -> str | None:
    path, _, frag = url[len(DOCS_RS) :].partition("#")
    page = RUSTDOC / (path + "index.html" if not path or path.endswith("/") else path)
    if not page.is_file():
        return f"no rustdoc page {page.relative_to(REPO_ROOT)}"
    if frag and frag not in ids(page):
        return f"no #{frag} in {page.relative_to(REPO_ROOT)}"
    return None


def check_relative(md: Path, url: str) -> str | None:
    path, _, frag = url.partition("#")
    target = (md.parent / path).resolve() if path else md.resolve()
    if not target.exists():
        return f"no file {path}"
    if not frag or target.suffix != ".md":
        return None
    try:
        rel = target.relative_to(BOOK_SRC.resolve())
    except ValueError:
        return None  # a repository file outside the book: GitHub renders it
    page = BOOK_OUT / rel.with_suffix(".html")
    if not page.is_file():
        return f"no rendered page {page.relative_to(REPO_ROOT)}"
    if frag not in ids(page):
        return f"no #{frag} in {page.relative_to(REPO_ROOT)}"
    return None


def main() -> int:
    for needed, how in ((BOOK_OUT, "mdbook build"), (RUSTDOC, "cargo doc")):
        if not needed.is_dir():
            print(f"{needed.relative_to(REPO_ROOT)} is missing: run `{how}` first")
            return 2
    pages = sorted(p for p in BOOK_SRC.rglob("*.md") if p.name != "changelog.md")
    pages.append(REPO_ROOT / "README.md")
    broken = []
    counts = {"docs.rs": 0, "relative": 0}
    for md in pages:
        for line, url in links(md):
            if url.startswith(DOCS_RS):
                counts["docs.rs"] += 1
                problem = check_docs_rs(url)
            elif re.match(r"[a-z][a-z0-9+.-]*:", url):
                continue  # other absolute URLs are not checked offline
            else:
                counts["relative"] += 1
                problem = check_relative(md, url)
            if problem:
                broken.append(f"{md.relative_to(REPO_ROOT)}:{line}: {url}: {problem}")
    print(f"checked {counts['relative']} relative and {counts['docs.rs']} docs.rs links")
    for b in broken:
        print(f"BROKEN {b}")
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
