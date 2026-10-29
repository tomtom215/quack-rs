<!-- SPDX-License-Identifier: MIT -->
# Engine-level benchmark

`benches/interval_bench.rs` times a pure function; nothing in it touches a
vector, a validity mask or a callback. This crate does: a loadable extension
whose functions are written twice, with quack-rs (`q_*`) and by hand against
`libduckdb-sys` (`r_*`), and a driver that times them inside a real DuckDB.

It answers "what does the SDK cost over hand-written code, and did this change
move it?", with an A/B on one machine:

```bash
git worktree add ../quack-base <the revision to compare against>
python3 benches/engine/compare.py --base ../quack-base --new . \
    --duckdb /path/to/duckdb --workdir /tmp/quack-bench
```

`compare.py` builds the benchmark extension against both trees (the same
benchmark source, so only the SDK differs), stamps both with `append_metadata`,
creates the tables, checks that both builds return the same results, and times
each query with `SET threads=1`, interleaving base, new and the hand-written twin
across sessions. Add `--check` to stop after the equality check.

| Query | What it exercises |
|---|---|
| `q_inc`, `q_add` on a nullable column | `map1` / `map2`: `VectorReader::is_valid`, `VectorWriter::set_null` |
| `q_inc` on a column with no NULLs | the same without a validity mask (a control: it should not move) |
| `q_len` on short and long strings | `read_str`: UTF-8 validation of inline and out-of-line strings |
| `q_trim` | `map1_str` returning an owned `String`: one allocation per row |
| `q_list3`, `q_list4n` | `ListBuilder`, without and with a NULL element |

How to read the output: look at the spread (min, median, max) before the
difference. On a shared machine two identical runs differ by 5% or more, so a
delta inside the spread is noise; raise `--sessions`, or run on an idle box.
The hand-written twin is the floor, not a goal: it hoists pointers and skips the
NULL checks the SDK does for you.

Results are not committed, because they depend on the machine. `AUDIT.md`
section 13 records one run with its hardware and build profile.
