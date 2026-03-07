# FAQ

Frequently asked questions about quack-rs and building DuckDB extensions in Rust.

---

## General

### What is quack-rs?

quack-rs is a Rust SDK for building DuckDB loadable extensions using DuckDB's
pure C Extension API. It provides safe, ergonomic builders for registering
scalar functions, aggregate functions, and SQL macros, along with helpers for
reading and writing DuckDB vectors, and utilities for publishing community
extensions.

### Why does this exist?

Building a DuckDB extension in Rust requires solving a set of undocumented
FFI problems that every developer discovers independently. quack-rs encodes
solutions to all 15 known pitfalls so you don't have to rediscover them.
See the [Pitfall Catalog](reference/pitfalls.md).

### What DuckDB version does quack-rs target?

quack-rs pins `libduckdb-sys = "=1.4.4"` (DuckDB 1.4.x). The C API version
string passed to the dispatch-table initializer is separately `"v1.2.0"`,
available as `quack_rs::DUCKDB_API_VERSION`. These are two distinct version
identifiers — the crate version and the C API protocol version.

### What is the minimum supported Rust version (MSRV)?

Rust **1.84.1** or later. This is enforced in `Cargo.toml` with
`rust-version = "1.84.1"`.

### Is quack-rs production-ready?

Yes. It was extracted from
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral), a
production DuckDB community extension. All 15 pitfalls it solves were discovered
in production.

---

## Functions

### Can I expose SQL macros as an extension?

**Yes, without any C++ wrapper code.** Use `quack_rs::sql_macro::SqlMacro`:

```rust
use quack_rs::sql_macro::SqlMacro;

// Scalar macro
let m = SqlMacro::scalar("double_it", &["x"], "x * 2")?;
unsafe { m.register(con) }?;

// Table macro
let m = SqlMacro::table("recent_events", &["n"],
    "SELECT * FROM events ORDER BY ts DESC LIMIT n")?;
unsafe { m.register(con) }?;
```

Register them inside your `init_extension` closure alongside aggregate and
scalar functions. See [SQL Macros](functions/sql-macros.md).

### Can I register multiple overloads of the same function?

Yes, using `AggregateFunctionSetBuilder` (for aggregates) or multiple
`ScalarFunctionBuilder` registrations (for scalars). See
[Overloading with Function Sets](functions/aggregate-sets.md).

### Can I register multiple functions in one extension?

Yes. The `init_extension` closure receives a `duckdb_connection` and can call
as many `register_*` functions as needed:

```rust
quack_rs::entry_point::init_extension(info, access, DUCKDB_API_VERSION, |con| {
    unsafe { register_word_count(con) }?;
    unsafe { register_sentence_count(con) }?;
    SqlMacro::scalar("double_it", &["x"], "x * 2")?
        .register(con)?;
    Ok(())
})
```

### Can I use the `duckdb` crate instead of `libduckdb-sys`?

No. The `duckdb` crate's `bundled` feature embeds its own copy of DuckDB. A
loadable extension must link against the DuckDB that loads it, not bundle a
separate copy. Use `libduckdb-sys` with the `loadable-extension` feature.

### Can I have a scalar function with no parameters?

Yes. Pass an empty slice to `param`:

```rust
ScalarFunctionBuilder::new("current_quack")
    .returns(TypeId::Varchar)
    .function(quack_callback)
    .register(con)?;
```

---

## Testing

### Do I need a DuckDB instance to run unit tests?

No. `AggregateTestHarness` simulates the aggregate lifecycle in pure Rust
without any DuckDB dependency. You can run `cargo test` without loading a DuckDB
binary.

### My unit tests all pass but the extension crashes. Why?

