# hello-ext

A comprehensive, fully-working DuckDB community extension built with [quack-rs]
that exercises **every feature** of the SDK. Use it as a reference implementation
or copy-paste starting point for your own extension.

## What it registers

| SQL | Kind | Demonstrates |
|-----|------|-------------|
| `word_count(text)` | Aggregate | `AggregateFunctionBuilder`, full lifecycle (state/update/combine/finalize) |
| `first_word(text)` | Scalar | `ScalarFunctionBuilder`, NULL propagation |
| `generate_series_ext(n)` | Table | `TableFunctionBuilder`, full bind/init/scan lifecycle |
| `CAST(VARCHAR AS INTEGER)` | Cast | `CastFunctionBuilder`, `CastMode::Normal` vs `CastMode::Try` |
| `sum_list(LIST(BIGINT))` | Scalar | `param_logical(LogicalType)`, `ListVector` child access |
| `make_pair(k, v)` | Scalar | `returns_logical(LogicalType)`, `StructVector` child writes |
| `coalesce_val(a, b)` | Scalar Set | `ScalarFunctionSetBuilder`, per-overload `null_handling` |
| `typed_sum(a,b)` / `typed_sum(a,b,c)` | Aggregate Set | `AggregateFunctionSetBuilder::overloads` |
| `double_it(x)` | SQL Macro (scalar) | `SqlMacro::scalar` |
| `seq_n(n)` | SQL Macro (table) | `SqlMacro::table` |
| `make_kv_map(k, v)` | Scalar | `MapVector`, `LogicalType::map()` |
| `gen_series_v2(n, step:=1)` | Table | `named_param`, `local_init`, `set_max_threads` |
| `CAST(DOUBLE AS BIGINT)` | Cast | `implicit_cost`, `extra_info` |
| `add_interval(iv, micros)` | Scalar | `DuckInterval` read/write |
| `all_types_echo(...)` | Scalar | All `VectorReader`/`VectorWriter` types, `ValidityBitmap` |
| `read_hello(name)` | Table | Backing function for replacement scan |
| `SELECT * FROM 'hello:xxx'` | Replacement Scan | `ReplacementScanBuilder`, `ReplacementScanInfo` |

All functions use `entry_point_v2!` with `Connection`/`Registrar` for type-safe
registration.

All **29 live SQL tests** pass against both **DuckDB 1.4.4** and **DuckDB 1.5.0**.

```sql
-- Aggregate: count words across rows
SELECT word_count(sentence) FROM (
    VALUES ('hello world'), ('one two three'), (NULL)
) t(sentence);
-- → 5  (2 + 3; NULL rows contribute 0)

-- Scalar: first word
SELECT first_word('hello world');           -- → 'hello'

-- Table function: generate a series
SELECT * FROM generate_series_ext(5);       -- → 0, 1, 2, 3, 4

-- Cast / TRY_CAST
SELECT CAST('42' AS INTEGER);              -- → 42
SELECT TRY_CAST('bad' AS INTEGER);         -- → NULL

-- Complex types
SELECT sum_list([1, 2, 3]);                -- → 6
SELECT make_pair('hello', 42);             -- → {'key': hello, 'value': 42}
SELECT make_kv_map('hello', 42);           -- → {hello=42}

-- Aggregate set (overloaded arity)
SELECT typed_sum(a, b) FROM (VALUES (1, 2), (3, 4)) t(a, b);       -- → 10
SELECT typed_sum(a, b, c) FROM (VALUES (1, 2, 3), (4, 5, 6)) t(a, b, c); -- → 21

-- Scalar set with NULL handling
SELECT coalesce_val(NULL::BIGINT, 99);     -- → 99
SELECT coalesce_val(NULL::VARCHAR, 'fb');  -- → 'fb'

-- SQL macros
SELECT double_it(21);                      -- → 42
SELECT * FROM seq_n(5);                    -- → 1..5

-- Table function with named param + local init
SELECT * FROM gen_series_v2(3, step := 10); -- → 0, 10, 20

-- Cast with implicit cost + extra_info
SELECT 3.7::BIGINT;                        -- → 4 (rounded)

-- INTERVAL
SELECT add_interval(INTERVAL '1 day', 1000000); -- → 1 day 00:00:01

-- All types echo (bool, i8-i128, u8-u64, f32, f64)
SELECT all_types_echo(true, 1::TINYINT, 2::SMALLINT, 3, 4::BIGINT,
    5::UTINYINT, 6::USMALLINT, 7::UINTEGER, 8::UBIGINT,
    9.5::FLOAT, 10.5, 11::HUGEINT);
-- → 'b=true,i8=1,i16=2,i32=3,i64=4,u8=5,u16=6,u32=7,u64=8,f32=9.5,f64=10.5,i128=11'

-- Replacement scan
SELECT * FROM 'hello:DuckDB';             -- → 'Hello, DuckDB!'
```

