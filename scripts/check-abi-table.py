#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify `src/abi.rs`'s DuckDB ABI layout table against upstream headers.

The `duckdb_ext_api_v1` struct that DuckDB hands to a loadable extension is an
array of function pointers. Its *stable* prefix has been frozen since DuckDB
v1.2.0, but the *unstable* remainder gains entries in the middle between
releases, which shifts every later slot. quack-rs pins a verified
`DuckDB release -> slot count` table in `src/abi.rs` so a layout mismatch is
caught at LOAD time instead of mis-dispatching.

This script re-derives that table straight from `src/include/duckdb_extension.h`
at each release tag and fails if `src/abi.rs` has drifted or a new release is
missing.

The release tags are enumerated from upstream at run time (`git ls-remote
--tags`), not from a list kept in this file: a hard-coded list only ever checks
the releases someone already knew about, so a new DuckDB release went unnoticed
(v1.4.5 shipped and was absent from both the list and the table). Every tag of
the form `vX.Y.Z` at or above v1.2.0 -- the first release with a
`duckdb_ext_api_v1` -- is checked; pre-release tags (`v1.6.0-rc1`, ...) are not
releases and are skipped.

A fetch that fails is **not** evidence that a release does not exist. Treating
it as one silently narrows the derived table and then reports `src/abi.rs` as
stale -- advice that, if followed, would shrink the layout table and make the
runtime guard refuse DuckDB versions it should accept. So a tag that cannot be
downloaded suspends the staleness comparison (exit 2) rather than failing it.

Exit codes:
    0  src/abi.rs matches upstream
    1  it has genuinely drifted, or a header is inconsistent
    2  the release tags could not be listed, or at least one release header
       could not be fetched, so the comparison would be unsound -- treat as
       "could not check", not as a failure

Usage:
    python3 scripts/check-abi-table.py              # verify
    python3 scripts/check-abi-table.py --print      # print the Rust table
    python3 scripts/check-abi-table.py --tags v1.5.6 v1.6.0
