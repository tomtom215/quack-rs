# ABI Compatibility

A loadable extension does not link against DuckDB's symbols. DuckDB hands it a
pointer to a `duckdb_ext_api_v1` struct — an array of function pointers — and the
extension calls through it.

Two things have to agree about that struct's layout: the header your extension
was **compiled** against (whichever `libduckdb-sys` version Cargo resolved), and
the DuckDB binary that is **loading** it. When they disagree, every call lands on
the wrong slot.

## The struct has two halves

| Region | Slots | Guarantee |
|--------|-------|-----------|
| Stable | `0 .. 357` | Frozen since DuckDB v1.2.0 — same functions, same order, in every release through v1.5.5 |
| Unstable | `357 ..` | DuckDB **inserts** new entries in the middle, shifting every later slot |

The stable prefix is what makes "build once, load anywhere" possible. The
unstable tail is not append-only:

| DuckDB | Total slots | What moved |
|--------|-------------|------------|
| v1.2.0 – v1.2.2 | 408 | baseline |
| v1.3.0 – v1.3.2 | 428 | appended |
| v1.4.0 – v1.4.4 | 459 | `duckdb_create_varint` → `duckdb_create_bignum`; appended |
| v1.5.0 – v1.5.1 | 545 | `duckdb_appender_clear` **inserted** at slot 410 |
| v1.5.2 – v1.5.5 | 546 | `duckdb_geometry_type_get_crs` **inserted** at slot 493 |

Four out of the last four minor/patch families moved something.

## Which half are you using?

Everything quack-rs exposes by default lives in the stable prefix: scalar,
aggregate, table and cast functions, vectors, data chunks, values, SQL macros,
replacement scans, the [`query`](../data/values-and-parameters.md) API and the
`datetime` conversions.

The `duckdb-1-5` and `duckdb-1-5-3` features are the unstable half — 105
functions covering scalar bind/init, copy functions, catalog access, `ErrorData`,
`FileSystem`, `Expression`, `SelectionVector`, config options, table descriptions
and the client context.

```rust,ignore
use quack_rs::abi;

// false unless `duckdb-1-5` is enabled.
assert!(!abi::uses_unstable_api());
```

## Why DuckDB does not catch this for you

DuckDB validates the ABI metadata in your extension's footer:

| ABI type | `-dv` means | Accepted by |
|----------|-------------|-------------|
| `C_STRUCT` | the **C API** version (`v1.2.0`) | any DuckDB whose C API version is at least that — then handed the *whole* struct, unstable region included |
| `C_STRUCT_UNSTABLE` | an exact **DuckDB release** (`v1.5.5`) | that release only |

So a `C_STRUCT` binary that touches the unstable region loads happily into the
wrong DuckDB and then mis-dispatches. DuckDB's own `extension-template-c` says:

> WARNING: When set to 1, the `duckdb_extension.h` from the
> `TARGET_DUCKDB_VERSION` must be used, using any other version of the header is
> unsafe.

Built against v1.5.0's headers and loaded into v1.5.5, an extension calling
`ClientContext::from_connection` invokes `duckdb_destroy_client_context` on a
`duckdb_connection`. In practice:

```text
double free or corruption (out)
Aborted (core dumped)
```

## What quack-rs does

Two layers.

**Build metadata.** If you enable `duckdb-1-5`, stamp the binary
`C_STRUCT_UNSTABLE` with the DuckDB release you built against, so DuckDB refuses
the wrong engine at install time:

```makefile
USE_UNSTABLE_C_API=1
TARGET_DUCKDB_VERSION=v1.5.5
```

`ScaffoldConfig` generates and validates this pairing:

```rust
use quack_rs::scaffold::ScaffoldConfig;

let config = ScaffoldConfig {
    name: "my_ext".to_string(),
    use_unstable_c_api: true,
    target_duckdb_version: "v1.5.5".to_string(),
    ..ScaffoldConfig::default()
};
```

**Runtime guard.** [`abi::check`] compares the compiled-in slot count against the
layout the running engine uses, resolved from `duckdb_library_version()` — which
lives at stable slot 7 and is therefore always dispatched correctly. The entry
point applies it according to an [`AbiPolicy`]:

| Policy | Behaviour |
|--------|-----------|
| `Strict` (default) | Refuse to load, with a message naming both layouts and the fix |
| `Warn` | Report through `set_error`, then load anyway |
| `Trust` | Skip the check |

```rust,ignore
use quack_rs::abi::AbiPolicy;

// Default: Strict.
quack_rs::entry_point!(my_ext_init_c_api, register);

// Explicit — appropriate when the binary is stamped C_STRUCT_UNSTABLE, because
// DuckDB already refuses to load it into the wrong release.
quack_rs::entry_point!(my_ext_init_c_api, AbiPolicy::Trust, register);
```

A refused load looks like this:

```text
DuckDB C extension API layout mismatch: this extension was built against a
duckdb_ext_api_v1 with 545 slots, but DuckDB v1.5.5 provides 546. The extension
uses the unstable region of the C API (quack-rs feature `duckdb-1-5`), whose slot
indices differ between these releases, so loading it would dispatch to the wrong
functions. Rebuild the extension against DuckDB v1.5.5, and stamp it with
`--abi-type C_STRUCT_UNSTABLE --duckdb-version v1.5.5` (or `USE_UNSTABLE_C_API=1`
with extension-ci-tools) so this is caught at install time.
```

## Unknown DuckDB versions

`Strict` also refuses a DuckDB release quack-rs has no verified layout for — a
newer release, or a `-dev` build. That is deliberate: DuckDB changed the unstable
region in every recent release, so "unknown" is not evidence of "compatible".
Rebuild against the release you are targeting, or set `AbiPolicy::Trust` if you
have verified the layout yourself.

`scripts/check-abi-table.py` re-derives quack-rs's layout table from every
upstream release header and runs in CI, so the table tracks DuckDB.

[`abi::check`]: https://docs.rs/quack-rs/latest/quack_rs/abi/fn.check.html
[`AbiPolicy`]: https://docs.rs/quack-rs/latest/quack_rs/abi/enum.AbiPolicy.html
