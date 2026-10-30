# ABI Compatibility

This page explains DuckDB C Extension API ABI compatibility: which DuckDB releases a
quack-rs extension binary can load into, and how quack-rs guards against a layout
mismatch. A loadable extension does not link against DuckDB's symbols. DuckDB hands it a
pointer to a `duckdb_ext_api_v1` struct — an array of function pointers — and the
extension calls through it.

Two things have to agree about that struct's layout: the header your extension
was **compiled** against (whichever `libduckdb-sys` version Cargo resolved), and
the DuckDB binary that is **loading** it. When they disagree, every call lands on
the wrong slot.

## The struct has two halves

| Region | Slots | Guarantee |
|--------|-------|-----------|
| Stable | `0 .. 357` | Frozen since DuckDB v1.2.0 — same slots, same order, same signatures in every release through v2.0.0 (two slots, 114 and 138, were renamed `varint` → `bignum` in v1.4.0 with an identical struct layout) |
| Unstable | `357 ..` | Before v1.5.6, DuckDB twice **inserted** new entries in the middle, shifting every later slot; v2.0.0 only appended |

The stable prefix is what makes "build once, load anywhere" possible. The
unstable tail has not always been append-only:

| DuckDB | Total slots | What moved |
|--------|-------------|------------|
| v1.2.0 – v1.2.2 | 408 | baseline |
| v1.3.0 – v1.3.2 | 428 | appended |
| v1.4.0 – v1.4.5 | 459 | `duckdb_create_varint` → `duckdb_create_bignum`; appended |
| v1.5.0 – v1.5.1 | 545 | `duckdb_appender_clear` **inserted** at slot 410 |
| v1.5.2 – v1.5.6 | 546 | `duckdb_geometry_type_get_crs` **inserted** at slot 493 |
| v2.0.0 | 548 | `duckdb_create_timestamp_tz_ns`, `duckdb_get_timestamp_tz_ns` appended |

DuckDB v1.5.6 declares all 546 slots stable for extensions that target C API
v1.5.6 (earlier releases declared only the first 357 stable). The layout itself
is unchanged from v1.5.2. DuckDB v2.0.0 still declares C API v1.5.6 for this
struct, and its other 546 declarations are textually identical to v1.5.6's, so a
build against the v1.5.2 – v1.5.6 bindings dispatches correctly on v2.0.0.
`scripts/check-abi-table.py` derives this table from each release's
`duckdb_extension.h`.

Every family from v1.2 to v1.5 changed the unstable tail, and twice (v1.5.0 and
v1.5.2) an insertion in the middle shifted every later slot.

## Which half are you using?

Everything quack-rs exposes by default lives in the stable prefix: scalar,
aggregate, table and cast functions, vectors, data chunks, values, SQL macros,
replacement scans, the [`query`](../data/values-and-parameters.md) API and the
`datetime` conversions.

The `duckdb-1-5`, `duckdb-1-5-3` and `duckdb-1-5-4` features wrap the unstable
half — 130 of its 189 functions, covering scalar bind/init, copy functions in both
directions, the Arrow C Data Interface bridge, catalog access, `ErrorData`,
`FileSystem`, `Expression`, `SelectionVector`, config options, table descriptions,
`TIME_NS` values and the client context.

`abi::uses_unstable_api()` reports which half a build uses. The book's own test
build enables `duckdb-1-5-4` (and therefore `duckdb-1-5`), so this block is
compiled there but not run:

```rust,no_run
use quack_rs::abi;

// false unless `duckdb-1-5` is enabled.
assert!(!abi::uses_unstable_api());
```

## Why DuckDB does not catch this for you

DuckDB validates the ABI metadata in your extension's footer:

| ABI type | Version field (`-dv`) means | Accepted by |
|----------|-------------|-------------|
| `C_STRUCT` | the **C API** version (`v1.2.0` or `v1.5.6`) | any DuckDB whose C API version is at least that — then handed the *whole* struct, unstable region included. v1.2.0 – v1.5.5 declare C API `v1.2.0`; v1.5.6 and v2.0.0 declare `v1.5.6` |
| `C_STRUCT_UNSTABLE` | an exact **DuckDB release** (`v1.5.5`) | that 1.x release only; DuckDB 2.0 sends every such binary to the C API v2 entry point `<name>_init_c_api_v2`, which quack-rs does not generate, so it does not load there |

So a `C_STRUCT` `v1.2.0` binary that touches the unstable region loads happily
into the wrong DuckDB and then mis-dispatches. DuckDB's own `extension-template-c` says:

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

