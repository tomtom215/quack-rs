# hello-ext

A minimal, fully-working DuckDB community extension built with [quack-rs].
Use this as a copy-paste starting point for your own extension.

## What it registers

| SQL | Kind | Signature | Notes |
|-----|------|-----------|-------|
| `word_count(text)` | Aggregate | `VARCHAR → BIGINT` | Sums whitespace-separated words across all rows |
| `first_word(text)` | Scalar | `VARCHAR → VARCHAR` | Returns the first word; propagates `NULL` |

```sql
-- Aggregate: count words across rows
SELECT word_count(sentence) FROM (
    VALUES ('hello world'), ('one two three'), (NULL)
) t(sentence);
-- → 5  (2 + 3; NULL rows contribute 0)

-- Scalar: first word of each row
SELECT first_word(sentence) FROM (
    VALUES ('hello world'), ('  padded  '), (''), (NULL)
) t(sentence);
-- → 'hello', 'padded', '', NULL
```

## Prerequisites

- Rust 1.84.1 or later (`rustup update stable`)
- DuckDB v1.4.x CLI for manual testing ([download][duckdb-releases])

## Build

```bash
# From this directory:
cargo build --release
```

Output:

| Platform | File |
|----------|------|
| Linux    | `target/release/libhello_ext.so` |
| macOS    | `target/release/libhello_ext.dylib` |
| Windows  | `target/release/hello_ext.dll` |

## Run the unit tests

The pure-Rust logic (`count_words`, `first_word`) and aggregate state
transitions are all testable without a running DuckDB instance:

```bash
cargo test
```

All tests live in `src/lib.rs` under `#[cfg(test)]`.

## Load and test in DuckDB

```sql
-- Adjust the path to your build output:
LOAD 'target/release/libhello_ext.so';

-- Scalar
SELECT first_word('hello world');          -- 'hello'
SELECT first_word('');                     -- ''
SELECT first_word(NULL);                   -- NULL

-- Aggregate
SELECT word_count(v) FROM (
    VALUES ('one'), ('two three'), (NULL), ('four five six')
) t(v);                                    -- 6

-- Group-by aggregate
SELECT category, word_count(text) FROM my_table GROUP BY category;
```

> **Tip:** DuckDB resolves the extension path relative to the shell's working
> directory.  Run `duckdb` from the repo root, or use an absolute path.

## Adapting this for your own extension

1. **Copy** this directory: `cp -r examples/hello-ext ../my-ext`
2. **Rename** the crate in `Cargo.toml` (`name = "my-ext"`)
3. **Replace** the functions in `src/lib.rs`:
   - For a **scalar** function, follow the `first_word_scalar` pattern
   - For an **aggregate** function, follow the `word_count` pattern
4. **Update the entry point** — the symbol `my_ext_init_c_api` must match
   your crate name with underscores replacing hyphens
5. **Run** `cargo build --release` and load in DuckDB

### Checklist for a real extension

- [ ] Replace placeholder functions with your logic
- [ ] Add `repository`, `homepage`, `documentation` to `Cargo.toml`
- [ ] Add a `description.yml` (see `quack_rs::validate::parse_description_yml`)
- [ ] Verify your `[profile.release]` has `panic = "abort"`, `lto = true`
      (use `quack_rs::validate::check_release_profile`)
- [ ] Add integration tests using `duckdb = { features = ["bundled"] }`

## Code tour

```
src/lib.rs
│
├── WordCountState              struct — one i64 field, implements AggregateState
├── wc_state_size / wc_state_init / wc_state_destroy
│   └── Thin wrappers around FfiState<WordCountState>::*_callback
│       FfiState handles the unsafe placement-new / drop-in-place for you.
│
├── wc_update                   reads VARCHAR column, calls count_words(), accumulates
├── wc_combine                  merges source states into target (parallel plans)
│   └── Pitfall L1: copy *all* fields — not just the result field
├── wc_finalize                 writes BIGINT output; marks NULL if state is invalid
│
├── first_word_scalar           reads VARCHAR, propagates NULL, writes VARCHAR output
│
├── count_words / first_word    pure Rust — no unsafe, easy to unit-test
│
├── register()                  calls AggregateFunctionBuilder + ScalarFunctionBuilder
│   └── Returns ExtensionError on registration failure
│
└── entry_point!(hello_ext_init_c_api, ...)
    └── Generates the #[no_mangle] extern "C" symbol DuckDB loads by name
```

### Key quack-rs types used

| Type | What it does |
|------|-------------|
| `FfiState<S>` | Manages placement-new / drop-in-place for aggregate state |
| `VectorReader` | Safe indexed access to a DuckDB column (read_str, is_valid, …) |
| `VectorWriter` | Safe indexed writes to a DuckDB vector (write_i64, write_varchar, set_null, …) |
| `AggregateFunctionBuilder` | Builder that registers an aggregate with DuckDB |
| `ScalarFunctionBuilder` | Builder that registers a scalar function with DuckDB |
| `AggregateTestHarness<S>` | Unit-test helper — no DuckDB process needed |
| `entry_point!` | Macro that emits the `#[no_mangle] extern "C"` entry point |

### Common pitfalls (with mitigations in this example)

| # | Pitfall | Where it shows up | Mitigation here |
|---|---------|-------------------|-----------------|
| L1 | `combine` must copy **all** state fields | `wc_combine` | Comment + test |
| L4 | `set_null` requires `ensure_validity_writable` first | `VectorWriter::set_null` | Handled inside `VectorWriter` |
| P8 | C API version ≠ DuckDB release version | `DUCKDB_API_VERSION` | Provided by `quack_rs` |
| P13 | No `panic!` across FFI | entry point | `init_extension` catches errors |

[quack-rs]: https://docs.rs/quack-rs
[duckdb-releases]: https://github.com/duckdb/duckdb/releases
