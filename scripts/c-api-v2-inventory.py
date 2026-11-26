#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Inventory of DuckDB's C extension API v2, one row per function-pointer slot.

quack-rs targets C API v1 (`duckdb_extension.h`); `duckdb_extension_v2.h` is a
separate, version-exact API that an extension uses instead of v1, never beside
it. This script lists every slot of the v2 table so that a v2 backend can be
planned against the header rather than a summary of it
(`docs/c-api-v2/design.md`).

Columns:

    symbol        the slot's name, e.g. duckdb_v2_scalar_function_set_name
    family        the handle type of its first parameter
                  (`duckdb_v2_<family>_handle` -> <family>), or, when the first
                  parameter is not a handle, the name's prefix up to the first
                  family seen by handle (marked `(by name)`)
    returns       the declared return type
    v1_name       `duckdb_<rest>` when the v1 header has a slot of that name
                  (same name is not same contract: a starting point only)
    v1_in_quack   `yes` when quack-rs's library code names that v1 slot, with
                  the meaning `scripts/check-api-coverage.py` gives "used"
    disposition   `unclassified` for every row: the classification is the
                  design work, not something this script can derive

The slot set is checked against the header's `DUCKDB_C_API` declarations in
`duckdb_v2.h`: the two must name the same functions, or the script exits 1.

Usage:
    python3 scripts/c-api-v2-inventory.py --include DIR [--out FILE]
DIR holds `duckdb_extension.h`, `duckdb_extension_v2.h` and `duckdb_v2.h`
(the release's include directory, or the `libduckdb` release archive).
"""

import argparse
import csv
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

SLOT = re.compile(r"\(\s*\*\s*(duckdb_v2_\w+)\s*\)\s*\(([^;]*?)\)\s*;", re.S)
V1_SLOT = re.compile(r"\(\s*\*\s*(duckdb_\w+)\s*\)")
DECL = re.compile(r"\bDUCKDB_C_API\s+([^;{}]+?)\b(duckdb_v2_\w+)\s*\(", re.S)
HANDLE = re.compile(r"^\s*(?:const\s+)?duckdb_v2_(\w+?)_handle\b")


def coverage_scan():
    spec = importlib.util.spec_from_file_location("coverage", ROOT / "scripts" / "check-api-coverage.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # type: ignore[union-attr]
    used, _ = module.scan(ROOT / "src")
    return used


def inventory(include: Path) -> list[dict[str, str]]:
    v2 = (include / "duckdb_extension_v2.h").read_text(encoding="utf-8")
    decl_text = (include / "duckdb_v2.h").read_text(encoding="utf-8")
    v1 = (include / "duckdb_extension.h").read_text(encoding="utf-8")

    slots = {name: params for name, params in SLOT.findall(v2)}
    returns = {name: " ".join(ret.split()) for ret, name in DECL.findall(decl_text)}
    if set(slots) != set(returns):
        only_slots = sorted(set(slots) - set(returns))
        only_decls = sorted(set(returns) - set(slots))
        sys.exit(f"slot and declaration sets differ: slots only {only_slots[:5]}, declarations only {only_decls[:5]}")
    v1_slots = set(V1_SLOT.findall(v1))
    used = coverage_scan()

    families = set()
    by_handle = {}
    for name, params in slots.items():
        match = HANDLE.match(params.split(",")[0])
        if match:
            by_handle[name] = match.group(1)
            families.add(match.group(1))

    rows = []
    for name in sorted(slots):
        family = by_handle.get(name)
        if family is None:
            rest = name[len("duckdb_v2_"):]
            prefix = max((f for f in families if rest.startswith(f)), key=len, default=rest.split("_")[0])
            family = f"{prefix} (by name)"
        v1_name = "duckdb_" + name[len("duckdb_v2_"):]
        has_v1 = v1_name in v1_slots
        rows.append({
            "symbol": name,
            "family": family,
            "returns": returns[name],
            "v1_name": v1_name if has_v1 else "",
            "v1_in_quack": "yes" if has_v1 and v1_name in used else "",
            "disposition": "unclassified",
        })
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--include", type=Path, required=True, help="directory with the three headers")
    parser.add_argument("--out", type=Path, help="CSV to write (default: stdout)")
    args = parser.parse_args()
    rows = inventory(args.include)
    out = args.out.open("w", newline="", encoding="utf-8") if args.out else sys.stdout
    writer = csv.DictWriter(out, fieldnames=list(rows[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)
    if args.out:
        out.close()
    families = {r["family"] for r in rows}
    print(f"{len(rows)} slots, {len(families)} families, "
          f"{sum(1 for r in rows if r['v1_name'])} with a v1 slot of the same name, "
          f"{sum(1 for r in rows if r['v1_in_quack'])} of those used by quack-rs", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
