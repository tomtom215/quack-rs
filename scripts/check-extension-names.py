#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check quack-rs's reserved extension names against upstream DuckDB.

`validate_community_extension_name` refuses a name `DuckDB` itself uses: one of
its built-in aliases (`INSTALL s3` really installs `httpfs`, so a community
extension called `s3` can never be installed) or a core extension's name (the
community-extensions FAQ: such an extension "has to be renamed"). Those lists
live in `src/validate/extension_name.rs` and go stale when DuckDB adds a core
extension or an alias, so this script re-derives them.

Checked:

- `DUCKDB_EXTENSION_ALIASES` must equal `internal_aliases` in
  `src/main/extension/extension_alias.cpp` (pairs, exactly).
- `DUCKDB_CORE_EXTENSION_NAMES` must contain every name in
  `internal_extensions[]` (`src/main/extension/extension_helper.cpp`),
  `AUTOLOADABLE_EXTENSIONS` (`src/include/duckdb/main/extension_entries.hpp`)
  and the core-extension table of the current DuckDB documentation
  (`duckdb/duckdb-web`, `docs/<latest_short_duckdb_version>/core_extensions/
  overview.md`), minus the aliases — and nothing else but the names in
  `HISTORICAL` below, each with its reason.
- `DUCKDB_RESERVED_EXTENSION_NAMES` must be exactly the two combined.
- No published community extension (`_includes/list_of_community_extensions.md`
  in `duckdb/duckdb-web`) may have a reserved name: if one does, either the
  list is wrong or DuckDB allowed the name, and it needs looking at.

Exit codes:
    0  the lists agree
    1  they diverge
    2  an upstream file could not be fetched or parsed (network, layout change)

Usage:
    python3 scripts/check-extension-names.py
    python3 scripts/check-extension-names.py --ref v2.0.0
