# Upgrading from 0.18 to 0.20

This page covers the two steps after [0.16 to 0.18](upgrading-0-18.md): 0.18 to 0.19.0,
and 0.19.0 to 0.20.0. The [changelog](../reference/changelog.md) is the full record; this
page says what to look for. Neither 0.19.0 nor 0.20.0 removes a public item that a correct
extension used. 0.20.0 has two changes that can stop a build (see "Changes that stop a
build"); several calls that used to succeed now return an error, which can fail a `LOAD`
that worked before, and a few results that were wrong are now right.

To upgrade, set the version, build, run your SQL tests, and then load the built extension
into every DuckDB release you support:

```toml
[dependencies]
quack-rs = "0.20"

[dependencies.libduckdb-sys]
version = ">=1.4.4, <1.20100"
default-features = false
features = ["loadable-extension"]
```

## From 0.18 to 0.19.0

You need 0.19.0 to support DuckDB 2.0.0. On 2.0.0, quack-rs 0.18.0 refuses to load a
build that uses the `duckdb-1-5*` features, every casting `Value` getter (`as_i64`,
`as_str`, …) returns `None`, and `bundled-test` no longer builds once Cargo resolves
`libduckdb-sys` 1.20000.x (a fresh resolve on Rust 1.88 or newer).

### An extension that uses the `duckdb-1-5*` features

DuckDB 2.0 cannot load a `C_STRUCT_UNSTABLE` binary through the entry point quack-rs
generates, so the stamp 0.18 advised no longer works there. Rebuild against the 1.5.6
bindings and stamp the binary with the C API version instead:

| In 0.18 | From 0.19 |
|---------|-----------|
| `append_metadata … --abi-type C_STRUCT_UNSTABLE --duckdb-version v1.5.5` (the exact release) | `append_metadata … --abi-type C_STRUCT --duckdb-version v1.5.6`, built against `libduckdb-sys = "~1.10506.0"`. DuckDB 1.5.6 and 2.0.0 load it; earlier releases refuse it, and the default `Strict` guard refuses a release quack-rs has not listed (2.0.1, for now). |
| `TARGET_DUCKDB_VERSION=v1.5.5` and `USE_UNSTABLE_C_API=1` in the `Makefile` | `TARGET_DUCKDB_VERSION=v1.5.6` and no `USE_UNSTABLE_C_API`, with the `extension-ci-tools` submodule on the `v2.0-cyanoptera` branch |
| `ScaffoldConfig { use_unstable_c_api: true, target_duckdb_version: "v1.5.5", … }` | `target_duckdb_version: quack_rs::DUCKDB_API_VERSION_1_5_6.to_string()` and `use_unstable_c_api: false` |

An extension that uses only the default features needs no change: its `C_STRUCT`
`v1.2.0` binary loads into every DuckDB from 1.4.4 through 2.0.x. See
[ABI Compatibility](../concepts/abi.md).

### Behaviour changes

| Change | What to check |
|--------|---------------|
| `parse_description_yml` and `validate_description_yml_str`: a `repo.ref` that is not a commit hash is a warning; quoted values are no longer trimmed (`name: " my_ext"` is refused, as the community build refuses it); `validate_rust_extension` requires an exact `rust` token in `requires_toolchains` (`"python3; rust"` installs no Rust toolchain). | A `description.yml` that passed before can now fail. Run the validator on it. |
| `generate_scaffold` requires `version` to be semver (`2025120401`, `1.0` and `v1.0.0` are refused), and `ScaffoldConfig::default()` excludes `wasm_mvp;wasm_eh;wasm_threads;windows_amd64_mingw`. | Pass an explicit `excluded_platforms` if you build those platforms. A project generated earlier is not rewritten; diff a fresh one against yours. |
| `validate::platform`: `osx_amd64` is an opt-in platform, as in `extension-ci-tools` for DuckDB 2.0 (DuckDB 2.1 drops it). | Listing it in `excluded_platforms` has no effect; list it in `opt_in_platforms` to have it built. |
| On DuckDB 2.0.0, an appender error caused by an open stream reports `ResourceInUse` instead of `Invalid`. | Code that matched on the error category. |
| Registering several table functions through the entry point's `Connection` lists the catalog once per `LOAD`, not once per function. | Nothing: faster. A table function registered on the raw connection after that listing is not seen by the check (see [Table Functions](../functions/table-functions.md#registering-a-name-twice)). |
| `DuckDbSecretInfo`'s `Debug` prints the length of `secret_string`, not its text. | A test that compared the `Debug` output. |
| The published crate no longer contains `scripts/`, `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md` or `GOVERNANCE.md`, and `mutants` is a dev-dependency only. | Nothing, unless a build script of yours read those files from the registry source. |

## From 0.19.0 to 0.20.0

### Dependency requirement

0.20.0 requires `libduckdb-sys` `>=1.4.4, <1.20100`; 0.18.0 and 0.19.0 required
`>=1.4.4, <2`, which also admits bindings for DuckDB releases that do not exist yet.
The new bound stops before the DuckDB 2.1 bindings because `libduckdb-sys` keeps the C API
names at its crate root only through a re-export that upstream marks for removal; the
[Installation](installation.md#adding-quack-rs-to-an-existing-extension) page has the
reasoning and what happens when 2.1 bindings are published.

| Change | What to do |
|--------|------------|
| A project that asks for `libduckdb-sys` 1.20100 or newer no longer resolves with 0.20.0. | Remove that requirement. Cargo picks 1.20000.x, which is the DuckDB 2.0 bindings. |
| You stay on 0.18.0 or 0.19.0 and want the same ceiling. | Add `libduckdb-sys = ">=1.4.4, <1.20100"` to your own `Cargo.toml`; those releases cannot be changed after publication. |
| `default-features = false` on `libduckdb-sys` is now the documented form (the scaffold, the README and the book write it). | Optional. With the 1.20000 bindings it leaves out the HTTP and TLS download tooling their default features build. Set it in your own manifest too: a dependency elsewhere in the graph that leaves the defaults on turns them back on. |

### Changes that stop a build

| Change | What to do |
|--------|------------|
| `AbiCheck`, `AbiPolicy`, `ConfigOptionScope` and `TlsVersion` are `#[non_exhaustive]`, so DuckDB releases that add a variant do not break your `match`. An exhaustive `match` on one of them outside quack-rs no longer compiles (error E0004). Constructing the variants, comparing them and `TlsVersion`'s ordering are unchanged. | Add a `_` arm. |
| `entry_point!` and `entry_point_v2!` fail to compile when the extension is built with `panic = "abort"` (a profile setting, or `-C panic=abort` in `RUSTFLAGS`), because every panic guard in quack-rs is `catch_unwind`, which does nothing there. The error names the fix. An entry point written by hand around `init_extension` has no such check. | Set `panic = "unwind"` under `[profile.release]` (the scaffold already does). |

### Behaviour changes

| Change | What to do |
|--------|------------|
| Registering an aggregate, or an overload in a set, **with no parameter** is an error. `SELECT my_count() FROM t` counted 100 rows on DuckDB 1.5.6 and 0 on 2.0.0. If your registration function propagates the error with `?`, `LOAD` fails. | Give the aggregate one parameter and put a macro in front for the zero-argument spelling: `SqlMacro::scalar("my_count", &[], "my_count_1(1::BIGINT)")`. See [Known Limitations](../reference/known-limitations.md#aggregates-that-take-no-argument-count-no-rows-on-duckdb-20). |
| `TypeId::Variant` (feature `duckdb-1-5-3`) is refused in a function signature: `LogicalType::try_new`, and so every builder's `param`, `returns` and result-column slot, returns an error, and `LogicalType::new` panics. It registered before, and the first query that reached it crashed DuckDB 2.0.0 or invalidated the database on 1.5.x. | Declare the parameter as `TypeId::Any`, which accepts a `VARIANT`. |
| `ListVector::get_entry` and `MapVector::get_entry` return stale bytes for a NULL row (lengths around 7·10¹³ were seen on 2.0.0). | Read with `get_entry_checked`, which returns `None` for a NULL row and for an entry outside the child, or test `is_valid(row)` first. See [Complex Types](../data/complex-types.md). |
| `append_metadata` writes a sibling file and renames it over the output; it rewrote the output in place, which crashed a host that had the previous build loaded. It also warns when the library does not export `<output file name>_init_c_api`. | Nothing to change. The warning is not an error: stamping to a temporary name and renaming is ordinary. |
| `parse_description_yml` warns about a platform group name (`wasm`, `windows`, `linux`, `osx`) in `excluded_platforms` or `opt_in_platforms`, and about an `opt_in_platforms` entry that is not an opt-in platform. `DescriptionYml` gains `opt_in_platforms`. | If CI fails on warnings, write the platforms out: `wasm_mvp;wasm_eh;wasm_threads`. A group name excluded nothing in the community build. |
| `TIMESTAMPTZ_NS` (DuckDB 2.0.0) is supported: `TypeId::TimestampTzNs`, `VectorReader::read_timestamp_tz_ns`, `VectorWriter::write_timestamp_tz_ns`, `Value::timestamp_tz_ns` and `as_timestamp_tz_ns`. `TypeId::try_from_duckdb_type(42)` returned `None` before, and `LogicalType::get_type_id` panicked on such a column. | Code that treated id 42 as unknown. Before DuckDB 2.0.0, creating the type returns an error and the getter returns `None`. |
| The scaffold no longer generates `extension_config.cmake` or the `EXT_CONFIG` line of the `Makefile`; the generated `Cargo.toml` carries `license`; the generated `.cargo/config.toml` sets `-C target-feature=-crt-static` for the two musl targets. | In a project generated earlier, delete the two files and the line (no `extension-ci-tools` makefile reads them). Add the musl setting only if you opt into the musl platforms. |
| `VectorReader::is_valid`, `VectorWriter::set_null`, `ListBuilder` and `DuckStringView::as_str` do less work per row. | Nothing: the API is unchanged. |
| The duplicate-overload guard of `ScalarFunctionSetBuilder` and `AggregateFunctionSetBuilder` (and the catalog check of scalar registrations) compares `DECIMAL`, `MAP` and `ENUM` parameters by type alone, also inside `LIST`, `ARRAY`, `STRUCT` and `UNION`, as DuckDB resolves overloads. A set such as `f(DECIMAL(4,1))` with `f(DECIMAL(18,3))` registered before and then failed every call with "Could not choose a best candidate function"; it is now refused at registration, which fails a `LOAD` that propagates the error. | Give the overloads different names, or take `ANY` and branch on the type at run time. See [Scalar Functions](../functions/scalar.md#decimal-parameters-are-not-generic). |
| `WarningCollector` keeps at most 10,000 warnings between drains (`DEFAULT_MAX_WARNINGS`); later ones are counted in `dropped()`, not stored. Before, a collector that was never drained grew without bound. | Code that expects every warning to be stored: read `dropped()`, or build the collector with `WarningCollector::with_limit(n)`. |
| `data_chunk_from_arrow` refuses (`InvalidInput`) arrays that DuckDB would read out of bounds: list, large-list and map offsets that are negative, decrease or end past the child, a negative list-view offset or size, a child shorter than the rows its parent reads, a dictionary index outside the dictionary. | These arrays violate the Arrow specification; fix the producer. See [Arrow Interop](../duckdb-1-5/arrow.md). |
| A panicking `Clone` of `ScalarBindData` fails the query with the panic message; before, DuckDB's copy of the bound call silently had no bind data and the query answered as if none had been set. A second `ScalarBindData::set` in one bind frees the value it replaces (it was leaked). | Nothing, unless a test pinned the old answer. |
| `SqlMacro::register` refuses a `CREATE MACRO` text that does not parse, with DuckDB's parse message, instead of executing it. A body such as `1); DROP TABLE t; SELECT (` made DuckDB 2.0.0 run the `DROP` (1.4.4 and 1.5.6 parse the whole string first and run nothing). | Nothing, unless a macro body of yours did not parse and relied on the error. |
| `BindInfo::set_cardinality` clamps its hint to `i64::MAX` (above it, a join with the function failed with an internal error). | Nothing. |
| A `BIT` value whose bytes are malformed (only the raw `duckdb_create_bit` builds one) is refused by `Value::as_str`, `display_string` and `Debug`, which print `UNRENDERABLE`; DuckDB wrote out of bounds rendering it. | Nothing, unless a test printed such a value. |
| The entry point runs your registration closure in one transaction and rolls it back when the closure returns `Err` or panics, so a failed `LOAD` leaves no function, macro, type or table behind and can be retried in the same database. Casts, replacement scans and configuration options are not rolled back. A statement that fails inside the closure can abort the transaction (always on DuckDB 2.0), so every later statement fails with "Current transaction is aborted". | Register casts, replacement scans and configuration options last, return the first error instead of carrying on, and do not run `BEGIN`, `COMMIT` or `ROLLBACK` on the registration connection. Passing `Registration::Autocommit` through `options =` restores the earlier behaviour. See [The Entry Point](../concepts/entry-point.md#registration-runs-in-one-transaction). |
| A `duckdb-1-5*` build under the default `AbiPolicy::Strict` refuses a DuckDB release that quack-rs's layout table does not list. 2.0.1 is not listed yet: a build that loads into 1.5.6 and 2.0.0 refuses 2.0.1. Default-feature builds are unaffected. | Rebuild with `QUACK_RS_TARGET_DUCKDB_VERSION=v2.0.1` once you have checked that release, or upgrade quack-rs to a release that lists it. See [ABI Compatibility](../concepts/abi.md#unknown-duckdb-versions). |
| An extension named like a DuckDB alias or core extension (`s3`, `md`, `json`, ...) is refused by `generate_scaffold` and `parse_description_yml`. `INSTALL s3 FROM community` asks for `httpfs`, so such an extension could never be installed under its name. | Rename the extension. See [Names DuckDB reserves](../publishing.md#names-duckdb-reserves). |
| `parse_description_yml` and `validate_description_yml_str` read every section by PyYAML's rules and in linear time; they reject text that `yaml.safe_load` rejects (before, `docs:` and any section other than `extension:` and `repo:` were skipped by indentation) and read `repo.variegata` and `repo.andium`. | A `description.yml` that passed before can now fail; run the validator on it. |

### New API

- `ScalarFunctionSetBuilder::overload_typed`: closure-built functions as overloads of a set.
- `ChunkWriter::release_row`: give back a claimed row that the source cannot fill.
- `ScalarBindData::try_get`: the bind data, or the message of a `Clone` that panicked.
- `WarningCollector::with_limit` and `dropped`.
- `entry_point::{EntryPointOptions, Registration}`, taken by `options =` in the entry-point
  macros.
- `validate::validate_community_extension_name`, with the lists of reserved names.
- `Value::try_cast`, `array_items`, `union_tag` (`duckdb-1-5`), `bit`, `as_bit`, `bignum` and
  `as_bignum`: see [Values & Parameters](../data/values-and-parameters.md).
- `query::StatementType` with `PreparedStatement::statement_type` and `parameter_type`,
  `QueryResult::statement_type`; with `duckdb-1-5`, `PreparedStatement::columns`,
  `query::table_names` (and the same method on `OwnedConnection` and `Connection`) and
  `Appender::for_query`, an appender whose rows feed an `INSERT … SELECT` or `MERGE INTO`.
- `from_raw` and `into_raw` on `Appender`, `PreparedStatement`, `QueryResult` and
  `OwnedConnection`, for handles from a C call quack-rs does not wrap.
- Typed, tag-checked extra info (`extra_info_typed` on the aggregate, cast and `COPY` builders,
  read with `extra_info_ref::<T>()`) and typed `COPY TO` state: see
  [Copy Functions](../functions/copy-functions.md#typed-state-without-raw-pointers).
  `FileHandle` and `ClientContext` are `Send`. `ErrorData` implements `From<ExtensionError>`.

### New pitfalls

The [Pitfall Catalog](../reference/pitfalls.md) gained L20, L21, P16 and P17 (a log-storage
registration that frees its data twice, a chunk row that is published whether or not it was
written, `duckdb_get_bit` aborting on NULL, and `std::fs` bypassing DuckDB's permission
settings) and corrected L3 and L11. If your extension reads files, read
[P17](../reference/pitfalls.md#p17) first: a table function or `COPY` reader that opens a
path from SQL with `std::fs` ignores `SET enable_external_access = false`; open it with
`FileSystem`.
