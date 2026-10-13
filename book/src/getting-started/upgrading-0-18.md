# Upgrading from 0.16 to 0.18

quack-rs 0.18 follows 0.16 directly; 0.17 was never published. Most of its
breaking changes close holes where safe code could crash DuckDB, return a wrong
answer or abort the process. Many need no change in a correct extension. This
page lists each change marked **Breaking:** in the
[changelog](../reference/changelog.md), grouped by area, with what to change.

To upgrade, set the version in `Cargo.toml`, build, and work through the
compiler errors with this page open:

```toml
[dependencies]
quack-rs = "0.18"
```

Then run your SQL tests. A few changes are behavioural (a registration that used
to succeed now returns an error) and show up only at run time.

## Callback macros

| Change | What to change |
|--------|----------------|
| A `*_callback!` body must have type `()` (a `cast_callback!` body still returns `bool`). | Remove `?` and trailing `Ok(())` from bodies. Report errors through the callback's `set_error` and `return`. |
| A macro body is no longer an `unsafe` context. | Wrap each unsafe call or raw-pointer read in its own `unsafe { … }` block. |
| Scalar bind and init callbacks take `RawScalarBindInfo` / `RawScalarInitInfo` (in `ScalarBindFn`, `ScalarInitFn`, `scalar_bind_callback!`, `scalar_init_callback!`, `ScalarBindInfo::new`, `ScalarInitInfo::new`). | For a hand-written scalar bind/init `extern "C" fn`, change the parameter type to `RawScalarBindInfo` / `RawScalarInitInfo`. Use `scalar_bind_callback!` / `scalar_init_callback!`, never the `table_*` macros, for scalar functions. |

See [Panic-Safe Callbacks](../functions/panic-safe-callbacks.md).

## Aggregates

| Change | What to change |
|--------|----------------|
| `FfiState<T>` stores a small `T` inline; its layout is private and the public `inner: *mut T` field is gone. Each state carries a tag that `destroy` checks. | Stop reading `inner`. Use `FfiState::<T>::with_state` / `with_state_mut` and the `size_callback`, `init_callback`, `destroy_callback` (or `ffi_state::<T>()`). |
| `AggregateState` requires `Sync`. | Replace `Cell` / `RefCell` fields in the state with atomics or a `Mutex`, or keep that data outside the state. |

## Scalar functions

| Change | What to change |
|--------|----------------|
| `ScalarFunctionBuilder::register` and `ScalarFunctionSetBuilder::register` (and `map1`, `map2`, …) refuse a parameter signature `duckdb_functions()` already lists under that name, built-ins included. | Rename the function, or change its signature, if registration now fails. Such an overload used to silently replace the existing one. |
| Overloads that DuckDB's binder would find ambiguous with varargs are refused at registration. | Remove or change the overlapping overload. |
| `validate_function_name` refuses the 53 keywords DuckDB cannot call unquoted (`DUCKDB_UNCALLABLE_KEYWORDS`); `SqlMacro` parameters are checked with `validate_parameter_name`. | Rename a function called, for example, `coalesce`, or a macro parameter that is a keyword. |
| On DuckDB 1.5.0 to 1.5.4, `ScalarBindInfo::argument` returns nothing and fails the bind with an explanation. | Do not rely on bind-time argument inspection on those releases, or require DuckDB 1.5.5+. |

## Table functions

