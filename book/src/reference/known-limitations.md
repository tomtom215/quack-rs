# Known Limitations

## Window functions are not available

DuckDB **window functions** (`OVER (...)` clauses) are implemented entirely in
DuckDB's C++ layer and have **no counterpart in the public C extension API**.

This is not a gap in `quack-rs` or in `libduckdb-sys` — the relevant symbol
(`duckdb_create_window_function`) simply does not exist in the C API:

| Symbol | C API (1.4.x)? | C API (1.5.0+)? | C++ API? |
|--------|----------------|-----------------|----------|
| `duckdb_create_window_function` | **No** | **No** | Yes |
| `duckdb_create_copy_function`   | **No** | **Yes** | Yes |
| `duckdb_create_scalar_function` | Yes    | Yes     | Yes |
| `duckdb_create_aggregate_function` | Yes | Yes     | Yes |
| `duckdb_create_table_function`  | Yes    | Yes     | Yes |
| `duckdb_create_cast_function`   | Yes    | Yes     | Yes |

**What this means for your extension:**

A custom window operator requires a C++ extension. An ordinary aggregate can be
used *as* a window (`agg(x) OVER (...)`), but see the next section before
recommending that to your users: two of those shapes crash every aggregate
registered through the C API.

If DuckDB exposes window registration in a future C API version, `quack-rs`
will add wrappers in the corresponding release.

## Aggregates crash under `OVER ()` and `ORDER BY` (DuckDB defect, Pitfall L11)

Every aggregate registered through the C API — quack-rs's or anyone else's —
reads out of bounds when DuckDB runs it as a whole-partition window
(`agg(x) OVER ()`, `agg(x) OVER (PARTITION BY p)`) or as an ordered aggregate
(`agg(x ORDER BY y)`). The usual result is a segmentation fault that takes the
host process down. The book's own example aggregate does it:

```sql
-- with hello-ext loaded: segfaults on DuckDB 1.4.4 and 1.5.5
SELECT max(w) FROM (SELECT word_count(s) OVER () AS w
                    FROM (SELECT 'a b c' AS s FROM range(5000)));
```

