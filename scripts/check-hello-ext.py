#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run hello-ext's documented SQL checks against a real DuckDB.

`examples/hello-ext/README.md` lists 31 statements with the answer each should
give once the extension is loaded. CI used to run two of them. This runs all of
`examples/hello-ext/sql_checks.txt` — the same statements, with the exact CLI
output each must produce — and also fails if the README's statements and the
fixture's ever differ, so neither can drift from the other unnoticed.

Each statement runs in a fresh `duckdb -unsigned` process that first LOADs the
extension. The script also reports how many checks would pass *without* LOAD
(DuckDB's built-ins answering), so a check that cannot fail is visible.

Exit codes:
    0  every check matched
    1  a check failed, or the README and the fixture disagree
    2  bad arguments or unreadable files

Usage:
    python3 scripts/check-hello-ext.py --duckdb ./duckdb \\
        --extension /tmp/ext/hello_ext.duckdb_extension
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
FIXTURE = REPO_ROOT / "examples" / "hello-ext" / "sql_checks.txt"
README = REPO_ROOT / "examples" / "hello-ext" / "README.md"


def normalise(sql: str) -> str:
    """Collapse whitespace, including just inside parentheses."""
    s = " ".join(sql.split())
    return re.sub(r"\(\s+", "(", re.sub(r"\s+\)", ")", s)).rstrip(";").strip()


def load_fixture() -> list[tuple[str, str, str]]:
    checks = []
    block: list[str] = []
    ident: str | None = None
    for line in FIXTURE.read_text(encoding="utf-8").splitlines() + ["== END"]:
        if line.startswith("#"):
            continue
        if line.startswith("== "):
            if ident is not None:
                text = "\n".join(block).strip()
                sql, _, expected = text.partition("\n----\n")
                checks.append((ident, sql.strip(), expected.strip()))
            ident, block = line[3:].strip(), []
        else:
            block.append(line)
    return checks


def readme_statements() -> list[str]:
    text = README.read_text(encoding="utf-8")
    start = text.index("```sql\nSET allow_extensions_metadata_mismatch=true;")
    body = text[start + len("```sql\n") :]
    body = body[: body.index("\n```")]
    code = "\n".join(re.sub(r"--.*$", "", line) for line in body.splitlines())
    stmts = [normalise(s) for s in code.split(";") if s.strip()]
    return [s for s in stmts if not s.upper().startswith(("SET ", "LOAD "))]


def run(cli: str, sql: str, extension: str | None) -> str:
    prelude = f"LOAD '{extension}';\n" if extension else ""
    proc = subprocess.run(
        [cli, "-unsigned", "-csv", "-noheader", "-nullvalue", "NULL", "-c", prelude + sql],
        capture_output=True,
        text=True,
        timeout=120,
    )
    if proc.returncode != 0 or "Error" in proc.stderr:
        first = (proc.stderr.strip().splitlines() or ["(no message)"])[0]
        return "ERROR: " + first
    lines = []
    for line in proc.stdout.strip().splitlines():
        if len(line) >= 2 and line.startswith('"') and line.endswith('"'):
            line = line[1:-1].replace('""', '"')
        lines.append(line)
    return "\n".join(lines)


def matches(got: str, expected: str) -> bool:
    if expected.startswith("ERROR: "):
        return got.startswith(expected)
    return got == expected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--duckdb", required=True, help="path to the DuckDB CLI")
    parser.add_argument("--extension", required=True, help="the .duckdb_extension to LOAD")
    args = parser.parse_args()

    try:
        checks = load_fixture()
        documented = readme_statements()
    except (OSError, ValueError) as e:
        print(f"cannot read the checks: {e}", file=sys.stderr)
        return 2

    status = 0
    fixture_sql = [normalise(sql) for _, sql, _ in checks]
    if fixture_sql != documented:
        status = 1
        print("FAIL: README.md's statements and sql_checks.txt differ:")
        for i in range(max(len(fixture_sql), len(documented))):
            a = documented[i] if i < len(documented) else "(missing)"
            b = fixture_sql[i] if i < len(fixture_sql) else "(missing)"
            if a != b:
                print(f"  README : {a}\n  fixture: {b}")

    failures = 0
    vacuous = []
    for ident, sql, expected in checks:
        got = run(args.duckdb, sql, args.extension)
        if not matches(got, expected):
            failures += 1
            print(f"FAIL {ident}: {sql}\n  expected: {expected!r}\n  got:      {got!r}")
        if matches(run(args.duckdb, sql, None), expected):
            vacuous.append(ident)
    print(f"{len(checks) - failures}/{len(checks)} checks pass with the extension loaded")
    print(
        f"{len(vacuous)} also pass without it (DuckDB's built-ins give the same answer): "
        f"{', '.join(vacuous) or 'none'}"
    )
    if failures:
        status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
