#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check quack-rs's DuckDB platform list against upstream.

`validate::platform` exists so an extension author can be told early that a name
in `description.yml`'s `excluded_platforms` is wrong. That is only worth
anything if the list is right, and it drifts silently: `DuckDB` retired
`linux_amd64_gcc4` and added `linux_amd64_musl` / `linux_arm64_musl` without
quack-rs noticing, so the validator was simultaneously rejecting real platforms
and accepting one that no longer exists.

Three lists are checked:

- `DUCKDB_CI_PLATFORMS` must equal the matrix's `duckdb_arch` values exactly.
- `DUCKDB_OPT_IN_PLATFORMS` must equal the ones the matrix marks `opt_in`.
- `DUCKDB_PLATFORM_GROUPS` must equal the matrix's top-level keys, and
  `DUCKDB_PLATFORMS` must accept every CI platform.

The authority is `config/distribution_matrix.json` in
`duckdb/extension-ci-tools` — the file the community-extensions build reads to
decide what to build.

Exit codes:
    0  the lists agree
    1  they diverge
    2  the matrix could not be fetched or parsed (network, layout change)

Usage:
    python3 scripts/check-platform-table.py
    python3 scripts/check-platform-table.py --ref v1.3.2
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PLATFORM_RS = REPO_ROOT / "src" / "validate" / "platform.rs"

MATRIX_URL = (
    "https://raw.githubusercontent.com/duckdb/extension-ci-tools/"
    "{ref}/config/distribution_matrix.json"
)


def rust_list(name: str, text: str) -> list[str] | None:
    """Extract a `pub const <name>: &[&str] = &[...]` list from platform.rs."""
    match = re.search(
        rf"pub const {name}:\s*&\[&str\]\s*=\s*&\[(.*?)\];", text, re.DOTALL
    )
    if not match:
        return None
    return re.findall(r'"([^"]+)"', match.group(1))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--ref",
        default="main",
        help="extension-ci-tools ref to check (default: main)",
    )
    args = parser.parse_args()

    text = PLATFORM_RS.read_text()
    # DUCKDB_CI_PLATFORMS is the matrix-derived list; DUCKDB_PLATFORMS is a
    # superset that also accepts names appearing in real `excluded_platforms`
    # fields (windows_amd64_rtools, and the matrix's group names).
    ours = rust_list("DUCKDB_CI_PLATFORMS", text)
    every = rust_list("DUCKDB_PLATFORMS", text)
    opt_in = rust_list("DUCKDB_OPT_IN_PLATFORMS", text)
    groups = rust_list("DUCKDB_PLATFORM_GROUPS", text)
    if ours is None or every is None or opt_in is None or groups is None:
        sys.exit(f"could not parse the platform lists out of {PLATFORM_RS}")

    url = MATRIX_URL.format(ref=args.ref)
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            matrix = json.loads(response.read().decode("utf-8"))
    except Exception as error:  # noqa: BLE001 - any failure is "could not check"
        print(f"::warning::could not fetch {url}: {error}")
        return 2

    upstream: dict[str, bool] = {}
    for group in matrix.values():
        for entry in group.get("include", []):
            arch = entry.get("duckdb_arch")
            if not arch:
                print(f"::warning::an entry in {url} has no duckdb_arch")
                return 2
            upstream[arch] = bool(entry.get("opt_in", False))

    if not upstream:
        print(f"::warning::{url} listed no architectures — layout may have changed")
        return 2

    upstream_opt_in = {arch for arch, is_opt_in in upstream.items() if is_opt_in}

    missing = sorted(set(upstream) - set(ours))
    extra = sorted(set(ours) - set(upstream))
    opt_in_missing = sorted(upstream_opt_in - set(opt_in))
    opt_in_extra = sorted(set(opt_in) - upstream_opt_in)

    # The group names come from the matrix's own top-level keys.
    upstream_groups = sorted(matrix.keys())
    group_missing = sorted(set(upstream_groups) - set(groups))
    group_extra = sorted(set(groups) - set(upstream_groups))
    # DUCKDB_PLATFORMS must accept everything the CI can build.
    not_accepted = sorted(set(ours) - set(every))

    print(f"upstream: {len(upstream)} architectures ({len(upstream_opt_in)} opt-in), "
          f"{len(upstream_groups)} groups")
    print(f"quack-rs: {len(ours)} CI platforms ({len(opt_in)} opt-in), "
          f"{len(groups)} groups, {len(every)} accepted names")

    problems = False
    for label, names in (
        ("upstream builds it, DUCKDB_CI_PLATFORMS omits it", missing),
        ("DUCKDB_CI_PLATFORMS lists it, upstream does not build it", extra),
        ("upstream marks it opt-in, quack-rs does not", opt_in_missing),
        ("quack-rs marks it opt-in, upstream does not", opt_in_extra),
        ("a matrix group name is missing from DUCKDB_PLATFORM_GROUPS", group_missing),
        ("DUCKDB_PLATFORM_GROUPS names a group upstream does not have", group_extra),
        ("a CI platform is not accepted by validate_platform", not_accepted),
    ):
        if names:
            problems = True
            print(f"\nFAIL: {label}: {', '.join(names)}")

    if problems:
        print(
            f"\nUpdate the platform lists in {PLATFORM_RS.relative_to(REPO_ROOT)} "
            f"to match {url}."
        )
        return 1

    print("\nOK: src/validate/platform.rs matches upstream")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