| Change | What to change |
|--------|----------------|
| `TableFunctionBuilder::with_state` requires `S: Clone + Send`; every execution scans a fresh clone. | Derive `Clone` on the state, or switch to the new `with_bind_init` for state that cannot be cloned. |
| `TypedTableFunctionBuilder::projection_pushdown` is removed. | Use the raw `TableFunctionBuilder` for projection pushdown. |
| `TypedTableFunctionBuilder::build` errors if `projection_pushdown(true)` was set before `with_state` / `with_bind_init`. | Remove the `projection_pushdown(true)` call, or use the raw builder. |
| `FfiBindData::set` and `FfiInitData::set` require `T: Send + Sync`; `FfiLocalInitData::set` requires `T: Send`. | Make the data thread-safe: `Arc`/`Mutex`/atomics in place of `Rc`/`Cell`/`RefCell`. |
| `InitInfo::projected_column_index` returns `Option<usize>`. | Handle `None` (past the end of the projection) instead of treating 0 as a column. |
| `TableFunctionBuilder::register` refuses a name that is already a table function or table macro. | Choose a name that is not taken (`range`, another extension's function, your own earlier registration). |

## Replacement scans

| Change | What to change |
|--------|----------------|
| `ReplacementScanBuilder::register_with_data` and `Connection::register_replacement_scan_with_data` require `T: Send + Sync`. | Make the data thread-safe. |

## Copy functions

| Change | What to change |
|--------|----------------|
| `CopyFunctionBuilder` is no longer `Send` or `Sync`. | Build and register it on the same thread; do not move an unregistered builder across threads. |
| `CopyBindInfo::column_type` returns `Option<LogicalType>`. | Handle `None` for an out-of-range index. |
| `CopyGlobalInitInfo::get_file_path` returns `Result<String, ExtensionError>`. | Handle the error (a path that is not valid UTF-8), or call `get_file_path_bytes` for the raw bytes. |
| `CopyFunctionBuilder::register` refuses a format name that already exists (`csv`, `parquet`, …). | Give the format a name of its own. |

## Values

| Change | What to change |
|--------|----------------|
| Every scalar `Value` getter (`as_i8` … `as_u128`, `as_f32`, `as_f64`, `as_bool`, the date/time/timestamp family, `as_interval`, `as_uuid`, `as_decimal`) returns `Option<T>`. | Handle `None` (null handle, SQL `NULL`, non-scalar value or failed cast), or use the `as_*_or(default)` forms. |
| `Value::as_blob` accepts only a `BLOB` and errors on SQL `NULL`; `Value::as_str` errors on SQL `NULL`. | Check the value's type and nullness first, or handle the `Err`. |
| `Value::time_ns` and `Value::timestamp` return `Result<Value, ExtensionError>` and refuse out-of-range payloads. | Add `?` or handle the error. |

## Types

| Change | What to change |
|--------|----------------|
| The `STRUCT` and `UNION` constructors (`struct_type`, `struct_type_from_logical`, `union_type`, `union_type_from_logical`) refuse duplicate names (ignoring ASCII case) and more than `MAX_UNION_MEMBERS` members; the plain forms panic. | Make field and member names unique, or use the `try_` forms and handle the error. |
| `try_array(_, 0)`, an empty `union_type` and an empty `Value::array_value` are refused. | Do not build zero-size arrays or empty unions; DuckDB's SQL refuses them too. |
| `TypeId::Varint.sql_name()` returns `"BIGNUM"` instead of `"VARINT"`. | Update code that compares the returned string. |

## Vectors and dates

| Change | What to change |
|--------|----------------|
| `VectorWriter::set_null` / `set_null_range` (and `DataChunk::propagate_nulls`) on a `STRUCT` or `ARRAY` vector also null the row's fields or elements, recursively. | To reuse such a row, call `set_valid` on it first, then write the fields. |
| `VectorWriter::write_varchar` / `write_blob` and `StructWriter::write_varchar` / `write_blob` panic for a value longer than `MAX_STRING_LEN`. | Use `try_write_varchar` / `try_write_blob` to get an error instead, or keep values under 4 GiB. |
| `SelectionVector::new` returns `Result<Self, ExtensionError>` and zeroes the buffer. | Add `?`. |
| `datetime::date_to_days`, `time_from_micros`, `time_tz_from_bits`, `timestamp_from_micros`, `timestamp_to_micros`, `time_tz_bits` and `decimal_to_f64` return `Option`. | Handle `None` for out-of-range input. |

## Queries and the appender

| Change | What to change |
|--------|----------------|
| `QueryResult::next_chunk` returns `Result<Option<OwnedDataChunk>, ExtensionError>`. | Write `while let Some(chunk) = result.next_chunk()? { … }`. |
| `Appender` refuses to lose rows: a failed `row()` poisons it; `close()` with an unfinished row, any mutation after `close()`, and `append_chunk` in the middle of a row are errors. | Handle the errors these now return. With `duckdb-1-5`, `clear()` resets the appender and clears the poison. |
| `PreparedStatement::bind_time` / `bind_timestamp` / `bind_timestamp_tz` and `Appender::append_time` / `append_timestamp` refuse out-of-range payloads. | Handle the error. |
| `Appender` and `Connection` are no longer `RefUnwindSafe`. | Wrap a `catch_unwind` closure that captures one in `std::panic::AssertUnwindSafe`. |

## Catalog, client context and file system

| Change | What to change |
|--------|----------------|
| `CatalogEntry::lookup` and `Catalog::get_entry` return `Result<Option<CatalogEntry>, ExtensionError>`. | Treat `Ok(None)` as "not found" and `Err` as "refused". |
| Lookups are refused for `Schema`, `Database`, `PreparedStatement` and `Invalid`; for a `Type` or `Collation` name DuckDB would autoload an extension for, while `autoload_known_extensions` is on; and in any catalog whose type is not `"duckdb"`. | Query those with SQL, or `SET autoload_known_extensions = false` before an autoloading name. |
| `CatalogEntry` copies the name and type at lookup and holds no handle. | No change for `name()` / `entry_type()`; stop relying on the entry's handle. |
| `FileSystem<'ctx>` borrows its `ClientContext`; `FileSystem::open` returns `FileHandle<'fs>`, which borrows the `FileSystem`. | Add lifetime parameters where you store them, and keep the context and file system alive longer than the handles. |

See [Catalog & Client Context](../duckdb-1-5/catalog.md).

## Config options

| Change | What to change |
|--------|----------------|
| `ConfigOptionBuilder::register` refuses an option with no default, a pseudo-type (`ANY`, `SQLNULL`, `INTEGER_LITERAL`, `STRING_LITERAL`) or a type it cannot build a default for (`BIGNUM`, `GEOMETRY`, `VARIANT`, …), and a name `duckdb_settings()` already lists (case-insensitive, aliases included). | Call `default_value(...)`, use a concrete type (`VARCHAR` if in doubt), and prefix the name with your extension's name. |

See [Config Options](../duckdb-1-5/config-options.md).

## Errors

| Change | What to change |
|--------|----------------|
| `DuckDbErrorType` gains `Autoload`, `Sequence` and `InvalidConfiguration`, which used to map to `Invalid`. | Match the new variants where you used `Invalid` to catch those errors. |

## SQL macros

| Change | What to change |
|--------|----------------|
| `SqlMacro::to_sql` emits double-quoted identifiers. | Update code or tests that compare the generated SQL text. Calling the macro is unchanged. |
| `SqlMacro::register` refuses a body with more than one statement. | Keep each macro body to one expression or query. |

## Arrow (`duckdb-1-5-4`)

| Change | What to change |
|--------|----------------|
| `ArrowConvertedSchema::from_raw` takes the `&ArrowSchema` the handle was built from instead of a column count, and is no longer `const`. | Pass the schema. |

## Testing

| Change | What to change |
|--------|----------------|
| `MockVectorWriter` behaves like a real output vector: `set_null` then a write leaves the row NULL, an unwritten row is valid (check `is_written`), a write past capacity panics, a string over `MAX_STRING_LEN` panics. | Fix the callback the failing test exposes, or update the test's expectations. |
| `MockRegistrar` runs the same checks as real registration (missing callback or return type, empty set, incomplete copy function, config option without type or default) and records nothing on failure. | Complete the builder the test registers. |

## Scaffold and validation

| Change | What to change |
|--------|----------------|
| `generate_scaffold` validates `description`, `maintainer`, `github_repo` and `git_ref`, and writes YAML-quoted values. | Pass a non-empty, trimmed description and a one-line maintainer, `owner/repo`, and a commit hash or tag. |
| `validate_spdx_license` rejects nesting deeper than 64 parentheses. | No change for real license expressions. |
| `validate::description_yml::DescriptionYml` is `#[non_exhaustive]`. | Read its fields by name; build one with `parse_description_yml` instead of a struct literal. |

## Counting the entries

The [0.18.0] section of the changelog marks 54 entries **Breaking:**. Nine of
them repeat a change listed under Changed (in Fixed or Security, with "see
Changed"), so this page lists 45 distinct changes.
