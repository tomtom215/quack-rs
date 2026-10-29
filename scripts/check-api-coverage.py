#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Classify every slot of DuckDB's C extension API as wrapped or deliberately not.

`duckdb_ext_api_v1` is the table of function pointers a loadable extension
calls DuckDB through (546 slots at v1.5.6, 548 at v2.0.0). Nothing used to say
which of them quack-rs wraps, so the answer was a hand count that disagreed with
itself (AUDIT.md section 5.1 records 98, 99 and 105) and a slot a new DuckDB
release added went unnoticed until someone read the header.

This script makes the answer mechanical. A slot is **wrapped** when its name
appears in the code (not a comment) of any file under `src/`. Every other slot
must be listed in `scripts/api-coverage-allowlist.txt` with a class and a reason:

    deprecated   DuckDB marks it deprecated in `duckdb.h`
    superseded   a newer call does the same job, or it is an internal helper
    host-side    only a host application (not an extension) has a use for it
    candidate    worth wrapping; not done yet

It fails when

  * a slot is neither wrapped nor listed (a new DuckDB release added it, or a
    wrapper was deleted): wrap it, or list it with a reason;
  * a listed slot is wrapped after all: delete its line, so the list stays true.

A listed name the header does not declare is ignored (the same list serves the
546-slot and the 548-slot header).

The header is fetched for the DuckDB release that `Cargo.lock`'s `libduckdb-sys`
corresponds to (`scripts/duckdb-version-from-lock.sh`), the same way
`check-abi-table.py` fetches its headers, or read from `--header`.

Exit codes:
    0  every slot is wrapped or listed, and no listed slot is wrapped
    1  the classification is wrong (see the output)
    2  the header could not be fetched -- "could not check", not a failure

Usage:
    python3 scripts/check-api-coverage.py
    python3 scripts/check-api-coverage.py --header path/to/duckdb_extension.h
    python3 scripts/check-api-coverage.py --list     # print the unwrapped slots
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = ROOT / "scripts" / "api-coverage-allowlist.txt"
CLASSES = {"deprecated", "superseded", "host-side", "candidate"}


def load_abi_table_module():
    """`check-abi-table.py` (its name is not importable) for its header parser."""
    path = ROOT / "scripts" / "check-abi-table.py"
    spec = importlib.util.spec_from_file_location("check_abi_table", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def code_only(text: str) -> str:
    """`text` with `/* */` and `//` comments (doc comments included) removed.

    A `//` inside a string literal is not told apart from a comment; the names
    searched for never sit in one after it.
    """
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", " ", text)


def wrapped_names(names: list[str]) -> set[str]:
    """The names in `names` that appear in the code of some file under `src/`."""
    wanted = set(names)
    found: set[str] = set()
    for path in sorted((ROOT / "src").rglob("*.rs")):
        code = code_only(path.read_text(encoding="utf-8"))
        found |= wanted & set(re.findall(r"\bduckdb_\w+\b", code))
    return found


def read_allowlist() -> dict[str, tuple[str, str]]:
    entries: dict[str, tuple[str, str]] = {}
    for number, line in enumerate(ALLOWLIST.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 2)
        if len(parts) != 3 or parts[1] not in CLASSES:
            sys.exit(f"{ALLOWLIST.name}:{number}: expected `name class reason`, class one of {sorted(CLASSES)}")
        name, cls, reason = parts
        if name in entries:
            sys.exit(f"{ALLOWLIST.name}:{number}: {name} is listed twice")
        entries[name] = (cls, reason)
    return entries


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--header", type=Path, help="duckdb_extension.h to read instead of fetching one")
    parser.add_argument("--list", action="store_true", help="print the slots no src/ code uses, and exit")
    args = parser.parse_args()

    abi = load_abi_table_module()
    if args.header:
        header = args.header.read_text(encoding="utf-8")
        label = str(args.header)
    else:
        tag = subprocess.run(
            ["bash", str(ROOT / "scripts" / "duckdb-version-from-lock.sh")],
            check=True, capture_output=True, text=True, cwd=ROOT,
        ).stdout.strip()
        status, header = abi.fetch(tag)
        if status != "ok":
            print(f"could not fetch duckdb_extension.h for {tag}: {status} {header or ''}; check skipped")
            return 2
        label = f"DuckDB {tag}"

    slots = abi.struct_fields(header, unstable=True, api=None)
    wrapped = wrapped_names(slots)
    unwrapped = [name for name in slots if name not in wrapped]
    if args.list:
        print("\n".join(unwrapped))
        return 0

    allowed = read_allowlist()
    problems: list[str] = []
    for name in unwrapped:
        if name not in allowed:
            problems.append(f"unclassified: {name} (wrap it, or list it in {ALLOWLIST.name} with a reason)")
    for name, (cls, _) in allowed.items():
        if name in wrapped:
            problems.append(f"stale: {name} is listed as {cls} but src/ uses it; delete its line")

    counts = {cls: sum(1 for n in unwrapped if allowed.get(n, ("",))[0] == cls) for cls in sorted(CLASSES)}
    print(
        f"{label}: {len(slots)} slots, {len(wrapped)} wrapped, {len(unwrapped)} not "
        + ", ".join(f"{n} {cls}" for cls, n in counts.items())
    )
    for problem in problems:
        print(f"  {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