**Build metadata.** If you enable `duckdb-1-5`, build against the v1.5.2 – v1.5.6
bindings (`libduckdb-sys` `~1.10506.0`) and stamp the binary `C_STRUCT` with C API
`v1.5.6` (`quack_rs::DUCKDB_API_VERSION_1_5_6`): `append_metadata --abi-type C_STRUCT
--duckdb-version v1.5.6`. DuckDB v1.5.5 and earlier then refuse it at `LOAD` ("The
file was built for DuckDB C API version 'v1.5.6'..."), and v1.5.6 and v2.0.0 load
it. With `extension-ci-tools`, set in the `Makefile`:

```makefile
TARGET_DUCKDB_VERSION=v1.5.6
# and leave USE_UNSTABLE_C_API unset (or 0)
```

`generate_scaffold` writes this from a `ScaffoldConfig`, pins `libduckdb-sys` to
`~1.10506.0`, and rejects a `target_duckdb_version` that does not match
`use_unstable_c_api`:

```rust
use quack_rs::scaffold::ScaffoldConfig;

let config = ScaffoldConfig {
    name: "my_ext".to_string(),
    target_duckdb_version: quack_rs::DUCKDB_API_VERSION_1_5_6.to_string(),
    ..ScaffoldConfig::default()
};
```

Stamping `C_STRUCT_UNSTABLE` with the exact DuckDB release
(`USE_UNSTABLE_C_API=1`, `use_unstable_c_api: true`) still pins a binary to one
1.x release, but DuckDB 2.0 cannot load such a binary, and `extension-ci-tools`
`v2.0-cyanoptera` still stamps `C_STRUCT_UNSTABLE` when `USE_UNSTABLE_C_API=1`.
`append_metadata` and the scaffold refuse `C_STRUCT_UNSTABLE` with a 2.x release.

**Runtime guard.** When a `duckdb-1-5*` feature is enabled, [`abi::check`] compares
the compiled-in slot count against the layout the running engine uses, resolved from
`duckdb_library_version()` — which lives at stable slot 7 and is therefore always
dispatched correctly. (Without those features the check reports `StableOnly` and
always passes.) The entry point applies it according to an [`AbiPolicy`]:

| Policy | Behaviour |
|--------|-----------|
| `Strict` (default) | Refuse to load, with a message naming both layouts and the fix |
| `AllowUnknownEngine` | Refuse a layout the table knows is different; allow a release the table has no entry for |
| `Warn` | Print the diagnostic to stderr, then load anyway (`set_error` would fail the load) |
| `Trust` | Skip the check |

`AllowUnknownEngine` and `Trust` are only as safe as your knowledge of the
engine: calling into the unstable region of a layout the extension was not built
for is undefined behaviour. Before DuckDB v1.4.5 was in quack-rs's table, a
v1.5.5 build loaded into v1.4.5 under `AllowUnknownEngine` and segfaulted.

The two lines below are alternatives: both define the same exported symbol, so
they do not compile together.

```rust,ignore
use quack_rs::abi::AbiPolicy;

// Default: Strict.
quack_rs::entry_point!(my_ext_init_c_api, register);

// Explicit — appropriate when the binary's metadata already makes DuckDB refuse
// the wrong release: C_STRUCT_UNSTABLE on 1.x. For C_STRUCT v1.5.6 (built
// against the v1.5.6 bindings) DuckDB refuses every release before v1.5.6,
// and Trust then rests on DuckDB's statement that C API v1.5.6 is stable;
// the default Strict guard checks the engine's layout instead.
quack_rs::entry_point!(my_ext_init_c_api, AbiPolicy::Trust, register);
```

A refused load looks like this (a v1.5.0 build loaded into DuckDB v1.5.5):

```text
DuckDB C extension API layout mismatch: this extension was built against a
duckdb_ext_api_v1 with 545 slots, but DuckDB v1.5.5 provides 546. The extension
uses the unstable region of the C API (quack-rs feature `duckdb-1-5`), whose slot
indices differ between these releases, so loading it would dispatch to the wrong
functions. Rebuild the extension against DuckDB v1.5.5. Stamp a build that uses
the unstable region with `--abi-type C_STRUCT --duckdb-version v1.5.6` (with
extension-ci-tools: `TARGET_DUCKDB_VERSION=v1.5.6` and no `USE_UNSTABLE_C_API`)
so DuckDB itself refuses it before v1.5.6, from which that layout is stable. Not
`C_STRUCT_UNSTABLE`: DuckDB 2.0 loads such a binary only through the C API v2
entry point, which quack-rs does not generate.
```

For an engine older than v1.5.0 the advice changes: such an engine cannot run a
`duckdb-1-5` build at all, so the message says to build a variant without those
features or to upgrade DuckDB.

## Unknown DuckDB versions

`Strict` also refuses a DuckDB release quack-rs has no verified layout for — a
newer release (such as v2.0.1 or v2.1.0), or a `-dev` build. That is deliberate:
DuckDB changed the unstable region in every minor release from 1.3.0 to 2.0.0, and
in the 1.5.2 patch release, so "unknown" is not evidence of "compatible". DuckDB's
own header says the same: DuckDB v2.0.0's `duckdb_extension.h` opens with "this API
is not yet stable, meaning it is only guaranteed to work for this specific DuckDB
version", and v2.0.0 itself added two slots to the band that v1.5.6 had declared
stable. So a `duckdb-1-5*` extension that loads into 1.5.6 and 2.0.0 is refused by
2.0.1 until a quack-rs release lists it or you rebuild with
`QUACK_RS_TARGET_DUCKDB_VERSION=v2.0.1`; an extension built with the default
features reports `StableOnly` and loads there. A build
against the v1.5.2 – v1.5.6 bindings is accepted on v2.0.0, whose layout only
appends to it; a build against 1.5.0/1.5.1 (545 slots) is still refused there.
The refusal lists the fixes, best first:

1. rebuild against the release you are targeting and set
   `QUACK_RS_TARGET_DUCKDB_VERSION` to it, which turns the check into a
   positive match without waiting for a quack-rs release;
2. upgrade quack-rs to a version whose layout table lists that release;
3. if the extension does not need the unstable region, build it without the
   `duckdb-1-5` features: the stable prefix is laid out identically in every
   DuckDB from v1.2.0 through v2.0.0.

It never suggests `AllowUnknownEngine` or `Trust`, for the reason above.

`scripts/check-abi-table.py` re-derives quack-rs's layout table from every
upstream release header and runs in CI, so the table tracks DuckDB.

[`abi::check`]: https://docs.rs/quack-rs/latest/quack_rs/abi/fn.check.html
[`AbiPolicy`]: https://docs.rs/quack-rs/latest/quack_rs/abi/enum.AbiPolicy.html
