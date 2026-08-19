#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check quack-rs's MSRV against the Rust toolchains DuckDB's CI pins.

An extension published to the DuckDB community-extensions repository is built by
`duckdb/extension-ci-tools`'s reusable `_extension_distribution.yml` workflow.
That workflow pins a Rust toolchain per platform — notably an *exact* version for
the WebAssembly job, where a `dtolnay/rust-toolchain@... # 1.86.0` pin means
Cargo refuses any crate declaring a higher `rust-version`.

So quack-rs's MSRV is not just a policy choice: raise it above what that workflow
provides and every downstream extension silently stops being buildable for those
platforms, with no signal in quack-rs's own CI. This script makes that signal
exist.

Exit codes:
    0  every pinned toolchain can build quack-rs
    1  a pinned toolchain is older than quack-rs's MSRV
    2  the workflow could not be fetched or parsed (network, layout change)

Usage:
    python3 scripts/check-msrv-vs-duckdb-ci.py
    python3 scripts/check-msrv-vs-duckdb-ci.py --ref v1.3.2
"""

from __future__ import annotations

import argparse
import re
import sys
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = REPO_ROOT / "Cargo.toml"

WORKFLOW_URL = (
    "https://raw.githubusercontent.com/duckdb/extension-ci-tools/"
    "{ref}/.github/workflows/_extension_distribution.yml"
)

# `- uses: dtolnay/rust-toolchain@<sha> # <ref>` — the trailing comment is the
# convention DuckDB uses to record which ref the SHA pins.
TOOLCHAIN_PIN = re.compile(
    r"uses:\s*dtolnay/rust-toolchain@[0-9a-f]{40}\s*#\s*(?P<ref>\S+)"
)
SEMVER = re.compile(r"^(\d+)\.(\d+)(?:\.(\d+))?$")


def msrv() -> tuple[int, int, int]:
    match = re.search(r'(?m)^rust-version\s*=\s*"([^"]+)"', CARGO_TOML.read_text())
    if not match:
        sys.exit("could not find rust-version in Cargo.toml")
    return parse(match.group(1)) or sys.exit(f"unparseable rust-version {match.group(1)!r}")


def parse(version: str) -> tuple[int, int, int] | None:
    match = SEMVER.match(version)
    if not match:
        return None
    major, minor, patch = match.groups()
    return int(major), int(minor), int(patch or 0)


def render(version: tuple[int, int, int]) -> str:
    return ".".join(str(part) for part in version)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--ref",
        default="main",
        help="extension-ci-tools ref to check (default: main)",
    )
    args = parser.parse_args()

    ours = msrv()
    print(f"quack-rs MSRV: {render(ours)}")

    url = WORKFLOW_URL.format(ref=args.ref)
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            workflow = response.read().decode("utf-8")
    except Exception as error:  # noqa: BLE001 - any failure is "could not check"
        print(f"::warning::could not fetch {url}: {error}")
        return 2

    pins = TOOLCHAIN_PIN.findall(workflow)
    if not pins:
        print(
            "::warning::found no dtolnay/rust-toolchain pins in "
            f"{url} — the workflow layout may have changed"
        )
        return 2

    problems = []
    for ref in sorted(set(pins)):
        pinned = parse(ref)
        if pinned is None:
            # "stable", "nightly", a branch name — tracks current Rust, so it
            # can build any reasonable MSRV.
            print(f"  {ref:<10} floating toolchain, not a constraint")
            continue
        verdict = "ok" if pinned >= ours else "TOO OLD"
        print(f"  {ref:<10} pinned toolchain — {verdict}")
        if pinned < ours:
            problems.append(ref)

    if problems:
        print()
        for ref in problems:
            print(
                f"FAIL: extension-ci-tools pins Rust {ref}, but quack-rs requires "
                f"{render(ours)}. Extensions built with quack-rs cannot be produced "
                "for the platforms that job covers. Lower rust-version, or accept "
                "that those platforms must be listed in description.yml's "
                "excluded_platforms."
            )
        return 1

    print("\nOK: every pinned toolchain can build quack-rs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
