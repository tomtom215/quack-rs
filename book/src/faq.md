# FAQ

Frequently asked questions about quack-rs and building DuckDB extensions in Rust.

---

## General

### What is quack-rs?

quack-rs is a Rust SDK for building DuckDB loadable extensions using DuckDB's
pure C Extension API. It provides safe, ergonomic builders for registering
scalar functions, aggregate functions, table functions, cast functions,
replacement scans, SQL macros, and copy functions (via the `duckdb-1-5`
feature), along with helpers for reading and writing DuckDB vectors, and
utilities for publishing community extensions.

### Why does this exist?

Building a DuckDB extension in Rust means solving a set of undocumented FFI
problems that each developer otherwise discovers independently. quack-rs
documents all 31 known pitfalls, and its API prevents most of them. See the
[Pitfall Catalog](reference/pitfalls.md).

### What DuckDB version does quack-rs target?

quack-rs requires `libduckdb-sys = ">=1.4.4, <2"` and supports DuckDB 1.4.x
and 1.5.x. CI loads a built extension into DuckDB v1.4.4, v1.5.0, v1.5.5 and the
latest release.

The C API version passed to the dispatch-table initializer is `"v1.2.0"`,
available as `quack_rs::DUCKDB_API_VERSION`. Every DuckDB 1.4.x and 1.5.x
release loads extensions built for it (1.5.6 declares C API `v1.5.6`, but still
accepts `v1.2.0`). The C API version is a separate identifier from the DuckDB
release and from the `libduckdb-sys` crate version (e.g. `1.10505.0` for
DuckDB 1.5.5).

### What is the minimum supported Rust version (MSRV)?

Rust **1.86.0** or later. This is enforced in `Cargo.toml` with
`rust-version = "1.86.0"`.

### Is quack-rs production-ready?

It is pre-1.0, and you should judge it against your own requirements rather than
take a yes. What the record shows:

- **The API still changes.** Minor releases before 1.0 can break it; each one lists
  its breaking changes, with migration notes, in the [changelog](reference/changelog.md).
- **Audits keep finding real defects.** The review released as 0.16.0 fixed 24
  defects in earlier releases, including two heap-corruption paths. The 0.18.0 audits fixed further soundness holes,
  process aborts and wrong answers, and found one serious defect in unreleased
  code before it was published: aggregate functions returned wrong results in
  release builds with Cargo's default profile. `AUDIT.md` in the repository
  records each audit, what it found, and whether each fix was reproduced against
  a real DuckDB or derived from DuckDB's source.
- **Some limits are DuckDB's.** The C API has defects quack-rs can only document or
  work around; see [Known Limitations](reference/known-limitations.md).

It was extracted from
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral), a DuckDB
community extension, where the first 16 of the pitfalls it now documents were
discovered. If you ship an extension built on it, run end-to-end tests that load
the extension into each DuckDB release you support (see the
[Testing Guide](testing.md)).

---

## Functions

### Can I expose SQL macros as an extension?

**Yes, without any C++ wrapper code.** Use `quack_rs::sql_macro::SqlMacro`:

```rust
# use libduckdb_sys::duckdb_connection;
# fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
use quack_rs::sql_macro::SqlMacro;

// Scalar macro
let m = SqlMacro::scalar("double_it", &["x"], "x * 2")?;
unsafe { m.register(con) }?;

// Table macro
let m = SqlMacro::table("recent_events", &["n"],
    "SELECT * FROM events ORDER BY ts DESC LIMIT n")?;
unsafe { m.register(con) }?;
# Ok(())
# }
```

Register them in your registration closure (the one passed to `entry_point!`
or `init_extension`) alongside your other functions. A table macro's body is
bound when it is created, so the
`events` table must already exist or `register` returns an error.
See [SQL Macros](functions/sql-macros.md).

### Can I register multiple overloads of the same function?

Yes, using `AggregateFunctionSetBuilder` (for aggregates) or
`ScalarFunctionSetBuilder` (for scalars). Both support complex parameter types
via `param_logical(LogicalType)` and complex return types via
`returns_logical(LogicalType)`, and in both, **each overload may return a
different type** — DuckDB resolves an overload from its parameter types and
arity alone. See
[Overloading with Function Sets](functions/aggregate-sets.md).

### Can I register multiple functions in one extension?

Yes. The registration closure receives a `duckdb_connection` and can register
as many functions as needed:

```rust
# use libduckdb_sys::{duckdb_connection, duckdb_extension_access, duckdb_extension_info};
# use quack_rs::error::ExtensionError;
# use quack_rs::sql_macro::SqlMacro;
# use quack_rs::DUCKDB_API_VERSION;
# unsafe fn register_word_count(_: duckdb_connection) -> Result<(), ExtensionError> { Ok(()) }
# unsafe fn register_sentence_count(_: duckdb_connection) -> Result<(), ExtensionError> { Ok(()) }
# unsafe fn demo(info: duckdb_extension_info, access: *const duckdb_extension_access) -> bool {
quack_rs::entry_point::init_extension(info, access, DUCKDB_API_VERSION, |con| {
    unsafe { register_word_count(con) }?;
    unsafe { register_sentence_count(con) }?;
    unsafe {
        SqlMacro::scalar("double_it", &["x"], "x * 2")?
            .register(con)?;
    }
    Ok(())
})
# }
```

### Can I use the `duckdb` crate instead of `libduckdb-sys`?

No. The `duckdb` crate's `bundled` feature embeds its own copy of DuckDB. A
loadable extension must link against the DuckDB that loads it, not bundle a
separate copy. Use `libduckdb-sys` with the `loadable-extension` feature.

### Can I have a scalar function with no parameters?

Yes. Just do not call `param`:

