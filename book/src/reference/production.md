<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215) -->

# Production Configuration

What to set, use and avoid when an extension built with quack-rs runs where a wrong
answer, a crash or a leak matters. Each item names the setting or API it is about and
the page that explains it. None of it makes an extension correct, validated or
compliant with anything: quack-rs is an FFI layer, and the extension's own logic, its
tests and the host's isolation are outside it.

The items come from the independent review of 2026-11-27 (`AUDIT.md` section 17),
checked against the code and against DuckDB 1.5.6 and 2.0.1.

## Version

- **Use a release that has the fixes.** 0.19.x and 0.20.x carry the defects listed
  under "Known defects in supported releases" in `SECURITY.md`; they are fixed on
  `main` for 0.21.0. Pin the exact quack-rs and `libduckdb-sys` versions in the
  extension's own lockfile, and qualify the exact DuckDB release you deploy (2.0.1 is
  the newest one this book's measurements cover).

## Build

- **`QUACK_RS_REQUIRE_STRICT_ABI=1`** in the build environment, so every entry point
  applies `AbiPolicy::Strict` whatever its source names
  ([ABI Compatibility](../concepts/abi.md#forbidding-the-permissive-policies-in-a-build)).
- **`QUACK_RS_TARGET_DUCKDB_VERSION`** set to the release the bindings were resolved
  against, when that release is newer than quack-rs's layout table.
- **Release profile**: `panic = "unwind"` (required: the panic guards need it) and
  `overflow-checks = true` (integer overflow panics, which the callback macros report as
  a SQL error, instead of wrapping). The scaffold generates both.
- **One DuckDB library per process.** A second library image is refused at `LOAD`
  ([Known Limitations](known-limitations.md#one-duckdb-engine-per-process)).

## Numbers

[Numbers: What Is Checked](../data/numbers.md) is the full list. In short:

- Write `DECIMAL` through `VectorWriter::decimal()`, not `write_decimal`; build values
  with `Value::try_varchar` / `try_blob`; finish lists with `ListBuilder::try_finish`,
  so a row refused for the element limit is an error rather than a silent NULL.
- Compute with `checked_*` arithmetic and return an error for values the domain
  rejects. `overflow-checks` is a backstop, not the policy.
- For decimal division, DuckDB 2.0's `decimal_division(a, b, scale)` returns a
  `DECIMAL` rounded half to even: on 2.0.1, `decimal_division(1.25, 1.00, 1)` is `1.2`
  and `(1.35, 1.00, 1)` is `1.4`, while `round(1.25, 1)` is `1.3`. Plain `/` on two
  `DECIMAL(18, 2)` returns a `DOUBLE`. 1.5.6 has no `decimal_division`.
- A floating-point aggregate's result can change with the thread count and the plan.
  Accumulate in integers where the answer must be exact or reproducible.

## Aggregates

- **`T::default()` must not panic** in an aggregate a window can run: DuckDB's window
  path can then hand the destructor pointers it never wrote
  ([Known Limitations](known-limitations.md)).
- **A state that owns heap memory leaks** when the result scan stops early (`LIMIT`
  above the aggregate, an error, a cancel), still on 2.0.1. Keep states small and free
  of `Vec`, `String` and `HashMap` where you can, or budget for the leak.
- **`combine` must carry the configuration as well as the totals** into a fresh target
  state (pitfall L1 in the [Pitfall Catalog](pitfalls.md)); forgetting it gives wrong
  answers with every memory access valid.

## Table functions and chunks

- Claim output rows with `ChunkWriter::begin_row` and `commit` each row once every
  column is written; an abandoned row is given back instead of being published with
  stale contents.
- A chunk imported from Arrow may share the producer's buffers
  (`OwnedDataChunk::is_externally_backed`). The safe
  `Appender::append_default_to_chunk` refuses it; the `unsafe` writers leave it to you.
  Read the `# Safety` section of `arrow::data_chunk_from_arrow` before importing
  arrays you did not produce: some of its requirements (padding past a validity
  bitmap or a dictionary) go beyond the Arrow format's.

## SQL, transactions and appenders

- Run SQL built from values with `prepare` and bound parameters. `prepare` accepts
  exactly one statement; `query` runs every statement in the string, and outside an
  explicit transaction the ones before a failure stay committed
  ([Running SQL](../data/running-sql.md)).
- Group the operations of one business transaction in an explicit `BEGIN` / `COMMIT`,
  and check the result of `COMMIT`.
- With the appender, check the results of `flush` and `close` rather than relying on
  the drop, which cannot return an error ([Bulk Appender](../data/appender.md)).
- A DuckDB rollback does not undo what the extension did outside DuckDB (files written,
  requests sent).

## The host

- **A loaded extension runs with the host process's privileges.** DuckDB's
  `enable_external_access`, `allowed_directories` and `allowed_paths` bind only I/O that
  goes through DuckDB; route SQL-supplied paths through `file_system::FileSystem` and
  contain the rest at the operating-system level (`SECURITY.md`, "Trust model").
- **Lock the configuration** once the extension repositories you trust are set up. On
  1.5.6 and 2.0.1 the defaults are `allow_community_extensions`,
  `autoinstall_known_extensions` and `autoload_known_extensions` true,
  `allow_unsigned_extensions` false and `lock_configuration` false:

  ```sql
  SET allow_community_extensions = false;
  SET autoinstall_known_extensions = false;
  SET autoload_known_extensions = false;
  SET lock_configuration = true;  -- every later SET fails
  ```

- **`audit_tls_provider` reads what a provider declares**; it does not perform or
  observe a handshake. Test certificate and hostname validation against the real
  endpoints ([TLS Configuration](../security/tls.md)).
- **`SecretEntry` zeroizes its own buffers**, not copies made by `clone`, by
  conversions or by the operating system ([Secrets Management](../security/secrets.md)).
- Set a process memory limit as well as DuckDB's `memory_limit`, which does not count
  the extension's own heap.