The cause is in DuckDB (`CAPIAggregateUpdate` hands the callback a constant
state vector), there is no way for an extension to detect or prevent it, and it
is reported upstream as
[duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109). Until it
is fixed, tell your users not to use your aggregates in those two shapes.
Frames that are not whole-partition (`ROWS BETWEEN 5 PRECEDING AND CURRENT ROW`)
and `DISTINCT` windows work. See
[Pitfall L11](pitfalls.md#l11-c-api-aggregates-crash-under-aggx-over--and-aggx-order-by-y).

## Aggregate states leak when `finalize` reports an error (DuckDB behaviour)

When an aggregate's `finalize` callback reports an error
(`AggregateFunctionInfo::set_error`), DuckDB 1.5.5 does not call the destructor
for every state the query created: an ungrouped query initialised 2 states and
destroyed 1, a grouped one 4 and 2. When `finalize` succeeds, every state is
destroyed. The extension cannot tell which states were abandoned, so whatever
they own is leaked: with `FfiState<T>`, whatever `T` owns on the heap, and the
box of a `T` too large to store inline (see the next section). The query still
fails with your message. If that leak matters (a long-lived process whose
queries often fail this way), keep what a state owns small. Only `finalize` was measured; errors reported from other callbacks were
not. The behaviour is pinned by
`aggregate_states_are_not_all_destroyed_when_finalize_fails` in
`tests/ffi_roundtrip/lifecycle.rs`.

## Grouped-aggregate states the scan never reaches are never destroyed (DuckDB defect)

DuckDB 1.4.4 to 1.5.5 destroys a grouped aggregate's states as its result
scan passes them. When the scan stops early, the states it has not reached are
never destroyed — not after the query, not when the connection or the database
closes. That happens under a `LIMIT` above the aggregate, an error raised
above it, or an interrupt (`InterruptHandle::cancel`). Measured on one thread
with 300,000 groups: under `LIMIT 10`, 2,048 of 300,000 states were destroyed
(4,096 on 1.4.x); with an error raised half-way through the result, 151,552.
DuckDB's own aggregates are affected the same way: `mode()` under `LIMIT 10`
leaked about 100 MB per query. See `docs/upstream-duckdb-reports.md`, item 20.

The query's answer is right; the cost is memory. `FfiState<T>` stores a `T`
of at most 256 bytes, aligned no more strictly than `usize`, inside DuckDB's
own state bytes, which DuckDB frees with the hash table, so such a state leaks
nothing unless `T` itself owns heap memory (a `Vec`, `String` or `HashMap`).
A larger `T` is boxed, and the box leaks with it. Until DuckDB fixes this,
keep aggregate states small and free of heap allocations where you can.
Pinned by `states_a_grouped_scan_never_reaches_leak_no_rust_heap` in
`tests/aggregate_leaks.rs`.

## An abandoned stream keeps its table-function state (DuckDB behaviour)

Dropping a streaming `QueryResult` part-way through, and then its
`PreparedStatement`, does not free the query's operator states: DuckDB keeps
the active query on the connection until the next statement runs there (or
the connection closes). A table function's state — for a typed table
function, the `with_state` value and its per-scan clone — therefore lives
until then. Nothing leaks, but a state that holds a file, a lock or a large
buffer holds it for that long; run any statement (`SELECT 1`) on the
connection to release it. Pinned by
`an_abandoned_stream_keeps_its_table_state_until_the_next_statement` in
`tests/ffi_roundtrip/query_stream.rs`.

## Running out of memory inside a callback aborts the process

An allocation failure is not an error quack-rs can report. On the Rust side,
the default allocation-error handler aborts. On the DuckDB side,
`duckdb_list_vector_reserve`, `duckdb_vector_copy_sel` and
`duckdb_vector_assign_string_element_len` allocate without catching, so their
`std::bad_alloc` crosses the Rust callback frame and aborts the process
("Rust cannot catch foreign exceptions"). quack-rs allocates inside
callbacks only on error paths and in `data_chunk_to_arrow`'s pre-export
check, which copies each column it checks. Bound what your callbacks
allocate, and set DuckDB's `memory_limit` so its own operators fail cleanly
before the process runs out.

## COPY functions (resolved in DuckDB 1.5.0; both directions since)

DuckDB 1.5.0 added `duckdb_create_copy_function` and related symbols to the public
C extension API. quack-rs wraps these in the `copy_function` module behind the
`duckdb-1-5` feature flag. See `CopyFunctionBuilder` for usage.

This was previously listed as a known limitation (no C API counterpart prior to 1.5.0).

`COPY … FROM` was a second, narrower gap: quack-rs wrapped the writing half only.
`CopyFunctionBuilder::copy_from` now attaches a quack-rs table function as a
format's reader, and a copy function may implement either direction or both — a
read-only format leaves the writing callbacks unset entirely. See the
[Copy Functions](../functions/copy-functions.md) chapter.

## Arrow interop (resolved behind `duckdb-1-5-4`)

DuckDB 1.5.0 added a conversion family that moves data straight between a
`duckdb_data_chunk` and the Arrow C Data Interface. quack-rs wraps all eight
non-deprecated entries in the `arrow` module, with **no `arrow` crate
dependency** — see the [Arrow Interop](../duckdb-1-5/arrow.md) chapter.

The remaining fourteen Arrow entries in the C API struct are the older
`duckdb_query_arrow` result API, which lives inside
`#ifndef DUCKDB_API_NO_DEPRECATED`; they are deliberately not wrapped.

The feature is `duckdb-1-5-4` rather than `duckdb-1-5` because `libduckdb-sys`
declared the two Arrow ABI records as opaque zero-sized placeholders until
1.10504.0. The DuckDB functions themselves are present from 1.5.0.

## Callback accessor wrappers (resolved)

quack-rs now wraps all major **callback accessor** functions — the C API
functions used *inside* your callbacks to retrieve arguments, set errors,
access bind data, etc.

| Category | Wrapper type | Available |
|----------|-------------|-----------|
| **Scalar function execution** | `ScalarFunctionInfo` | Always |
| **Scalar function bind** | `ScalarBindInfo` | `duckdb-1-5` |
| **Scalar function init** | `ScalarInitInfo` | `duckdb-1-5` |
| **Aggregate function callbacks** | `AggregateFunctionInfo` | Always |
| **Table function bind** | `BindInfo` | Always |
| **Table function init** | `InitInfo` | Always |
| **Table function scan** | `FunctionInfo` | Always |
| **Cast function callbacks** | `CastFunctionInfo` | Always |
| **Copy function bind** | `CopyBindInfo` | `duckdb-1-5` |
| **Copy function global init** | `CopyGlobalInitInfo` | `duckdb-1-5` |
| **Copy function sink** | `CopySinkInfo` | `duckdb-1-5` |
| **Copy function finalize** | `CopyFinalizeInfo` | `duckdb-1-5` |

All callback accessor functions are now wrapped, including `get_client_context`
on all callback types (returns a `ClientContext`; see the `client_context` module).

## Complex type creation (resolved)

`LogicalType` now provides constructors for all complex parameterized types:

| Method | Type created |
|--------|-------------|
| `LogicalType::decimal(width, scale)` | `DECIMAL(p, s)` |
| `LogicalType::enum_type(members)` | `ENUM('a', 'b', ...)` |
| `LogicalType::array(child, size)` | `type[N]` |
| `LogicalType::union_type(members)` | `UNION(a INT, b VARCHAR)` |
| `LogicalType::list(child)` | `LIST(type)` |
| `LogicalType::struct_type(fields)` | `STRUCT(...)` |
| `LogicalType::map(key, value)` | `MAP(K, V)` |

All constructors have `_from_logical` variants for nested complex types.
Introspection methods (`get_type_id`, `list_child_type`, `struct_child_count`,
`decimal_width`, etc.) are also available.

## VARIANT and GEOMETRY types (resolved — exposed behind `duckdb-1-5-3`)

DuckDB v1.5.1 introduced the `VARIANT` type for Iceberg v3 support. As of
**DuckDB 1.5.3** it is present in the C type enum as `DUCKDB_TYPE_VARIANT` (41),
and the `GEOMETRY` type (`DUCKDB_TYPE_GEOMETRY`, 40) is present as well.

quack-rs exposes these as `TypeId::Variant` and `TypeId::Geometry`, gated behind
the **`duckdb-1-5-3`** feature. That feature layers on top of `duckdb-1-5` and
requires `libduckdb-sys >= 1.10503.1` (DuckDB 1.5.3). The separate gate exists
because these type-enum values postdate the `duckdb-1-5` feature's 1.5.0 floor
(`VARIANT` only landed in 1.5.3); keeping them out of `duckdb-1-5` preserves
compatibility for consumers pinned to libduckdb-sys 1.5.0–1.5.2.

```toml
[dependencies]
quack-rs = { version = "0.18", features = ["duckdb-1-5-3"] }
```

Neither type yet has dedicated `VectorReader`/`VectorWriter` helpers; access
their data via the raw pointer from `duckdb_vector_get_data` when needed.
