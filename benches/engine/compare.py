#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Time quack-rs-written scalar functions in a real DuckDB, before and after a change.

Builds this directory's benchmark extension against two quack-rs source trees
(the same benchmark source for both, so the only difference is the SDK), stamps
both with `append_metadata`, creates the data, checks that the two trees return
the same results, and times each query interleaved: base, new and the
hand-written `r_*` twin, in alternating order across sessions, so drift in the
machine's load hits all three alike.

    python3 benches/engine/compare.py --base ../quack-rs-0.19.0 --new . \\
        --duckdb /path/to/duckdb --workdir /tmp/quack-bench

Each `--base`/`--new` is a quack-rs checkout (`git worktree add ../base v0.19.0`
makes one). Each session runs one query `repeats + 1` times in one `duckdb`
process with `SET threads=1` and discards the first run (the table is read cold).
Reported per variant: median, minimum and maximum wall time of all timed runs and
the median per row, plus the median's distance from the hand-written twin. Read
the spread before the difference: a delta inside it is noise.

Needs Linux (the `.so` and `linux_amd64` stamp), a Rust toolchain, and a DuckDB CLI
of a release whose C API the build is stamped for (`--duckdb-version`, v1.2.0 by
default, loads in every release since 1.2.0).
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUN_TIME = re.compile(r"Run Time \(s\): real ([\d.]+) user ([\d.]+) sys ([\d.]+)")

# (title, quack-rs query, hand-written query, rows)
CASES = [
    ("nullable BIGINT, map1 (10% NULL)", "SELECT sum(q_inc(x)) FROM t_n", "SELECT sum(r_inc(x)) FROM t_n", "t_n"),
    ("nullable BIGINT, map2 (10% NULL)", "SELECT sum(q_add(x, x)) FROM t_n", "SELECT sum(r_add(x, x)) FROM t_n", "t_n"),
    ("non-NULL BIGINT, map1 (control)", "SELECT sum(q_inc(x)) FROM t_i", "SELECT sum(r_inc(x)) FROM t_i", "t_i"),
    ("VARCHAR -> BIGINT, short strings", "SELECT sum(q_len(s)) FROM t_ss", "SELECT sum(r_len(s)) FROM t_ss", "t_ss"),
    ("VARCHAR -> BIGINT, long strings", "SELECT sum(q_len(s)) FROM t_sl", "SELECT sum(r_len(s)) FROM t_sl", "t_sl"),
    ("VARCHAR -> VARCHAR trim, short", "SELECT sum(length(q_trim(s))) FROM t_ss", "SELECT sum(length(r_trim(s))) FROM t_ss", "t_ss"),
    ("BIGINT -> [x, x+1, x+2]", "SELECT sum(len(q_list3(x))) FROM t_i", "SELECT sum(len(r_list3_grow(x))) FROM t_i", "t_i"),
    ("BIGINT -> [x, x+1, x+2, NULL]", "SELECT sum(len(q_list4n(x))) FROM t_i", "SELECT sum(len(r_list4n(x))) FROM t_i", "t_i"),
]


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)


def build(tree: Path, label: str, work: Path, target: Path) -> Path:
    """Build the benchmark extension against `tree`; return the unstamped library."""
    crate = work / f"crate-{label}"
    shutil.rmtree(crate, ignore_errors=True)
    shutil.copytree(HERE, crate, ignore=shutil.ignore_patterns("target", "Cargo.lock", "__pycache__"))
    manifest = crate / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    manifest.write_text(text.replace('path = "../.."', f'path = "{tree.resolve()}"'), encoding="utf-8")
    env = {**os.environ, "CARGO_TARGET_DIR": str(target)}
    print(f"building {label}: {tree}", flush=True)
    run(["cargo", "build", "--release", "--manifest-path", str(manifest)], env=env)
    library = work / f"libbenchext-{label}.so"
    shutil.copy2(target / "release" / "libbenchext.so", library)
    return library


def stamp(tool: Path, library: Path, label: str, work: Path, duckdb_version: str) -> Path:
    # The file name decides the entry point DuckDB looks for: `benchext_init_c_api`.
    out = work / label / "benchext.duckdb_extension"
    out.parent.mkdir(parents=True, exist_ok=True)
    run([str(tool), str(library), str(out), "--abi-type", "C_STRUCT",
         "--duckdb-version", duckdb_version, "--extension-version", "v0.0.0"])
    return out


def make_data(duckdb: str, db: Path, rows: int) -> None:
    if db.exists():
        return
    long_rows = rows // 2
    sql = f"""
CREATE TABLE t_n AS SELECT CASE WHEN hash(i) % 10 = 0 THEN NULL ELSE i END AS x FROM range({rows}) t(i);
CREATE TABLE t_i AS SELECT i AS x FROM range({rows}) t(i);
CREATE TABLE t_ss AS SELECT repeat(' ', (i % 3)::INTEGER) || (i % 100000)::VARCHAR || repeat(' ', (i % 2)::INTEGER) AS s FROM range({rows}) t(i);
CREATE TABLE t_sl AS SELECT '   ' || repeat('abcdefghij', 4) || (i % 100000)::VARCHAR || '  ' AS s FROM range({long_rows}) t(i);
CHECKPOINT;
"""
    run([duckdb, str(db)], input=sql)