```rust
# use libduckdb_sys::{duckdb_connection, duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# use quack_rs::prelude::*;
# unsafe extern "C" fn quack_callback(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
unsafe {
    ScalarFunctionBuilder::new("current_quack")
        .returns(TypeId::Varchar)
        .function(quack_callback)
        .register(con)?;
}
# Ok(())
# }
```

---

## Testing

### Do I need a DuckDB instance to run unit tests?

No. `AggregateTestHarness` simulates the aggregate lifecycle in pure Rust
without any DuckDB dependency. You can run `cargo test` without loading a DuckDB
binary.

### My unit tests all pass but the extension crashes. Why?

Unit tests cannot detect FFI wiring bugs. See [Pitfall P3](reference/pitfalls.md#p3-e2e-testing-is-mandatory)
and the [Testing Guide](testing.md). Always run end-to-end tests that load the
packaged extension into a real DuckDB process.

### How do I test SQL macros?

`SqlMacro::to_sql()` is pure Rust and requires no DuckDB connection:

```rust
# use quack_rs::sql_macro::SqlMacro;
let m = SqlMacro::scalar("triple", &["x"], "x * 3").unwrap();
assert_eq!(m.to_sql(), r#"CREATE OR REPLACE MACRO "triple"("x") AS (x * 3)"#);
```

For an end-to-end test, call the macro from your SQLLogicTest file:

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
names must be unique among DuckDB community extensions; check
[community-extensions.duckdb.org](https://community-extensions.duckdb.org/)
before you pick one.

### Do I need to set up CI manually?

No. `generate_scaffold` produces `.github/workflows/extension-ci.yml` which
builds and tests your extension on Linux, macOS, and Windows automatically.

### Can my extension be installed with `INSTALL ... FROM community`?

Yes, once your pull request is merged into the community-extensions repository.
Until then, users load the `.duckdb_extension` file directly, starting DuckDB with
`-unsigned` because a local build is not signed:

```sql
LOAD './path/to/my_extension.duckdb_extension';
```

---

## Troubleshooting

### My aggregate returns wrong results with no error.

The most common cause is Pitfall L1: your `combine` callback is not propagating
all configuration fields. See
[Pitfall L1](reference/pitfalls.md#l1-combine-must-propagate-all-config-fields)
and test with `AggregateTestHarness::combine`.

### The NULLs my function writes come back as values.

You are likely calling `duckdb_vector_get_validity` without first calling
`duckdb_vector_ensure_validity_writable`. A vector that has never held a NULL
has no validity mask, so `duckdb_vector_get_validity` returns a null pointer and
`duckdb_validity_set_row_invalid` silently does nothing (dereferencing that
pointer yourself crashes). Use `VectorWriter::set_null` instead. See
[Pitfall L4](reference/pitfalls.md#l4-ensure_validity_writable-is-required-before-null-output).

### My function is not found in SQL after `LOAD`.

If `LOAD` succeeded, the entry point ran, so check the name you gave the
builder. If you build a function set through the raw C API instead of
quack-rs's set builders, every member needs its own name, or DuckDB silently
skips it ([Pitfall L6](reference/pitfalls.md#l6-function-set-name-must-be-set-on-each-member)).

If `LOAD` itself fails, check the entry point symbol. DuckDB takes the file's
base name, lowercases it and calls `<base name>_init_c_api`, so
`my_extension.duckdb_extension` needs `my_extension_init_c_api`
([Pitfall P1](reference/pitfalls.md#p1-library-name-must-match-extension-name)).

### `make configure` fails with a missing file error.

The `extension-ci-tools` submodule is missing. In a new project, such as one
fresh from the scaffold, add it once (`git submodule update --init` does nothing
until it has been added):

```bash
git submodule add https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
```

In a clone of a repository that already has the submodule:

```bash
git submodule update --init --recursive
```

See [Pitfall P4](reference/pitfalls.md#p4-extension-ci-tools-submodule-must-be-initialized).

### My SQLLogicTest fails in CI but passes locally.

SQLLogicTest does exact string matching. The most common issue is a difference
in NULL representation, decimal places, or line endings. Run the query in the
same DuckDB version used by CI and copy the output verbatim.

### How do I read a VARCHAR that is longer than 12 bytes?

`VectorReader::read_str` handles both the inline (≤ 12 bytes) and pointer
(> 12 bytes) formats automatically. No special handling needed.

### What happens if I read from a NULL row?

You get garbage data from the vector's data buffer — and for `VARCHAR` or
`BLOB`, possibly a stale pointer into freed memory. Always check `is_valid`
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

### Why must the release profile use `panic = "unwind"`?

quack-rs wraps every entry point and every callback it generates in
`catch_unwind` and reports a panic as an ordinary SQL error (a raw
`extern "C"` callback you write yourself needs a `*_callback!` macro or
`catch_ffi_panic`; see [Installation](getting-started/installation.md#why-panic--unwind-not-abort)).
`catch_unwind` cannot catch anything under
`panic = "abort"`: the process terminates at the panic site, taking the user's
DuckDB session with it. `validate_release_profile` rejects `abort` for this
reason. Returning `Result` and using `?` is still the right style — the guards
are a safety net, not a substitute.

### Can I use async Rust in my extension?

Not directly in FFI callbacks. DuckDB's callbacks are synchronous C functions.
You can run an async runtime such as Tokio and block on async tasks inside
callbacks (for example with `Runtime::block_on`), but the callbacks themselves
must return synchronously.

### How does `FfiState<T>` prevent double-free?

Each state slot starts with a tag that `init_callback` writes once the `T`
is in place. `destroy_callback` drops the `T` (freeing its box, when `T` is
too large to store inline) only when the tag matches, and clears the tag
first. A second call to `destroy_callback` on the same state finds no tag and
does nothing.
