#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
"""Write the per-page SEO tags into the built mdBook site.

mdBook 0.4 renders every page from one template and exposes only the
chapter's *source* path (`{{ path }}` = "functions/scalar.md") and a relative
`path_to_root`, so a template cannot produce a page's absolute public URL, and
it has one book-wide `description`. This runs after `mdbook build` and, in
each page's <head>:

- sets <meta name="description"> to a page-specific summary: the page's first
  paragraph (trimmed to whole sentences, <= 155 characters), or the entry in
  DESCRIPTION_OVERRIDES when that paragraph is not a summary (e.g. it only
  introduces a code block with "Add the following:");
- adds <link rel="canonical"> and og:url with the page's public URL
  (https://quack-rs.com/functions/scalar.html; the home page, which mdBook
  writes as both index.html and introduction.html, is https://quack-rs.com/);
- adds og:title and og:description;
- marks 404.html and print.html (the whole book on one page) noindex;
- gives the home page a descriptive <title> instead of "Introduction".

The site-wide tags (og:image, twitter:card, JSON-LD) are in
book/theme/head.hbs. Deriving descriptions from the first paragraph keeps
them in step with the page as it is edited, with no second copy to maintain.

Usage:
    mdbook build && scripts/seo-postbuild.py            # edits book/book/
    scripts/seo-postbuild.py PATH/TO/BUILD              # another build dir

Idempotent. Exits 1 if a page ends up with no description, or two indexable
pages share a description or <title>; prints a warning (a GitHub annotation
in Actions) when a derived description looks weak, so an override can be
added here.
"""

from __future__ import annotations

import argparse
import html
import os
import re
import sys
from html.parser import HTMLParser
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BUILD = REPO_ROOT / "book" / "book"
SITE_URL = "https://quack-rs.com"
SITE_NAME = "quack-rs"
MAX_LEN = 155
MIN_LEN = 60

HOME_SOURCE = "introduction.md"
HOME_TITLE = "quack-rs: the Rust SDK for DuckDB loadable extensions"
NOINDEX = {"404.html", "print.html"}

# Keyed by the page's source path under book/src. Only for pages whose first
# paragraph is not a usable summary; everything else is derived.
DESCRIPTION_OVERRIDES = {
    "introduction.md": (
        "quack-rs is a Rust SDK for DuckDB loadable extensions, no C or C++ "
        "needed: scalar, aggregate, table, cast, copy and replacement-scan "
        "functions, SQL macros."
    ),
    "getting-started/quick-start.md": (
        "Build a working DuckDB extension in Rust with quack-rs in three steps: "
        "add the dependency, write the extension, then build and load it in DuckDB."
    ),
    "getting-started/installation.md": (
        "Add quack-rs to a DuckDB extension crate: the Cargo.toml dependencies, "
        "the required cdylib and panic settings, the MSRV and test dependencies."
    ),
    "getting-started/first-extension.md": (
        "A walkthrough of hello-ext, the reference DuckDB extension bundled with "
        "quack-rs: an aggregate, scalar, table and cast function, end to end."
    ),
    "functions/replacement-scan.md": (
        "Replacement scans let users write SELECT * FROM 'file.myformat' and have "
        "your Rust extension scan it, the way DuckDB's CSV and Parquet readers work."
    ),
    "functions/copy-functions.md": (
        "Implement a custom COPY file format for DuckDB in Rust: bind, sink and "
        "finalize callbacks for COPY TO, and a table function for COPY FROM."
    ),
}

VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link",
        "meta", "param", "source", "track", "wbr"}
BLOCK_START = "<!-- seo:page -->"
BLOCK_END = "<!-- /seo:page -->"
BLOCK_RE = re.compile(re.escape(BLOCK_START) + r".*?" + re.escape(BLOCK_END) + r"\n?", re.S)
DESC_RE = re.compile(r'<meta name="description" content="[^"]*">')
TITLE_RE = re.compile(r"<title>(.*?)</title>", re.S)