"""

from __future__ import annotations

import argparse
import hashlib
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
ABI_RS = REPO_ROOT / "src" / "abi.rs"

HEADER_URL = "https://raw.githubusercontent.com/duckdb/duckdb/{tag}/src/include/duckdb_extension.h"
UPSTREAM_GIT = "https://github.com/duckdb/duckdb.git"

# The first release whose `duckdb_extension.h` defines `duckdb_ext_api_v1`.
FIRST_RELEASE = (1, 2, 0)
# A release tag. Anything with a suffix (`-rc1`, `-dev`) is not a release.
RELEASE_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")

FN_PTR = re.compile(r"\(\s*\*\s*(duckdb_\w+)\s*\)")


def fetch(tag: str, *, attempts: int = 3) -> tuple[str, str | None]:
    """Download `duckdb_extension.h` for a release tag.

    Returns `("ok", header)`, `("missing", None)` for a definite 404, or
    `("error", reason)` for anything else. The distinction matters: a 404 for a
    version that does not exist yet is routine, while a timeout or a 5xx says
    nothing about whether the release exists, and must not be allowed to shrink
    the derived table.
    """
    last = "unknown error"
    for attempt in range(attempts):
        try:
            with urllib.request.urlopen(HEADER_URL.format(tag=tag), timeout=60) as resp:
                return "ok", resp.read().decode("utf-8")
        except urllib.error.HTTPError as err:
            if err.code == 404:
                return "missing", None
            last = f"HTTP {err.code}"
        except Exception as err:  # noqa: BLE001 - network, DNS, TLS, timeouts
            last = f"{type(err).__name__}: {err}"
        if attempt + 1 < attempts:
            time.sleep(2 * (attempt + 1))
    return "error", last


def normalise(declaration: str) -> str:
    """A declaration with formatting that cannot change the ABI removed:
    comments, runs of whitespace, spaces next to punctuation, and `(void)`
    parameter lists, which C treats as `()` here (DuckDB `main` rewrote every
    `()` as `(void)` without changing a signature)."""
    text = re.sub(r"/\*.*?\*/", " ", declaration, flags=re.S)
    text = re.sub(r"//[^\n]*", " ", text)
    text = re.sub(r"\s+", " ", text).strip()
    text = re.sub(r"\s*([(),*;])\s*", r"\1", text)
    return text.replace("(void)", "()")


def struct_declarations(header: str, *, unstable: bool) -> list[str]:
    """Ordered, normalised function-pointer declarations in `duckdb_ext_api_v1`.

    The layout fingerprint is taken over these rather than over the names
    alone, so a release that changes a signature but keeps every name is
    caught too.
    """
    start = header.index("typedef struct {")
    end = header.index("} duckdb_ext_api_v1;")
    decls: list[str] = []
    buf = ""
    stack: list[bool] = []
    for line in header[start:end].split("\n"):
        stripped = line.strip()
        if stripped.startswith("#ifdef DUCKDB_EXTENSION_API_VERSION_UNSTABLE"):
            stack.append(unstable)
            continue
        if stripped.startswith("#if"):
            stack.append(True)
            continue
        if stripped.startswith("#endif"):
            if stack:
                stack.pop()
            continue
        if stripped.startswith("#"):
            continue
        if not all(stack):
            continue
        buf += "\n" + stripped
        if buf.strip().endswith(";"):
            if FN_PTR.search(buf):
                decls.append(normalise(buf))
            buf = ""
    return decls


def struct_fields(header: str, *, unstable: bool) -> list[str]:
    """Ordered function-pointer names in `duckdb_ext_api_v1`.

    `unstable` mirrors whether `DUCKDB_EXTENSION_API_VERSION_UNSTABLE` is
    defined. libduckdb-sys defines it when generating the loadable-extension
    bindings, and DuckDB defines it when building the struct it hands out, so
    `unstable=True` is the layout that actually matters at runtime.
    """
    start = header.index("typedef struct {")
    end = header.index("} duckdb_ext_api_v1;")
    fields: list[str] = []
    buf = ""
    stack: list[bool] = []
    for line in header[start:end].split("\n"):
        stripped = line.strip()
        if stripped.startswith("#ifdef DUCKDB_EXTENSION_API_VERSION_UNSTABLE"):
            stack.append(unstable)
            continue
        if stripped.startswith("#if"):
            stack.append(True)
            continue
        if stripped.startswith("#endif"):
            if stack:
                stack.pop()
            continue
        if stripped.startswith("#"):
            continue
        if not all(stack):
            continue
        buf += " " + stripped
        if buf.strip().endswith(";"):
            match = FN_PTR.search(buf)
            if match:
                fields.append(match.group(1))
            buf = ""
    return fields


def release_tags(*, attempts: int = 3) -> list[str] | None:
    """Every upstream release tag `vX.Y.Z` >= FIRST_RELEASE.

    Returns None when the tag list could not be read, which must be reported
    as "could not check" -- never mistaken for "there are no releases".
    """
    last = "unknown error"
    for attempt in range(attempts):
        try:
            out = subprocess.run(
                ["git", "ls-remote", "--tags", "--refs", UPSTREAM_GIT],
                check=True, capture_output=True, text=True, timeout=120,
            ).stdout
        except (OSError, subprocess.SubprocessError) as err:
            last = f"{type(err).__name__}: {err}"
        else:
            tags = []
            for line in out.splitlines():
                tag = line.split("\t", 1)[-1].replace("refs/tags/", "", 1)
                match = RELEASE_TAG.match(tag)
                if match and tuple(int(g) for g in match.groups()) >= FIRST_RELEASE:
                    tags.append(tag)
            if tags:
                return tags
            last = "git ls-remote listed no release tags"
        if attempt + 1 < attempts:
            time.sleep(2 * (attempt + 1))
    print(f"  could not list upstream release tags ({last})")
    return None


def version_key(tag: str) -> tuple[int, ...]:
    return tuple(int(p) for p in tag.lstrip("v").split("."))


def collapse(rows: list[tuple[tuple[int, int, int], int]]) -> list[tuple[int, int, int, int, int]]:
    """Collapse per-release rows into (major, minor, patch_lo, patch_hi, slots)."""
    out: list[list[int]] = []
    for (major, minor, patch), slots in rows:
        if out and out[-1][0] == major and out[-1][1] == minor \
                and out[-1][4] == slots and out[-1][3] + 1 == patch:
            out[-1][3] = patch
        else:
            out.append([major, minor, patch, patch, slots])
    return [tuple(row) for row in out]  # type: ignore[misc]


def parse_rust_table() -> tuple[list[tuple[int, int, int, int, int]], int]:
    """Read KNOWN_LAYOUTS and STABLE_API_SLOT_COUNT out of `src/abi.rs`."""
    text = ABI_RS.read_text()

    stable_match = re.search(r"pub const STABLE_API_SLOT_COUNT: usize = (\d+);", text)
    if not stable_match:
        sys.exit("could not find STABLE_API_SLOT_COUNT in src/abi.rs")

    body_match = re.search(r"KNOWN_LAYOUTS: &\[LayoutEntry\] = &\[(.*?)\];", text, re.S)
    if not body_match:
        sys.exit("could not find KNOWN_LAYOUTS in src/abi.rs")

    entries = [
        tuple(int(n) for n in row)
        for row in re.findall(r"\(\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+)\s*\)", body_match.group(1))
    ]
    return entries, int(stable_match.group(1))  # type: ignore[return-value]


def render(entries: list[tuple[int, int, int, int, int]]) -> str:
    lines = ["const KNOWN_LAYOUTS: &[LayoutEntry] = &["]
    for major, minor, lo, hi, slots in entries:
        lines.append(f"    ({major}, {minor}, {lo}, {hi}, {slots}),")
    lines.append("];")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--print", action="store_true", help="print the derived Rust table")
    parser.add_argument(
        "--tags", nargs="*", default=None,
        help="release tags to check (default: every upstream vX.Y.Z >= v1.2.0)",
    )
    args = parser.parse_args()

    listed = args.tags or release_tags()
    if listed is None:
        print("::warning::could not list DuckDB release tags; KNOWN_LAYOUTS was not checked")
        return 2
    tags = sorted(set(listed), key=version_key)
    print(f"checking {len(tags)} release(s): {tags[0]} .. {tags[-1]}")

    rows: list[tuple[tuple[int, int, int], int]] = []
    stable_counts: set[int] = set()
    layout_by_slots: dict[int, str] = {}
    problems: list[str] = []
    unfetchable: list[str] = []

    for tag in tags:
        status, header = fetch(tag)
        if status == "missing":
            print(f"  {tag}: not published (skipped)")
            continue
        if status == "error":
            print(f"  {tag}: COULD NOT FETCH ({header})")
            unfetchable.append(tag)
            continue
        full = struct_fields(header, unstable=True)
        stable = struct_fields(header, unstable=False)
        # Over whole declarations, not names: two layouts with the same slot
        # count and names but a changed signature must not look the same.
        declarations = struct_declarations(header, unstable=True)
        if [FN_PTR.search(d).group(1) for d in declarations] != full:
            problems.append(f"{tag}: declarations and field names disagree")
        digest = hashlib.sha256("\n".join(declarations).encode()).hexdigest()[:12]

        if full[: len(stable)] != stable:
            problems.append(f"{tag}: the stable prefix is not a prefix of the full struct")
        stable_counts.add(len(stable))

        previous = layout_by_slots.setdefault(len(full), digest)
        if previous != digest:
            problems.append(
                f"{tag}: slot count {len(full)} is shared by two different layouts "
                f"({previous} vs {digest}) — the slot count is no longer a safe layout fingerprint"
            )

        rows.append((version_key(tag), len(full)))  # type: ignore[arg-type]
        print(f"  {tag}: {len(full)} slots ({len(stable)} stable) layout={digest}")

    if not rows:
        print("::warning::no release headers could be downloaded")
        return 2

    derived = collapse(rows)

    if args.print:
        print()
        print(render(derived))
        return 0

    table, stable_const = parse_rust_table()

    if len(stable_counts) != 1:
        problems.append(f"stable prefix size is not constant across releases: {sorted(stable_counts)}")
    elif stable_const not in stable_counts:
        problems.append(
            f"STABLE_API_SLOT_COUNT is {stable_const} but upstream headers say {stable_counts.pop()}"
        )

    if table != derived:
        if unfetchable:
            # The derivation is missing releases, so it cannot be compared: a
            # gap makes the derived ranges narrower than reality, and "fixing"
            # src/abi.rs to match would weaken the runtime guard.
            print()
            print(
                "::warning::could not verify KNOWN_LAYOUTS -- "
                f"{len(unfetchable)} release header(s) failed to download: "
                f"{', '.join(unfetchable)}"
            )
            print(f"  in src/abi.rs: {table}")
            print(f"  derived from what downloaded: {derived}")
            print("  Not treating this as drift. Re-run when upstream is reachable.")
            return 2
        problems.append(
            "KNOWN_LAYOUTS in src/abi.rs is out of date.\n"
            f"  in src/abi.rs: {table}\n"
            f"  derived:       {derived}\n"
            f"Replace it with:\n\n{render(derived)}"
        )

    if problems:
        print()
        for problem in problems:
            print(f"FAIL: {problem}")
        return 1

    print(f"\nOK: src/abi.rs matches upstream ({len(derived)} layout families, "
          f"{stable_const} stable slots)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
