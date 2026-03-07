# Extension Anatomy

A DuckDB loadable extension is a shared library (`.so` / `.dylib` / `.dll`) that DuckDB loads
at runtime. Understanding what DuckDB expects makes every other part of quack-rs click.

---

## The initialization sequence

When DuckDB loads your extension, it:

1. Opens the shared library and looks up the symbol `{name}_init_c_api`
2. Calls that function with an `info` handle and a pointer to function dispatch pointers
3. Your function must:
   a. Call `duckdb_rs_extension_api_init(info, access, api_version)` to initialize the dispatch table
   b. Get the `duckdb_database` handle via `access.get_database(info)`
   c. Open a `duckdb_connection` via `duckdb_connect`
   d. Register functions on that connection
   e. Disconnect
   f. Return `true` (success) or `false` (failure)

`quack_rs::entry_point::init_extension` performs all of this correctly. The `entry_point!`
macro generates the required `#[no_mangle] extern "C"` symbol:

```rust
entry_point!(my_extension, |con| register(con));
// generates: pub unsafe extern "C" fn my_extension_init_c_api(...)
```

---

## Symbol naming

The symbol name **must** be `{extension_name}_init_c_api` — all lowercase, underscores only.
If the symbol is missing or misnamed, DuckDB fails to load the extension.

```
Extension name: "word_count_ext"
Expected symbol: word_count_ext_init_c_api
```

The `entry_point!` macro uses the [`paste`](https://docs.rs/paste) crate to concatenate
`{name}_init_c_api` at compile time, so the symbol is always correct.

---

## The `loadable-extension` feature

`libduckdb-sys` with `features = ["loadable-extension"]` changes how DuckDB API functions
work fundamentally:

```
Without feature:  duckdb_query(...)  →  calls linked libduckdb directly
With feature:     duckdb_query(...)  →  dispatches through an AtomicPtr table
```

The `AtomicPtr` table starts as null. DuckDB fills it in by calling
`duckdb_rs_extension_api_init`. This means:

- **Any call before `duckdb_rs_extension_api_init` panics** with `"DuckDB API not initialized"`
- **In `cargo test`, you cannot call any `duckdb_*` function** — the table is never initialized

This is why `quack-rs` uses `AggregateTestHarness` for testing: it simulates the aggregate
lifecycle in pure Rust, with zero DuckDB API calls.

---

## Dependency model

```
your-extension
├── quack-rs
│   └── libduckdb-sys =1.4.4 { loadable-extension }
│           (headers only — no bundled DuckDB library)
└── libduckdb-sys =1.4.4 { loadable-extension }
```

The `loadable-extension` feature produces a shared library that **does not statically link
DuckDB**. Instead, it receives DuckDB's function pointers at load time. This is the correct
model for extensions: you run inside DuckDB's process, using its memory and threading.

---

## Version pinning

`libduckdb-sys = "=1.4.4"` — the `=` is intentional and important.

DuckDB's C API changes between minor releases:
- New function signatures
- Changed constant values
- Renamed symbols

An `=` pin makes every DuckDB version upgrade a deliberate, auditable change. Without it,
a patch-level Cargo update could silently link against a DuckDB version with breaking API
changes.

---

## Binary compatibility

Extension binaries are tied to a specific DuckDB version and platform. Key facts:

- An extension compiled for DuckDB 1.4.4 will **not** load in DuckDB 1.5.0
- DuckDB verifies binary compatibility at load time and refuses mismatched binaries
- Official DuckDB extensions are cryptographically signed; community extensions are not
- To load unsigned extensions: `SET allow_unsigned_extensions = true` (development only)
- The community extension CI provides automated cross-platform builds for each DuckDB release