## Prerequisites

- Rust 1.84.1 or later (`rustup update stable`)
- DuckDB 1.4.x or 1.5.x CLI for live testing ([download][duckdb-releases])

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

The pure-Rust logic and aggregate state transitions are all testable without a
running DuckDB instance:

```bash
cargo test
```

39 tests live in `src/lib.rs` under `#[cfg(test)]`, covering:
- `count_words` / `first_word` string helpers
- `parse_varchar_to_int` parsing and edge cases
- `WordCountState` aggregate lifecycle via `AggregateTestHarness`
- `TypedSumState` aggregate set logic
- `GenerateSeriesState` batching logic
- `sum_list` / `coalesce` pure logic
- `DuckInterval` arithmetic
- `SqlMacro` construction
- `gen_series_v2` step logic

## Live DuckDB testing

To load the extension into a live DuckDB session you must first append a
512-byte metadata block to the `.so` file. DuckDB reads this block (the last
512 bytes of the file) to validate the extension before loading.

### Step 1: Package the extension

```bash
# From the workspace root, after cargo build --release:
cargo run --bin append_metadata -- \
    examples/hello-ext/target/release/libhello_ext.so \
    hello_ext.duckdb_extension \
    --abi-type C_STRUCT \
    --extension-version v0.1.0 \
    --duckdb-version v1.2.0 \
    --platform linux_amd64

# Or install once and use from anywhere:
cargo install --path . --bin append_metadata
append_metadata libhello_ext.so hello_ext.duckdb_extension \
    --extension-version v0.1.0 --duckdb-version v1.2.0 --platform linux_amd64
```

> **Metadata format:** The last 512 bytes of a `.duckdb_extension` file contain
> 8 × 32-byte null-terminated ASCII fields followed by a 256-byte signature area.
> Field 7 must be `"4"` (the magic), field 3 must be `"C_STRUCT"` for C API extensions
> (or `"CPP"` for C++ extensions), and field 6 must match the build platform.
> Fields 0–2 are reserved and must be zero-filled.

### Step 2: Load in DuckDB CLI

```bash
duckdb -unsigned
```