def table_rows(duckdb: str, db: Path) -> dict[str, int]:
    out = run([duckdb, "-readonly", str(db), "-noheader", "-list", "-c",
               "SELECT 't_n', count(*) FROM t_n UNION ALL SELECT 't_i', count(*) FROM t_i "
               "UNION ALL SELECT 't_ss', count(*) FROM t_ss UNION ALL SELECT 't_sl', count(*) FROM t_sl"]).stdout
    return {line.split("|")[0]: int(line.split("|")[1]) for line in out.split() if "|" in line}


def session(duckdb: str, db: Path, ext: Path, sql: str, repeats: int) -> list[float]:
    script = [".mode trash", f"LOAD '{ext}';", "SET threads=1;", ".timer on"] + [sql + ";"] * (repeats + 1)
    out = run([duckdb, "-unsigned", "-readonly", str(db)], input="\n".join(script))
    runs = [float(real) for real, _user, _sys in RUN_TIME.findall(out.stdout + out.stderr)]
    if len(runs) < repeats + 1:
        raise SystemExit(f"{sql!r}: expected {repeats + 1} timings, got {len(runs)}:\n{out.stdout[-500:]}{out.stderr[-500:]}")
    return runs[1:]  # the first run reads the table cold


def answer(duckdb: str, db: Path, ext: Path, sql: str) -> str:
    out = subprocess.run([duckdb, "-unsigned", "-readonly", str(db), "-noheader", "-list", "-c",
                          f"LOAD '{ext}'; {sql}"], capture_output=True, text=True)
    return (out.stdout + out.stderr).strip()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", type=Path, required=True, help="quack-rs checkout to compare against")
    ap.add_argument("--new", type=Path, required=True, help="quack-rs checkout with the change")
    ap.add_argument("--duckdb", default="duckdb", help="DuckDB CLI to time with")
    ap.add_argument("--workdir", type=Path, required=True, help="scratch directory (builds, data); reused if it exists")
    ap.add_argument("--target-dir", type=Path, help="cargo target dir (default: WORKDIR/target)")
    ap.add_argument("--rows", type=int, default=20_000_000, help="rows in the largest table (default 20M)")
    ap.add_argument("--sessions", type=int, default=6, help="interleaved sessions per variant (default 6)")
    ap.add_argument("--repeats", type=int, default=3, help="timed runs per session, after a discarded one (default 3)")
    ap.add_argument("--duckdb-version", default="v1.2.0", help="C API version to stamp (default v1.2.0)")
    ap.add_argument("--only", help="run only the cases whose title contains this text")
    ap.add_argument("--check", action="store_true", help="only compare results between the two builds")
    args = ap.parse_args()

    work = args.workdir.resolve()
    work.mkdir(parents=True, exist_ok=True)
    target = (args.target_dir or work / "target").resolve()
    libs = {label: build(tree, label, work, target) for label, tree in (("base", args.base), ("new", args.new))}
    print("building append_metadata", flush=True)
    run(["cargo", "build", "--release", "--bin", "append_metadata", "--manifest-path", str(args.new.resolve() / "Cargo.toml")],
        env={**os.environ, "CARGO_TARGET_DIR": str(target)})
    tool = target / "release" / "append_metadata"
    ext = {label: stamp(tool, lib, label, work, args.duckdb_version) for label, lib in libs.items()}

    db = work / "data.duckdb"
    make_data(args.duckdb, db, args.rows)
    rows = table_rows(args.duckdb, db)

    cases = [c for c in CASES if not args.only or args.only in c[0]]
    bad = 0
    for title, q, r, _table in cases:
        a, b = answer(args.duckdb, db, ext["base"], q), answer(args.duckdb, db, ext["new"], q)
        status = "same" if a == b and a and "Error" not in a else "DIFFERENT"
        bad += status != "same"
        print(f"{status:9s} {title}: base={a!r} new={b!r}")
    if bad:
        print(f"{bad} case(s) differ between the builds; timings would compare different work")
        return 1
    if args.check:
        return 0

    print(f"\nthreads=1, {args.sessions} sessions x {args.repeats} timed runs per variant, interleaved; "
          f"load average at start {os.getloadavg()[0]:.2f}")
    for title, q, r, table in cases:
        n = rows[table]
        variants = {"base": (q, ext["base"]), "new": (q, ext["new"]), "hand-written": (r, ext["new"])}
        samples: dict[str, list[float]] = {k: [] for k in variants}
        for s in range(args.sessions):
            for k in (list(variants) if s % 2 == 0 else list(reversed(list(variants)))):
                sql, path = variants[k]
                samples[k] += session(args.duckdb, db, path, sql, args.repeats)
        ref = statistics.median(samples["hand-written"])
        print(f"\n### {title} ({n:,} rows)")
        print(f"{'variant':14s} {'median':>8s} {'min':>8s} {'max':>8s} {'n':>3s} {'ns/row':>8s} {'vs hand-written':>16s}")
        for k, v in samples.items():
            med = statistics.median(v)
            print(f"{k:14s} {med:8.3f} {min(v):8.3f} {max(v):8.3f} {len(v):3d} {med / n * 1e9:8.2f} {(med - ref) / n * 1e9:+13.2f} ns")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    start = time.time()
    code = main()
    print(f"\ndone in {time.time() - start:.0f} s")
    sys.exit(code)
