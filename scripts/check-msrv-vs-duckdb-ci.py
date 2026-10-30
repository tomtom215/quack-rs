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

By default it checks `main`, the branch the community build uses for the
newest DuckDB release (DISTRIBUTION_REF, kept equal to
`scaffold::DISTRIBUTION_DUCKDB_VERSION`), and the branches that build the
DuckDB 1.x releases still in support (SUPPORTED_1X_REFS). The last are the ones
that constrain the MSRV: `main` and `v2.0-cyanoptera` pin Rust 1.97.1, and
`v1.5-variegata` and `v1.4-andium` still pin 1.86.0 for their WebAssembly
build, so a default run that checked only the first two could not fail.

Usage:
    python3 scripts/check-msrv-vs-duckdb-ci.py
    python3 scripts/check-msrv-vs-duckdb-ci.py --ref v1.3.2 [--ref main]
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

# The branch the community build uses for the newest DuckDB release (v2.0.0:
# duckdb/community-extensions build.yml; extension-ci-tools has no v2.0.0 branch).
DISTRIBUTION_REF = "v2.0-cyanoptera"

# The extension-ci-tools branches for the DuckDB 1.x lines still supported
# (1.5 until March 2027, 1.4 until November 2026). Drop one when DuckDB ends
# support for its line.
SUPPORTED_1X_REFS = ["v1.5-variegata", "v1.4-andium"]

# Three forms occur upstream, and the pattern must accept all of them: an
# exact ref (`dtolnay/rust-toolchain@1.86.0`, how the WebAssembly job pins the
# toolchain that constrains the MSRV), a SHA with the ref in a trailing comment
# (`@<sha> # 1.97.1`), and either one behind a YAML anchor
# (`uses: &rust-toolchain-action dtolnay/...`). The first version of this
# script matched only the SHA form without an anchor, so once upstream used an
# anchor it found nothing, and it never saw the `@1.86.0` pin at all.
TOOLCHAIN_PIN = re.compile(
    r"uses:\s*(?:&[\w-]+\s+)?dtolnay/rust-toolchain@"
    r"(?:[0-9a-f]{40}[ \t]*(?:#[ \t]*(?P<commented>[^\n]*))?|(?P<ref>[\w.-]+))"
)
# A SHA pin's comment names its toolchain in any of these styles: `# 1.86.0`,
# `# v1.86.0`, `# rust 1.86.0`. Read only the first word, a `v1.86.0` became
# `v1.86.0` (not a version) and `rust 1.86.0` became `rust`, both "floating":
# an upstream switch to either style would have silenced this guard.
COMMENT_VERSION = re.compile(r"(?<![\w.])v?(\d+\.\d+(?:\.\d+)?)(?![\w.])")
# The `toolchain:` input that a pin of the action's `master` branch requires
# (the style quack-rs's own workflows use): `@<sha> # master` then
# `toolchain: 1.86.0`. Only the input says which Rust that is.
TOOLCHAIN_INPUT = re.compile(r"(?m)^[ \t]*toolchain:[ \t]*[\"']?(?P<value>[^\"'\s#]+)")

SELF_TEST = {
    "        uses: dtolnay/rust-toolchain@1.86.0": ["1.86.0"],
    "        uses: dtolnay/rust-toolchain@stable": ["stable"],
    "        uses: &rust-toolchain-action dtolnay/rust-toolchain@"
    "46511b1c83438f0dd37c02d843619ece5a4abb5b # 1.97.1": ["1.97.1"],
    "      - uses: dtolnay/rust-toolchain@"
    "e2a55d2ffb04f378e9626c28d38b36d230d1e12f # master 2026-10-08": ["master"],
    "      - uses: dtolnay/rust-toolchain@"
    "46511b1c83438f0dd37c02d843619ece5a4abb5b # v1.86.0": ["1.86.0"],
    "      - uses: dtolnay/rust-toolchain@"
    "46511b1c83438f0dd37c02d843619ece5a4abb5b # rust 1.86.0": ["1.86.0"],
    "      - uses: dtolnay/rust-toolchain@46511b1c83438f0dd37c02d843619ece5a4abb5b": ["(no comment)"],
    "      - uses: dtolnay/rust-toolchain@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # master\n"
    "        with:\n          toolchain: 1.86.0": ["master", "1.86.0"],
    "        with:\n          toolchain: \"1.85\"  # wasm": ["1.85"],
}


def pin_of(match: re.Match[str]) -> str:
    if match.group("ref") is not None:
        return match.group("ref")
    comment = (match.group("commented") or "").strip()
    version = COMMENT_VERSION.search(comment)
    if version:
        return version.group(1)
    return comment.split()[0] if comment else "(no comment)"


def pins_in(workflow: str) -> list[str]:
    found = [(m.start(), pin_of(m)) for m in TOOLCHAIN_PIN.finditer(workflow)]
    found += [(m.start(), m.group("value")) for m in TOOLCHAIN_INPUT.finditer(workflow)]
    return [pin for _, pin in sorted(found)]
SEMVER = re.compile(r"^(\d+)\.(\d+)(?:\.(\d+))?$")


def msrv() -> tuple[int, int, int]:
    match = re.search(r'(?m)^rust-version\s*=\s*"([^"]+)"', CARGO_TOML.read_text())
    if not match:
        sys.exit("could not find rust-version in Cargo.toml")
    return parse(match.group(1)) or sys.exit(f"unparseable rust-version {match.group(1)!r}")


def parse(version: str) -> tuple[int, int, int] | None:
    match = SEMVER.match(version.removeprefix("v"))
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
        action="append",
        help=(
            "extension-ci-tools ref to check; repeatable "
            f"(default: main, {DISTRIBUTION_REF} and {', '.join(SUPPORTED_1X_REFS)})"
        ),
    )
    args = parser.parse_args()
    refs = args.ref or ["main", DISTRIBUTION_REF, *SUPPORTED_1X_REFS]

    for line, want in SELF_TEST.items():
        got = pins_in(line)
        if got != want:
            print(f"self-test failed: {line.strip()!r} gave {got}, expected {want!r}")
            return 1
    # A pin that names a release is a constraint, however it is spelled.
    for text, want in {"1.86.0": (1, 86, 0), "v1.86.0": (1, 86, 0), "1.85": (1, 85, 0), "stable": None}.items():
        if parse(text) != want:
            print(f"self-test failed: parse({text!r}) gave {parse(text)}, expected {want}")
            return 1

    ours = msrv()
    print(f"quack-rs MSRV: {render(ours)}")

    pins: list[str] = []
    for ref in refs:
        url = WORKFLOW_URL.format(ref=ref)
        try:
            with urllib.request.urlopen(url, timeout=60) as response:
                workflow = response.read().decode("utf-8")
        except Exception as error:  # noqa: BLE001 - any failure is "could not check"
            print(f"::warning::could not fetch {url}: {error}")
            return 2
        found = pins_in(workflow)
        if not found:
            print(
                "::warning::found no dtolnay/rust-toolchain pins in "
                f"{url} — the workflow layout may have changed"
            )
            return 2
        print(f"{ref}: {', '.join(sorted(set(found)))}")
        pins.extend(found)

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
