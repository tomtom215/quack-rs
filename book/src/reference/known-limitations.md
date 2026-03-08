# Known Limitations

## Window functions and COPY functions are not available

DuckDB **window functions** (`OVER (...)` clauses) and **COPY format handlers**
(custom file-format readers/writers) are implemented entirely in DuckDB's C++ layer
and have **no counterpart in the public C extension API**.

This is not a gap in `quack-rs` or in `libduckdb-sys` — the relevant symbols
(`duckdb_create_window_function`, `duckdb_create_copy_function`, etc.) simply do not
exist in the C API:

| Symbol | C API? | C++ API? |
|--------|--------|----------|
| `duckdb_create_window_function` | **No** | Yes |
| `duckdb_create_copy_function`   | **No** | Yes |
| `duckdb_create_scalar_function` | Yes    | Yes |
| `duckdb_create_aggregate_function` | Yes | Yes |
| `duckdb_create_table_function`  | Yes    | Yes |
| `duckdb_create_cast_function`   | Yes    | Yes |

**Verified against:**

- The [DuckDB stable C API reference](https://duckdb.org/docs/stable/clients/c/api)
  — window and COPY registration symbols are not listed.
- The `libduckdb-sys` 1.4.4 bindgen output (`bindgen_bundled_version.rs` and
  `bindgen_bundled_version_loadable.rs`) — neither file contains these symbols.

**What this means for your extension:**

If your extension needs window-function semantics, you can approximate them with
aggregate functions in most cases (DuckDB will push down the window logic). True
custom window operator registration requires writing a C++ extension.

If DuckDB exposes window or COPY registration in a future C API version, `quack-rs`
will add wrappers in the corresponding release.
