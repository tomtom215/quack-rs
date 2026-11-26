<!-- SPDX-License-Identifier: MIT -->

# A DuckDB C API v2 backend for quack-rs: design notes

**Status: design only. Nothing here is implemented.** quack-rs targets C API v1
(`duckdb_extension.h`); this note and `inventory-v2.0.1.csv` beside it are the
starting point for a v2 backend, written in answer to QRS-021, QRS-022 and QRS-023
of the maintainer review of 2026-11-25 (`AUDIT.md` section 16). The owner chose, that
day, a design and an inventory now and no implementation.

## What v2 is, from the 2.0.1 headers and bindings

Read in `duckdb_extension_v2.h`, `duckdb_v2.h` and `duckdb_static_extension.h` from
the v2.0.1 release, and in `libduckdb-sys` 1.20001.0 (each claim below was checked
there on 2026-11-25):

- **A separate API, not a superset.** The v2 function table has 641 slots
  (`duckdb_ext_api_v2`), the v1 table 548. Only 18 v2 names have a v1 slot of the
  same name once the `v2_` is dropped (the inventory's `v1_name` column), and the
  header says an extension "targets either the V1 or the V2 C API, never both".
- **No stable surface yet.** `duckdb_extension_v2.h` stops the build with `#error`
  unless the unstable surface is enabled: "No V2 function has been stabilized yet",
  and building with it "ties the extension to this DuckDB version". A v2 extension is
  therefore an exact-version binary (`C_STRUCT_UNSTABLE`), rebuilt for every DuckDB
  release, where a v1 quack-rs extension with default features loads into every
  release from 1.4.4 to 2.0.x.
- **One error model.** Every function returns a `DUCKDB_V2_ERROR` status and takes a
  `duckdb_v2_error_info_handle *err` out-parameter, where v1 mixes `duckdb_state`,
  null returns, `set_error` on an info handle and, in places, aborts. A safe wrapper
  can map every call to `Result` the same way.
- **Factories.** Types, values, functions and collections are created through a
  `duckdb_v2_factory_handle` from an instance, a connection or a context, valid as
  long as its source and never destroyed by the caller.
- **Entry point.** `<name>_init_c_api_v2(struct duckdb_v2_extension_input *input)`,
  registering through `duckdb_v2_extension_register_*` (scalar, aggregate, table,
  cast, custom type, replacement scan, copy, multi-file, pragma, custom option).
  quack-rs's `entry_point_v2!` is not this: it is the second version of quack-rs's
  own v1 entry-point macro.
- **Static linking.** `duckdb_static_extension.h` defines descriptor version 2 with
  both `entry_capi_v1` and `entry_capi_v2`. For a v1 entry DuckDB passes a null
  `get_api` (`DuckDB::LoadStaticCAPIExtension`, `extension_load.cpp`), which
  quack-rs's v1 entry point refuses (Known Limitations).
- **Rust bindings exist.** `libduckdb-sys` 1.20001.0 has a `capi-v2` feature exposing
  `libduckdb_sys::v2` and a loadable-extension initialiser
  `duckdb_rs_extension_api_v2_init`. Like the v1 initialiser it fills process-wide
  statics, so the one-engine-per-process limit applies to v2 as well.

## Approaches

| | A. `capi-v2` feature in quack-rs | B. Separate crate, shared core | C. Wait for a stable v2 surface |
|---|---|---|---|
| Shape | A feature that switches the whole crate to v2; `compile_error!` if combined with v1 | `quack-rs-v2` beside `quack-rs`; pure-Rust code (validation, scaffolding, datetime, interval, YAML) moved to a shared crate | Keep the inventory current per release; build nothing |
| Cost | Every module gains `cfg` branches; Cargo feature unification can select both backends in one build graph, which only the `compile_error!` catches | A workspace split, and a move of the shared code first | None now |
| Risk | Highest: v1's portability is the product, and a mistake in one branch reaches every user | Contained: v1 users are unaffected by v2 work | v2 users wait |
| Fit with "no stable surface" | Poor: an exact-version backend inside a version-portable crate | Good: the v2 crate can version with DuckDB | Good |

**Recommendation: C until DuckDB marks the first v2 functions stable, then B.**
While every v2 function is unstable, a v2 backend is a binary tied to one DuckDB build,
and wrappers written against 2.0.1 may not compile against 2.0.2; the cost of tracking
that across 641 slots is ongoing, not one-off. B keeps that cost out of the v1 crate.
Revisit when a DuckDB release stabilises part of v2, or when a user needs a capability
only v2 has (the inventory shows which: `stats` callbacks, `multi_file`, `log_sink`,
`arena`, `bignum` encode and decode, and others with no v1 counterpart).

## Work packages, when B starts

Ordered so that each package rests on the ones before it. Families are the
inventory's `family` column.

1. **Handles, errors, entry point** (`extension`, `instance`, `connection`, `context`,
   `error_info`, `factory`): one `Result` mapping for `DUCKDB_V2_ERROR`; ownership
   per handle class (owned, borrowed from a callback, borrowed from a factory); the
   `_init_c_api_v2` entry macro with an exact-version check before any other call.
2. **Types and values** (`logical_type`, `value`, `bignum_*`): constructors through a
   factory, checked as quack-rs's v1 `Value` constructors are.
3. **Vectors and chunks** (`vector`, `data_chunk`, `arena`): borrow-scoped views tied
   to the callback, so a view cannot outlive its chunk in safe code; capacity known to
   every writer (the lesson of QRS-008).
4. **Scalar functions**, then **aggregates** (with the v1 lifecycle lessons: a
   destructor for every state, tags or an equivalent for states `init` never ran),
   then **table functions** (projection, filters, partitions, `claim_batch`).
5. **COPY, casts, replacement scans, custom types, options, pragmas, logs.**
6. **Query and Arrow** (`connection` queries, `arrow_*`, `column_data_collection`).
7. **Static backend**: descriptor version 2 with `entry_capi_v2`, tested in a Rust
   host that links DuckDB.

## Acceptance, per package

- Every slot the package covers has a `disposition` in the inventory (safe wrapper,
  unsafe escape hatch, internal, host-only, or excluded with a reason) and a test ID.
- A safe wrapper has a positive test, an error test and, where it hands out a view, a
  lifetime test (a `compile_fail` doctest that the view cannot escape).
- The extension built for DuckDB vX.Y.Z loads there and is refused, before any other
  call, by every other release in the CI matrix.
- No build graph can contain both backends.

## Keeping the inventory current

```bash
scripts/fetch-duckdb.sh vX.Y.Z lib /tmp/duckdb-vX.Y.Z
python3 scripts/c-api-v2-inventory.py --include /tmp/duckdb-vX.Y.Z \
    --out docs/c-api-v2/inventory-vX.Y.Z.csv
```

The script fails if the header's slot list and its `DUCKDB_C_API` declarations name
different functions. For v2.0.1 it reports 641 slots in 99 families, 18 with a v1 slot
of the same name, 17 of those used by quack-rs; its 641 names equal the reviewer's
independently extracted list. The `disposition` column starts as `unclassified`:
classifying a slot is the design work, and the script does not guess it.