```sql
SET allow_extensions_metadata_mismatch=true;
LOAD 'hello_ext.duckdb_extension';

-- All 29 tests verified against DuckDB 1.4.4 and DuckDB 1.5.0:

-- T01: word_count aggregate
SELECT word_count(sentence) AS wc FROM (
    VALUES ('hello world'), ('one two three'), (NULL)) t(sentence);  -- 5

-- T02: first_word scalar
SELECT first_word('hello world');                                    -- hello

-- T03–T04: generate_series_ext table function
SELECT COUNT(*) FROM generate_series_ext(5);                         -- 5
SELECT COUNT(*) FROM generate_series_ext(0);                         -- 0

-- T05–T06: CAST / TRY_CAST
SELECT CAST('42' AS INTEGER);                                        -- 42
SELECT TRY_CAST('bad' AS INTEGER);                                   -- NULL

-- T07–T08: sum_list with param_logical
SELECT sum_list([1, 2, 3]);                                          -- 6
SELECT sum_list([10, NULL, 20]);                                     -- 30

-- T09: make_pair with returns_logical + StructVector
SELECT make_pair('hello', 42);                   -- {'key': hello, 'value': 42}

-- T10–T12: coalesce_val scalar set with null_handling
SELECT coalesce_val(NULL::BIGINT, 99);                               -- 99
SELECT coalesce_val(NULL::VARCHAR, 'fallback');                      -- fallback
SELECT coalesce_val(42::BIGINT, 99);                                 -- 42

-- T13–T14: typed_sum aggregate set (2-arg and 3-arg overloads)
SELECT typed_sum(a, b) FROM (VALUES (1, 2), (3, 4)) t(a, b);        -- 10
SELECT typed_sum(a, b, c) FROM (VALUES (1, 2, 3), (4, 5, 6)) t(a, b, c); -- 21

-- T15: double_it SQL scalar macro
SELECT double_it(21);                                                -- 42

-- T16: seq_n SQL table macro
SELECT * FROM seq_n(5);                                              -- 1..5

-- T17: make_kv_map with MapVector + LogicalType::map()
SELECT make_kv_map('hello', 42);                                     -- {hello=42}

-- T18–T19: gen_series_v2 with named_param + local_init
SELECT COUNT(*) FROM gen_series_v2(5);                               -- 5
SELECT * FROM gen_series_v2(3, step := 10);                          -- 0, 10, 20

-- T20: add_interval (DuckInterval read/write)
SELECT add_interval(INTERVAL '1 day', 1000000);                      -- 1 day 00:00:01

-- T21–T22: all_types_echo (all reader/writer types + ValidityBitmap)
SELECT all_types_echo(true, 1::TINYINT, 2::SMALLINT, 3, 4::BIGINT,
    5::UTINYINT, 6::USMALLINT, 7::UINTEGER, 8::UBIGINT,
    9.5::FLOAT, 10.5, 11::HUGEINT);
-- → 'b=true,i8=1,i16=2,i32=3,i64=4,u8=5,u16=6,u32=7,u64=8,f32=9.5,f64=10.5,i128=11'
SELECT all_types_echo(NULL::BOOLEAN, 1::TINYINT, 2::SMALLINT, 3, 4::BIGINT,
    5::UTINYINT, 6::USMALLINT, 7::UINTEGER, 8::UBIGINT,
    9.5::FLOAT, 10.5, 11::HUGEINT);                                 -- NULL

-- T23–T24: Replacement scan
SELECT * FROM read_hello('world');                                   -- Hello, world!
SELECT * FROM 'hello:DuckDB';                                       -- Hello, DuckDB!

-- T25: DOUBLE→BIGINT cast with implicit_cost + extra_info
SELECT 3.7::BIGINT;                                                  -- 4

-- T26–T28: NULL edge cases
SELECT sum_list(NULL::BIGINT[]);                                     -- NULL
SELECT make_pair(NULL, 42);                                          -- NULL
SELECT make_kv_map(NULL, 42);                                        -- NULL

-- T29: gen_series_v2 projection pushdown (value column only)
SELECT value FROM gen_series_v2(3);                                  -- 0, 1, 2
```

## Adapting this for your own extension

1. **Copy** this directory: `cp -r examples/hello-ext ../my-ext`
2. **Rename** the crate in `Cargo.toml` (`name = "my-ext"`)
3. **Replace** the functions in `src/lib.rs` — use the existing functions as
   patterns for the type you need (scalar, aggregate, table, cast, etc.)
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
├── entry_point_v2!(hello_ext_init_c_api, ...)
│   └── Uses Connection / Registrar for version-agnostic registration
│
├── register_all(&Connection)       orchestrates all registrations below
│
├── Aggregate: word_count
│   ├── WordCountState              implements AggregateState
│   ├── wc_update / wc_combine / wc_finalize
│   │   └── Pitfall L1: combine copies ALL state fields
│   └── count_words()               pure Rust helper
│
├── Scalar: first_word
│   ├── first_word_scalar           reads VARCHAR, writes VARCHAR
│   └── first_word()                pure Rust helper
│
├── Table: generate_series_ext
│   ├── GenerateSeriesState         FfiInitData<T> for scan state
│   └── gs_bind / gs_init / gs_scan
│
├── Cast: VARCHAR → INTEGER
│   ├── varchar_to_int              handles CastMode::Normal vs Try
│   └── parse_varchar_to_int()      pure Rust parser
│
├── Scalar: sum_list                param_logical(LogicalType::list(...))
│                                   ListVector child access
│
├── Scalar: make_pair               returns_logical(LogicalType::struct_type(...))
│                                   StructVector child writes
│
├── Scalar: make_kv_map             LogicalType::map(), MapVector
│
├── Scalar Set: coalesce_val        ScalarFunctionSetBuilder, per-overload null_handling
│   ├── coalesce_bigint             BIGINT overload
│   └── coalesce_varchar            VARCHAR overload
│
├── Aggregate Set: typed_sum        AggregateFunctionSetBuilder::overloads
│   ├── TypedSumState               shared state for both overloads
│   └── 2-arg and 3-arg callbacks
│
├── SQL Macros
│   ├── double_it(x)                SqlMacro::scalar("x * 2")
│   └── seq_n(n)                    SqlMacro::table("SELECT * FROM generate_series(1, n)")
│
├── Table: gen_series_v2            named_param("step"), local_init, set_max_threads
│   ├── GenSeriesV2Config           FfiBindData for bind-time config
│   ├── GenSeriesV2State            FfiInitData for scan state
│   ├── GenSeriesV2Local            FfiLocalInitData for per-thread state
│   └── gs_v2_bind / gs_v2_init / gs_v2_local_init / gs_v2_scan
│
├── Cast: DOUBLE → BIGINT           implicit_cost(100), extra_info (rounding mode)
│   └── double_to_bigint
│
├── Scalar: add_interval            DuckInterval read/write via VectorReader/VectorWriter
│
├── Scalar: all_types_echo          exercises ALL VectorReader/VectorWriter types:
│   └── bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, i128
│       plus ValidityBitmap for NULL detection
│
├── Table: read_hello               backing table function for replacement scan
│
└── Replacement Scan                ReplacementScanBuilder / ReplacementScanInfo
    └── hello_replacement_scan      matches 'hello:xxx' → read_hello('xxx')