"""

from __future__ import annotations

import argparse
import re
import sys
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
NAMES_RS = REPO_ROOT / "src" / "validate" / "extension_name.rs"

RAW = "https://raw.githubusercontent.com"
DUCKDB = RAW + "/duckdb/duckdb/{ref}/"
WEB = RAW + "/duckdb/duckdb-web/main/"

# Names kept although current DuckDB no longer lists them, with why.
HISTORICAL = {
    "jemalloc": "a core extension up to DuckDB 1.4.x (internal_extensions[] at v1.4.4)",
}


class Unavailable(Exception):
    """An upstream file could not be fetched or no longer has the expected shape."""


def fetch(url: str) -> str:
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            return response.read().decode("utf-8")
    except Exception as error:
        raise Unavailable(f"could not fetch {url}: {error}") from error


def section(text: str, start: str, end: str, url: str) -> str:
    at = text.find(start)
    stop = text.find(end, at + 1) if at >= 0 else -1
    if at < 0 or stop < 0:
        raise Unavailable(f"{url}: no `{start}` ... `{end}` block (layout changed?)")
    return text[at:stop]


def upstream_aliases(ref: str) -> set[tuple[str, str]]:
    url = DUCKDB.format(ref=ref) + "src/main/extension/extension_alias.cpp"
    body = section(fetch(url), "internal_aliases[]", "{nullptr, nullptr}", url)
    pairs = set(re.findall(r'\{"([a-z0-9_]+)",\s*"([a-z0-9_]+)"\}', body))
    if not pairs:
        raise Unavailable(f"{url}: no aliases found")
    return pairs


def upstream_internal(ref: str) -> set[str]:
    url = DUCKDB.format(ref=ref) + "src/main/extension/extension_helper.cpp"
    body = section(fetch(url), "internal_extensions[]", "{nullptr, nullptr", url)
    names = set(re.findall(r'\{"([a-z0-9_]+)",', body))
    if len(names) < 10:
        raise Unavailable(f"{url}: only {len(names)} internal extensions found")
    return names


def upstream_autoloadable(ref: str) -> set[str]:
    url = DUCKDB.format(ref=ref) + "src/include/duckdb/main/extension_entries.hpp"
    body = section(fetch(url), "AUTOLOADABLE_EXTENSIONS[]", "END_OF_AUTOLOADABLE", url)
    names = set(re.findall(r'"([a-z0-9_]+)"', body))
    if len(names) < 10:
        raise Unavailable(f"{url}: only {len(names)} autoloadable extensions found")
    return names


def latest_docs_version() -> str:
    url = WEB + "_config.yml"
    match = re.search(
        r'^latest_short_duckdb_version:\s*"([0-9.]+)"', fetch(url), re.MULTILINE
    )
    if not match:
        raise Unavailable(f"{url}: no latest_short_duckdb_version")
    return match.group(1)


def documented_core(version: str) -> set[str]:
    url = WEB + f"docs/{version}/core_extensions/overview.md"
    names = set(re.findall(r"^\|\s*\[([a-z0-9_]+)\]\(", fetch(url), re.MULTILINE))
    if len(names) < 10:
        raise Unavailable(f"{url}: only {len(names)} core extensions in the table")
    return names


def published_community() -> set[str]:
    url = WEB + "_includes/list_of_community_extensions.md"
    names = set(re.findall(r"^\|\s*\[([a-z0-9_]+)\]\(", fetch(url), re.MULTILINE))
    if len(names) < 100:
        raise Unavailable(f"{url}: only {len(names)} community extensions listed")
    return names


def rust_names(text: str, name: str) -> list[str] | None:
    match = re.search(
        rf"pub const {name}:\s*&\[&str\]\s*=\s*&\[(.*?)\];", text, re.DOTALL
    )
    return re.findall(r'"([^"]+)"', match.group(1)) if match else None


def rust_aliases(text: str) -> list[tuple[str, str]] | None:
    match = re.search(
        r"pub const DUCKDB_EXTENSION_ALIASES:\s*&\[\(&str, &str\)\]\s*=\s*&\[(.*?)\];",
        text,
        re.DOTALL,
    )
    return re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', match.group(1)) if match else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--ref", default="main", help="DuckDB git ref (default: main)")
    parser.add_argument(
        "--docs-version",
        help="DuckDB documentation version (default: latest_short_duckdb_version in duckdb-web)",
    )
    parser.add_argument(
        "--rust-file",
        type=Path,
        default=NAMES_RS,
        help="the Rust file holding the lists",
    )
    args = parser.parse_args()

    text = args.rust_file.read_text(encoding="utf-8")
    ours_aliases = rust_aliases(text)
    ours_core = rust_names(text, "DUCKDB_CORE_EXTENSION_NAMES")
    ours_reserved = rust_names(text, "DUCKDB_RESERVED_EXTENSION_NAMES")
    if ours_aliases is None or ours_core is None or ours_reserved is None:
        sys.exit(f"could not parse the name lists out of {args.rust_file}")

    try:
        aliases = upstream_aliases(args.ref)
        docs_version = args.docs_version or latest_docs_version()
        sources = {
            f"internal_extensions[] at {args.ref}": upstream_internal(args.ref),
            f"AUTOLOADABLE_EXTENSIONS at {args.ref}": upstream_autoloadable(args.ref),
            f"the DuckDB {docs_version} core-extension table": documented_core(
                docs_version
            ),
        }
        community = published_community()
    except Unavailable as error:
        print(f"::warning::{error}")
        return 2

    alias_names = {alias for alias, _ in aliases}
    upstream_core = set().union(*sources.values()) - alias_names
    problems: list[tuple[str, list[str]]] = []

    def fail(label: str, names) -> None:
        if names:
            problems.append((label, sorted(names)))

    fail(
        "an upstream alias is missing from DUCKDB_EXTENSION_ALIASES",
        [f"{a} -> {t}" for a, t in aliases - set(ours_aliases)],
    )
    fail(
        "DUCKDB_EXTENSION_ALIASES has an alias upstream does not",
        [f"{a} -> {t}" for a, t in set(ours_aliases) - aliases],
    )
    for label, names in sources.items():
        fail(
            f"{label} names a core extension DUCKDB_CORE_EXTENSION_NAMES lacks",
            (names - alias_names) - set(ours_core),
        )
    fail(
        "DUCKDB_CORE_EXTENSION_NAMES lists a name no upstream source has (and HISTORICAL "
        "does not explain)",
        set(ours_core) - upstream_core - set(HISTORICAL),
    )
    fail(
        "HISTORICAL lists a name DUCKDB_CORE_EXTENSION_NAMES does not have",
        set(HISTORICAL) - set(ours_core),
    )
    expected_reserved = set(ours_core) | {alias for alias, _ in ours_aliases}
    fail(
        "DUCKDB_RESERVED_EXTENSION_NAMES is not exactly the aliases and the core names",
        set(ours_reserved) ^ expected_reserved,
    )
    if ours_reserved != sorted(ours_reserved):
        problems.append(("DUCKDB_RESERVED_EXTENSION_NAMES is not sorted", []))
    fail(
        "a published community extension has a reserved name (is the list wrong?)",
        community & expected_reserved,
    )

    print(
        f"upstream ({args.ref}, docs {docs_version}): {len(aliases)} aliases, "
        f"{len(upstream_core)} core extension names; {len(community)} published community "
        "extensions"
    )
    print(
        f"quack-rs: {len(ours_aliases)} aliases, {len(ours_core)} core names "
        f"({len(HISTORICAL)} historical), {len(ours_reserved)} reserved"
    )
    for label, names in problems:
        print(f"\nFAIL: {label}" + (f": {', '.join(names)}" if names else ""))
    if problems:
        print(
            f"\nUpdate the lists in {args.rust_file} (and HISTORICAL here if a name is kept)."
        )
        return 1
    print(f"\nOK: {args.rust_file.name} matches upstream")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