class FirstParagraphs(HTMLParser):
    """Collects the text of the <p> elements that are direct children of <main>."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.stack: list[str] = []
        self.paragraphs: list[str] = []
        self._cur: list[str] | None = None

    def handle_starttag(self, tag, attrs):
        if tag in VOID:
            return
        self.stack.append(tag)
        if tag == "p" and self._cur is None and self.stack[-2:-1] == ["main"]:
            self._cur = []

    def handle_endtag(self, tag):
        if tag in VOID or tag not in self.stack:
            return
        while self.stack:
            top = self.stack.pop()
            if top == "p" and self._cur is not None and self.stack[-1:] == ["main"]:
                self.paragraphs.append(re.sub(r"\s+", " ", "".join(self._cur)).strip())
                self._cur = None
            if top == tag:
                break

    def handle_data(self, data):
        if self._cur is not None:
            self._cur.append(data)


def summarize(text: str) -> str:
    """`text` cut to whole sentences within MAX_LEN, or to a word boundary."""
    if len(text) <= MAX_LEN:
        return text
    out = ""
    for sentence in re.split(r"(?<=[.!?])\s+", text):
        candidate = f"{out} {sentence}".strip()
        if len(candidate) > MAX_LEN:
            break
        out = candidate
    if out:
        return out
    return text[: MAX_LEN - 1].rsplit(" ", 1)[0].rstrip(",;:—– ") + "…"


def source_path(rel: str) -> str:
    return HOME_SOURCE if rel == "index.html" else rel[: -len(".html")] + ".md"


def public_url(rel: str) -> str:
    if source_path(rel) == HOME_SOURCE:
        return SITE_URL + "/"
    return f"{SITE_URL}/{rel}"


def warn(msg: str) -> None:
    prefix = "::warning::" if os.environ.get("GITHUB_ACTIONS") else "warning: "
    print(prefix + msg)


def process(page: Path, rel: str) -> tuple[str, str] | None:
    """Rewrite `page`; return its (title, description) if it is indexable."""
    text = page.read_text(encoding="utf-8")
    # A re-run swaps the block back for mdBook's own (empty) description tag.
    text = BLOCK_RE.sub('<meta name="description" content="">\n', text)
    if not DESC_RE.search(text):
        raise SystemExit(f"error: {rel}: no <meta name=\"description\"> to replace")

    if rel in NOINDEX:
        block = "\n        ".join(
            [BLOCK_START, '<meta name="robots" content="noindex, follow">', BLOCK_END]
        )
        text = DESC_RE.sub(lambda _: block, text, count=1)
        page.write_text(text, encoding="utf-8")
        return None

    src = source_path(rel)
    title_m = TITLE_RE.search(text)
    title = html.unescape(title_m.group(1).strip()) if title_m else SITE_NAME
    if src == HOME_SOURCE:
        title = HOME_TITLE
        text = TITLE_RE.sub(f"<title>{html.escape(title, quote=False)}</title>", text, count=1)
        og_title = HOME_TITLE
    else:
        og_title = title

    if src in DESCRIPTION_OVERRIDES:
        desc = DESCRIPTION_OVERRIDES[src]
        if len(desc) > MAX_LEN:
            warn(f"{src}: override description is {len(desc)} chars (> {MAX_LEN})")
    else:
        parser = FirstParagraphs()
        parser.feed(text)
        first = next((p for p in parser.paragraphs if p), "")
        desc = summarize(first)
        if len(desc) < MIN_LEN or desc.endswith(":"):
            warn(
                f"{src}: derived description is weak ({desc!r}); "
                "add an entry to DESCRIPTION_OVERRIDES in scripts/seo-postbuild.py"
            )

    url = public_url(rel)
    e = lambda s: html.escape(s, quote=True)  # noqa: E731
    block = "\n        ".join(
        [
            BLOCK_START,
            f'<meta name="description" content="{e(desc)}">',
            f'<link rel="canonical" href="{e(url)}">',
            f'<meta property="og:url" content="{e(url)}">',
            f'<meta property="og:title" content="{e(og_title)}">',
            f'<meta property="og:description" content="{e(desc)}">',
            BLOCK_END,
        ]
    )
    text = DESC_RE.sub(lambda _: block, text, count=1)
    page.write_text(text, encoding="utf-8")
    return title, desc


def main() -> int:
    parser = argparse.ArgumentParser(description="Add per-page SEO tags to a built mdBook.")
    parser.add_argument("build_dir", nargs="?", type=Path, default=DEFAULT_BUILD)
    args = parser.parse_args()
    root: Path = args.build_dir
    if not (root / "index.html").is_file():
        print(f"error: {root} has no index.html; run `mdbook build` first", file=sys.stderr)
        return 2

    by_url: dict[str, tuple[str, str, str]] = {}
    for page in sorted(root.rglob("*.html")):
        head = page.read_text(encoding="utf-8", errors="replace")[:400]
        if "Book generated using mdBook" not in head:
            continue
        rel = page.relative_to(root).as_posix()
        result = process(page, rel)
        if result is not None:
            by_url.setdefault(public_url(rel), (rel, *result))

    failed = False
    seen_desc: dict[str, str] = {}
    seen_title: dict[str, str] = {}
    for rel, title, desc in by_url.values():
        if not desc:
            print(f"error: {rel}: no description (empty first paragraph and no override)")
            failed = True
        elif desc in seen_desc:
            print(f"error: {rel}: same description as {seen_desc[desc]}")
            failed = True
        seen_desc.setdefault(desc, rel)
        if title in seen_title:
            print(f"error: {rel}: same <title> as {seen_title[title]}: {title!r}")
            failed = True
        seen_title.setdefault(title, rel)

    print(f"SEO tags written to {len(by_url)} indexable pages in {root}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