Unit tests cannot detect FFI wiring bugs. See [Pitfall P3](reference/pitfalls.md#p3-e2e-testing-is-mandatory)
and the [Testing Guide](testing.md). Always run E2E tests by loading the
extension into an actual DuckDB process.

### How do I test SQL macros?

`SqlMacro::to_sql()` is pure Rust and requires no DuckDB connection:

```rust
let m = SqlMacro::scalar("triple", &["x"], "x * 3").unwrap();
assert_eq!(m.to_sql(), "CREATE OR REPLACE MACRO triple(x) AS (x * 3)");
```

For E2E testing, include the macro in your SQLLogicTest file:

```sql
query I
SELECT double_it(21);
----
42
```

---

## Publishing

### How do I publish to the DuckDB community extensions registry?

1. Scaffold your project with `generate_scaffold`
2. Push to GitHub
3. Submit a pull request to the
   [community-extensions](https://github.com/duckdb/community-extensions) repo
   with your `description.yml`

See [Community Extensions](publishing.md) for the full workflow.

### My extension name is taken. What should I do?

Use a vendor-prefixed name: `myorg_analytics` instead of `analytics`. Extension
names must be globally unique across the entire DuckDB ecosystem. Check
[community-extensions.duckdb.org](https://community-extensions.duckdb.org/)
first.

### Do I need to set up CI manually?

No. `generate_scaffold` produces `.github/workflows/extension-ci.yml` which
builds and tests your extension on Linux, macOS, and Windows automatically.

### Can my extension be installed with `INSTALL ... FROM community`?

Yes, once your pull request is merged into the community-extensions repository.
Until then, users load the `.duckdb_extension` binary directly:

```sql
LOAD './path/to/libmy_extension.duckdb_extension';
```

---

## Troubleshooting

### My aggregate returns wrong results with no error.

The most common cause is Pitfall L1: your `combine` callback is not propagating
all configuration fields. See
[Pitfall L1](reference/pitfalls.md#l1-combine-must-propagate-all-config-fields)
and test with `AggregateTestHarness::combine`.

### I'm getting a SEGFAULT when writing NULL.

You are likely calling `duckdb_vector_get_validity` without first calling
`duckdb_vector_ensure_validity_writable`. Use `VectorWriter::set_null` instead.
See [Pitfall L4](reference/pitfalls.md#l4-ensure_validity_writable-is-required-before-null-output).

### My function is not found in SQL after `LOAD`.

Most likely cause: the function was not registered (Pitfall L6 — function set
name not set on each member), or the entry point symbol name does not match
the extension name. The symbol must be `{extension_name}_init_c_api` (all
lowercase, underscores).

### `make configure` fails with a missing file error.

The `extension-ci-tools` submodule is not initialized:

```bash
git submodule update --init --recursive
```

### My SQLLogicTest fails in CI but passes locally.

SQLLogicTest does exact string matching. The most common issue is a difference
in NULL representation, decimal places, or line endings. Run the query in the
same DuckDB version used by CI and copy the output verbatim.

### How do I read a VARCHAR that is longer than 12 bytes?

`VectorReader::read_str` handles both the inline (≤ 12 bytes) and pointer
(> 12 bytes) formats automatically. No special handling needed.

### What happens if I read from a NULL row?

You get garbage data from the vector's data buffer. Always check `is_valid`
before reading. See [NULL Handling & Strings](data/nulls-and-strings.md).

---

## Architecture

### Why use `libduckdb-sys` with `loadable-extension` instead of the `duckdb` crate?

The `duckdb` crate is designed for embedding DuckDB, not for extending it. Its
`bundled` feature includes a statically linked DuckDB binary, which conflicts
with the DuckDB runtime that loads your extension. `libduckdb-sys` with
`loadable-extension` provides lazy-initialized function pointers that are
populated by DuckDB at extension load time.

### Why not use `duckdb-loadable-macros`?

`duckdb-loadable-macros` relies on `extract_raw_connection` which uses the
internal `Rc<RefCell<InnerConnection>>` layout. This is fragile and causes
SEGFAULTs when the layout changes between `duckdb` crate versions.
`init_extension` uses the correct C API entry sequence directly.

### Why is `panic = "abort"` required?

Panics cannot unwind across FFI boundaries in Rust. A panic in an
`unsafe extern "C"` callback is undefined behavior. `panic = "abort"` converts
panics to process termination, which is still bad but not undefined behavior.
Always use `Result` and `?` in your callbacks instead.

### Can I use async Rust in my extension?

Not directly in FFI callbacks. DuckDB's callbacks are synchronous C functions.
You can run a Tokio or async-std runtime and block on async tasks inside
callbacks (using `Runtime::block_on`), but the callbacks themselves must return
synchronously.

### How does `FfiState<T>` prevent double-free?

`FfiState<T>` stores the `Box<T>` as a raw pointer in `inner`. When
`destroy_callback` is called, it reconstitutes the `Box` (which drops `T` and
frees memory) and then sets `inner` to null. A second call to `destroy_callback`
on the same state sees a null `inner` and returns without freeing.
