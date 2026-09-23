#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
#
# Generates sitemap.xml from book/src/SUMMARY.md.
#
# Usage:
#   scripts/generate-sitemap.py            # rewrite book/src/sitemap.xml
#   scripts/generate-sitemap.py --check    # exit 1 if the committed file is stale
#   scripts/generate-sitemap.py --output PATH
#
# Paths resolve from the repository root, so it runs from any directory. The
# committed sitemap is what the published book serves; without `--check` in
# CI it silently fell 14 pages behind SUMMARY.md.

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SITE_URL = "https://quack-rs.com"
SUMMARY = REPO_ROOT / "book" / "src" / "SUMMARY.md"
OUTPUT = REPO_ROOT / "book" / "src" / "sitemap.xml"

# Priority tiers based on path depth and importance
PRIORITY_OVERRIDES = {
    "/": "1.0",
    "/getting-started/quick-start.html": "0.9",
}
CHANGEFREQ_OVERRIDES = {
    "/": "weekly",
    "/reference/changelog.html": "weekly",
}


def md_to_html(md_path: str) -> str:
    """Convert a .md path to its mdBook HTML output path."""
    if md_path == "introduction.md":
        return "/"
    return "/" + md_path.replace(".md", ".html")


def priority_for(html_path: str) -> str:
    if html_path in PRIORITY_OVERRIDES:
        return PRIORITY_OVERRIDES[html_path]
    depth = html_path.strip("/").count("/")
    if depth == 0:
        return "0.7"
    if depth == 1:
        return "0.7"
    return "0.6"


def changefreq_for(html_path: str) -> str:
    return CHANGEFREQ_OVERRIDES.get(html_path, "monthly")


def parse_summary(text: str) -> list[str]:
    """Extract .md paths from SUMMARY.md link entries."""
    paths = []
    for match in re.finditer(r"\[.*?\]\((.*?\.md)\)", text):
        paths.append(match.group(1))
    return paths


def render(summary_text: str) -> tuple[str, int]:
    """The sitemap for `summary_text`, and how many URLs it lists."""
    md_paths = parse_summary(summary_text)

    lines = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">',
    ]

    for md_path in md_paths:
        html_path = md_to_html(md_path)
        loc = SITE_URL + html_path
        freq = changefreq_for(html_path)
        prio = priority_for(html_path)
        lines.append(
            f"  <url>"
            f"<loc>{loc}</loc>"
            f"<changefreq>{freq}</changefreq>"
            f"<priority>{prio}</priority>"
            f"</url>"
        )

    lines.append("</urlset>")
    lines.append("")
    return "\n".join(lines), len(md_paths)


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate the book's sitemap.xml.")
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; exit 1 if the output file differs from what would be generated",
    )
    parser.add_argument("--output", type=Path, default=OUTPUT, help=f"default: {OUTPUT}")
    args = parser.parse_args()

    expected, count = render(SUMMARY.read_text(encoding="utf-8"))
    if args.check:
        current = args.output.read_text(encoding="utf-8") if args.output.exists() else ""
        if current == expected:
            print(f"{args.output} is up to date ({count} URLs)")
            return 0
        have = set(re.findall(r"<loc>(.*?)</loc>", current))
        want = set(re.findall(r"<loc>(.*?)</loc>", expected))
        print(f"{args.output} is stale: run scripts/generate-sitemap.py")
        for url in sorted(want - have):
            print(f"  missing: {url}")
        for url in sorted(have - want):
            print(f"  extra:   {url}")
        return 1

    args.output.write_text(expected, encoding="utf-8")
    print(f"Generated {args.output} with {count} URLs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
