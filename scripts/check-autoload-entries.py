#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check quack-rs's autoload name lists against every supported DuckDB release.

`src/catalog.rs` refuses a `Type` or `Collation` catalog lookup of a name that
makes `DuckDB` autoload an extension, because `duckdb_catalog_get_entry` has no
try/catch and a failed autoload aborts the process. The names come from
`EXTENSION_TYPES` and `EXTENSION_COLLATIONS` in `DuckDB`'s
`src/include/duckdb/main/extension_entries.hpp`, which change between releases.

An extension built with quack-rs may run on any supported `DuckDB`, so the Rust
lists must cover the union over all of them:

- a name some release autoloads for but quack-rs does not list is a lookup that
  can abort the process -> exit 1;
- a name quack-rs lists that no supported release autoloads for is only a
  needless refusal -> reported, but not a failure.

Releases are enumerated from the upstream tags (v1.5.0 and later, no
pre-release suffixes), so a new release is picked up without editing this file.
The floor is 1.5.0, not the crate's 1.4.4: the catalog wrappers need the
`duckdb-1-5` feature, and `src/abi.rs` refuses to load such a build into a
1.4.x engine, so a 1.4 list can never apply. (1.4.x's `EXTENSION_TYPES`
also names `geometry`, for `spatial`; 1.5.0 dropped it.)

Exit codes:
    0  the lists cover every supported release
    1  a release autoloads for a name quack-rs does not list
    2  upstream could not be reached or parsed

Usage:
    python3 scripts/check-autoload-entries.py
    python3 scripts/check-autoload-entries.py --tags v1.5.5
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CATALOG_RS = REPO_ROOT / "src" / "catalog.rs"
HEADER_URL = (
    "https://raw.githubusercontent.com/duckdb/duckdb/{tag}/"
    "src/include/duckdb/main/extension_entries.hpp"
)
FLOOR = (1, 5, 0)

LISTS = {
    "EXTENSION_TYPES": "AUTOLOADING_TYPE_NAMES",
    "EXTENSION_COLLATIONS": "AUTOLOADING_COLLATION_NAMES",
}


def release_tags() -> list[str]:
    out = subprocess.run(
        ["git", "ls-remote", "--tags", "https://github.com/duckdb/duckdb"],
        check=True,
        capture_output=True,
        text=True,
        timeout=120,
    ).stdout
    tags = set()
    for line in out.splitlines():
        m = re.search(r"refs/tags/(v(\d+)\.(\d+)\.(\d+))$", line)
        if m and tuple(int(x) for x in m.group(2, 3, 4)) >= FLOOR:
            tags.add(m.group(1))
    return sorted(tags, key=lambda t: tuple(int(x) for x in t[1:].split(".")))


def upstream_names(header: str, array: str) -> set[str]:
    m = re.search(rf"{array}\[\]\s*=\s*\{{(.*?)\}};", header, re.DOTALL)
    if not m:
        raise ValueError(f"{array} not found")
    return {name.lower() for name, _ in re.findall(r'\{"([^"]+)",\s*"([^"]+)"\}', m.group(1))}


def rust_names(text: str, const: str) -> set[str]:
    m = re.search(rf"const {const}:\s*&\[&str\]\s*=\s*&\[(.*?)\];", text, re.DOTALL)
    if not m:
        raise ValueError(f"{const} not found in {CATALOG_RS}")
    return set(re.findall(r'"([^"]+)"', m.group(1)))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--tags", nargs="*", help="check only these release tags")
    args = parser.parse_args()

    text = CATALOG_RS.read_text(encoding="utf-8")
    ours = {array: rust_names(text, const) for array, const in LISTS.items()}

    try:
        tags = args.tags or release_tags()
    except (subprocess.SubprocessError, OSError) as e:
        print(f"could not list DuckDB release tags: {e}", file=sys.stderr)
        return 2
    if not tags:
        print("no DuckDB release tags found", file=sys.stderr)
        return 2

    union: dict[str, set[str]] = {array: set() for array in LISTS}
    for tag in tags:
        try:
            with urllib.request.urlopen(HEADER_URL.format(tag=tag), timeout=60) as r:
                header = r.read().decode("utf-8")
            for array in LISTS:
                names = upstream_names(header, array)
                union[array] |= names
                print(f"{tag}: {array} {len(names)} names")
        except (urllib.error.URLError, OSError, ValueError) as e:
            print(f"{tag}: could not read extension_entries.hpp: {e}", file=sys.stderr)
            return 2

    status = 0
    for array, const in LISTS.items():
        missing = sorted(union[array] - ours[array])
        extra = sorted(ours[array] - union[array])
        if missing:
            status = 1
            print(f"FAIL: {const} lacks {missing} ({array} in some supported release)")
        if extra:
            print(f"note: {const} lists {extra}, which no checked release autoloads for")
    if status == 0:
        print(f"OK: src/catalog.rs covers {len(tags)} release(s): {', '.join(tags)}")
    return status


if __name__ == "__main__":
    sys.exit(main())
