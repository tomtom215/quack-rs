# hello-ext

A minimal, fully-working DuckDB community extension built with [quack-rs].
Use this as a copy-paste starting point for your own extension.

## What it registers

| SQL | Kind | Signature | Notes |
|-----|------|-----------|-------|
| `word_count(text)` | Aggregate | `VARCHAR → BIGINT` | Sums whitespace-separated words across all rows |
| `first_word(text)` | Scalar | `VARCHAR → VARCHAR` | Returns the first word; propagates `NULL` |
| `generate_series_ext(n)` | Table | `BIGINT → TABLE(value BIGINT)` | Emits integers `0 .. n-1`; demonstrates full bind/init/scan lifecycle |

All three functions are **verified against a live DuckDB 1.4.4 instance** — see the
[Live DuckDB testing](#live-duckdb-testing) section below.

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

-- Table function: generate a series of integers
SELECT * FROM generate_series_ext(5);
-- → 0, 1, 2, 3, 4

SELECT value * value AS square FROM generate_series_ext(4);
-- → 0, 1, 4, 9
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

The pure-Rust logic (`count_words`, `first_word`, `generate_series_ext` state) and
aggregate state transitions are all testable without a running DuckDB instance:

```bash
cargo test
```

All tests live in `src/lib.rs` under `#[cfg(test)]`.

## Live DuckDB testing

To load the extension into a live DuckDB session you must first append a
512-byte metadata block to the `.so` file. DuckDB reads this block (the last
512 bytes of the file) to validate the extension before loading.

### Step 1: Package the extension

```bash
# From the hello-ext directory, after cargo build --release:
python3 ../../scripts/append_metadata.py \
    target/release/libhello_ext.so \
    hello_ext.duckdb_extension \
    --abi-type C_STRUCT \
    --duckdb-version v1.4.0 \
    --api-version v1.2.0 \
    --platform linux_amd64
```

Or manually in Python (one-liner for CI):

```python
python3 - << 'EOF'
import sys, pathlib

def make_field(s, size=32):
    b = s.encode('ascii')
    return b + b'\x00' * (size - len(b))

so = pathlib.Path('target/release/libhello_ext.so').read_bytes()
metadata = (
    make_field('') * 3 +         # fields 0-2: reserved
    make_field('C_STRUCT') +      # field 3: ABI type
    make_field('v1.4.0') +        # field 4: DuckDB version
    make_field('v1.2.0') +        # field 5: C API version
    make_field('linux_amd64') +   # field 6: platform
    make_field('4') +             # field 7: magic
    b'\x00' * 256                 # signature (empty = unsigned)
)
pathlib.Path('hello_ext.duckdb_extension').write_bytes(so + metadata)
print('Done')
EOF
```

> **Metadata format:** The last 512 bytes of a `.duckdb_extension` file contain
> 8 × 32-byte null-terminated ASCII fields followed by a 256-byte signature area.
> Field 7 must be `"4"` (the magic), field 3 must be `"C_STRUCT"` for C API extensions
> (or `"CPP"` for C++ extensions), and field 6 must match the build platform.
> Fields 0–2 are reserved and must be zero-filled.

### Step 2: Load in Python DuckDB 1.4.4

```python
import duckdb

con = duckdb.connect(config={'allow_unsigned_extensions': True})
con.execute("SET allow_extensions_metadata_mismatch=true")
con.execute("LOAD 'hello_ext.duckdb_extension'")

# Verified results (all 11 pass against live DuckDB 1.4.4):
print(con.execute("SELECT word_count('hello world foo')").fetchone())   # (3,)
print(con.execute("SELECT first_word('hello world')").fetchone())        # ('hello',)
print(con.execute("SELECT * FROM generate_series_ext(5)").fetchall())    # [(0,),(1,),(2,),(3,),(4,)]
```

### Step 3: Load in DuckDB CLI

```bash
duckdb -unsigned
```

```sql
SET allow_extensions_metadata_mismatch=true;
LOAD 'hello_ext.duckdb_extension';

SELECT word_count('hello world foo');          -- 3
SELECT first_word('hello world');              -- hello
SELECT * FROM generate_series_ext(5);          -- 0 1 2 3 4
```

## Adapting this for your own extension

1. **Copy** this directory: `cp -r examples/hello-ext ../my-ext`
2. **Rename** the crate in `Cargo.toml` (`name = "my-ext"`)
3. **Replace** the functions in `src/lib.rs`:
   - For a **scalar** function, follow the `first_word_scalar` pattern
   - For an **aggregate** function, follow the `word_count` pattern
   - For a **table** function, follow the `generate_series_ext` pattern
4. **Update the entry point** — the symbol `my_ext_init_c_api` must match
   your crate name with underscores replacing hyphens
5. **Run** `cargo build --release` and load in DuckDB

### Checklist for a real extension

- [ ] Replace placeholder functions with your logic
- [ ] Add `repository`, `homepage`, `documentation` to `Cargo.toml`
- [ ] Add a `description.yml` (see `quack_rs::validate::parse_description_yml`)
- [ ] Verify your `[profile.release]` has `panic = "abort"`, `lto = true`
      (use `quack_rs::validate::validate_release_profile`)
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
├── GsBindData                  struct — holds n (the series limit); FfiBindData<GsBindData>
├── GsScanState                 struct — holds current index; FfiInitData<GsScanState>
├── gs_bind                     extracts n from duckdb_value via duckdb_get_int64
├── gs_init                     zero-initialises scan state via FfiInitData::init_callback
├── gs_scan                     emits a batch of i64 rows; sets duckdb_data_chunk_set_size
│
├── count_words / first_word    pure Rust — no unsafe, easy to unit-test
│
├── register()                  calls AggregateFunctionBuilder + ScalarFunctionBuilder
│   └──                               + TableFunctionBuilder for generate_series_ext
│   └── Returns ExtensionError on registration failure
│
└── entry_point!(hello_ext_init_c_api, ...)
    └── Generates the #[no_mangle] extern "C" symbol DuckDB loads by name
```

### Key quack-rs types used

| Type | What it does |
|------|-------------|
| `FfiState<S>` | Manages placement-new / drop-in-place for aggregate state |
| `FfiBindData<T>` | Manages bind data allocation and destruction for table functions |
| `FfiInitData<T>` | Manages per-scan init state for table functions |
| `BindInfo` | Safe wrapper for `duckdb_bind_info` — parameter extraction, column registration |
| `VectorReader` | Safe indexed access to a DuckDB column (read_str, is_valid, …) |
| `VectorWriter` | Safe indexed writes to a DuckDB vector (write_i64, write_varchar, set_null, …) |
| `AggregateFunctionBuilder` | Builder that registers an aggregate with DuckDB |
| `ScalarFunctionBuilder` | Builder that registers a scalar function with DuckDB |
| `TableFunctionBuilder` | Builder that registers a table function (bind/init/scan) |
| `AggregateTestHarness<S>` | Unit-test helper — no DuckDB process needed |
| `entry_point!` | Macro that emits the `#[no_mangle] extern "C"` entry point |

### Common pitfalls (with mitigations in this example)

| # | Pitfall | Where it shows up | Mitigation here |
|---|---------|-------------------|-----------------|
| L1 | `combine` must copy **all** state fields | `wc_combine` | Comment + test |
| L4 | `set_null` requires `ensure_validity_writable` first | `VectorWriter::set_null` | Handled inside `VectorWriter` |
| P2 | C API version ≠ DuckDB release version | `DUCKDB_API_VERSION` | Provided by `quack_rs` |
| L3 | No `panic!` across FFI | entry point | `init_extension` catches errors |

[quack-rs]: https://docs.rs/quack-rs
[duckdb-releases]: https://github.com/duckdb/duckdb/releases