```

### Key quack-rs types used

| Type | What it does |
|------|-------------|
| `Connection` | Version-agnostic wrapper for `duckdb_connection` |
| `Registrar` | Trait providing `register_scalar`, `register_aggregate`, etc. |
| `entry_point_v2!` | Generates `#[no_mangle] extern "C"` entry point with `Connection` |
| `FfiState<S>` | Manages placement-new / drop-in-place for aggregate state |
| `FfiBindData<T>` | Manages bind data allocation and destruction for table functions |
| `FfiInitData<T>` | Manages per-scan init state for table functions |
| `FfiLocalInitData<T>` | Per-thread local init state for table functions |
| `BindInfo` | Safe wrapper for `duckdb_bind_info` — parameter extraction, column registration |
| `InitInfo` | Safe wrapper for `duckdb_init_info` — `set_max_threads`, projection info |
| `VectorReader` | Safe indexed access to a DuckDB column (read_str, read_i64, read_bool, …) |
| `VectorWriter` | Safe indexed writes to a DuckDB vector (write_i64, write_varchar, set_null, …) |
| `ValidityBitmap` | Direct NULL bitmap read/write |
| `LogicalType` | RAII wrapper for complex types (list, struct, map) |
| `StructVector` | Write to STRUCT child vectors |
| `ListVector` | Access LIST element vectors |
| `MapVector` | Write to MAP key/value vectors |
| `DuckInterval` | 16-byte INTERVAL struct (months, days, micros) |
| `SqlMacro` | SQL macro registration (scalar and table macros, no FFI callbacks) |
| `ReplacementScanInfo` | Info handle for replacement scan callbacks |
| `AggregateFunctionBuilder` | Builder for a single aggregate function |
| `AggregateFunctionSetBuilder` | Builder for overloaded aggregate functions |
| `ScalarFunctionBuilder` | Builder for a single scalar function |
| `ScalarFunctionSetBuilder` | Builder for overloaded scalar functions |
| `TableFunctionBuilder` | Builder for table functions (bind/init/scan) |
| `CastFunctionBuilder` | Builder for CAST / TRY_CAST functions |
| `CastFunctionInfo` | Info handle inside cast callbacks — `cast_mode()`, error reporting |
| `AggregateTestHarness<S>` | Unit-test helper — no DuckDB process needed |

### Common pitfalls (with mitigations in this example)

| # | Pitfall | Where it shows up | Mitigation here |
|---|---------|-------------------|-----------------|
| L1 | `combine` must copy **all** state fields | `wc_combine`, `typed_sum_combine` | Comment + test |
| L4 | `set_null` requires `ensure_validity_writable` first | `VectorWriter::set_null` | Handled inside `VectorWriter` |
| L5 | Boolean reads must use `u8 != 0` | `all_types_echo` | `VectorReader::read_bool` |
| L6 | Set name must be set on each member | `coalesce_val`, `typed_sum` | Set builders handle it |
| L7 | `LogicalType` memory leak if not freed | `sum_list`, `make_pair`, `make_kv_map` | `LogicalType` implements `Drop` |
| P2 | C API version ≠ DuckDB release version | `DUCKDB_API_VERSION` | Provided by `quack_rs` |
| P7 | 16-byte string format | `VectorReader::read_str` | Handled inside `VectorReader` |
| P8 | INTERVAL layout | `add_interval` | `DuckInterval` struct |
| L3 | No `panic!` across FFI | entry point | `init_extension_v2` catches errors |

[quack-rs]: https://docs.rs/quack-rs
[duckdb-releases]: https://github.com/duckdb/duckdb/releases
