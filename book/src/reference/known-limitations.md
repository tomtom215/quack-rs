# Known Limitations

What quack-rs cannot do because the DuckDB C extension API does not allow it,
DuckDB defects and behaviour an extension should plan around, things your own code must get
right because nothing can check them for you, and, at the end, former limitations that
have been resolved. Each entry says which DuckDB releases it applies to and whether it was
measured or only read.

## What your extension must get right

### An extension that reads files with `std::fs` bypasses DuckDB's permission settings


**This is a security-relevant trap.** `SET enable_external_access = false` and
`SET allowed_directories = [...]` restrict what SQL can read and write, and the
restriction applies to everything DuckDB opens itself. It does not apply to code in your
extension that opens a file with `std::fs`: measured on DuckDB 1.5.6 and 2.0.0, with
external access off, `read_csv('data.csv')` fails with "Permission Error: Cannot access
file … file system operations are disabled by configuration", a table function that opens
the same path with quack-rs's `FileSystem` fails the same way, and a table function that
calls `std::fs::read` returns the file's contents. A function that takes a path from SQL
and reads it with `std::fs` therefore lets any user of that DuckDB read any file the
process can, which is what the setting exists to prevent. The same goes for any
network client you add: no DuckDB setting reaches it.

Open user-supplied paths with the `FileSystem` wrapper (feature `duckdb-1-5`), which
applies the settings and also resolves `s3://` and `http://` paths when `httpfs` is
loaded. See [Virtual File
System](../duckdb-1-5/file-system.md#security-only-filesystem-honours-duckdbs-permission-settings)
and [Pitfall P17](pitfalls.md#p17).

### A `duckdb-1-5*` extension refuses a DuckDB release quack-rs has not listed

With the default features, one binary loads into every DuckDB from 1.4.4 through 2.0.x. An
extension that enables a `duckdb-1-5*` feature calls the part of the C API that DuckDB
declares unstable, and quack-rs has verified that layout for DuckDB 1.5.2 to 1.5.6 and 2.0.0
only: built against the 1.5.6 bindings and stamped `C_STRUCT` with C API `v1.5.6`, it
loads into 1.5.6 and 2.0.0. Under the default `AbiPolicy::Strict` it refuses to load into any
other release with an error that names the fixes, and DuckDB 2.0.1 will be one until a quack-rs
release lists it. The layout cannot be extrapolated: DuckDB 2.0.0's `duckdb_extension.h` opens
with "this API is not yet stable, meaning it is only guaranteed to work for this specific
DuckDB version", 2.0.0 itself put two slots (`duckdb_create_timestamp_tz_ns` and
`duckdb_get_timestamp_tz_ns`) into the range that 1.5.6 declared stable, and the 1.5.2 patch
release inserted a slot in the middle of the struct. What to do when a new release appears:
check the release's `duckdb_extension.h` against the one you built with, then rebuild with
`QUACK_RS_TARGET_DUCKDB_VERSION` set to it, or move to a quack-rs release that lists it; or stay
on the default features, which only use the stable prefix and are not affected. See [ABI
Compatibility](../concepts/abi.md#unknown-duckdb-versions).

### A function that reads a setting must be marked volatile

DuckDB folds a non-volatile function call whose arguments are constants into the
plan, once. A function whose `init` reads a setting
(`ClientContext::config_option`) and is not `.volatile()` then keeps answering
with the value it read first: after `SET v_setting = 9; PREPARE p AS SELECT f();
SET v_setting = 10; EXECUTE p;` the plain function returned 9 and the same function
marked `.volatile()` returned 10. The C API v1 has only `volatile`, not "stable
within a query".

### NULL rows of `LIST` and `MAP` vectors hold stale entries

DuckDB writes no list entry for a NULL row, so the `offset` and `length` that
`ListVector::get_entry` and `MapVector::get_entry` return for one are whatever the
reused buffer held. On 2.0.0 a `nullif(l, [])` column had NULL rows with lengths
around 7·10¹³, and a loop that walked them read far outside the child vector: a
segmentation fault in a minority of runs, and no failure at all in most, which is
how the pattern went unnoticed. `get_entry_checked` returns `None` for a NULL row
and for an entry that does not lie inside the child; use it, or test the row's
validity before `get_entry`.

### A `VARIANT` cannot be declared in a function signature


The type `duckdb_create_logical_type(VARIANT)` returns has no `STRUCT` layout, because
DuckDB stores a VARIANT as a struct. A scalar function declared with it
registers, and the first query that reaches it takes DuckDB down: on 2.0.0
`SELECT v_out(1)` is a segmentation fault (`VectorStructBuffer` while
initialising the result vector; reproduced with the guard bypassed), and on 1.5.x
`SELECT v_in({'a':1}::VARIANT)` is an internal error that invalidates the
database. quack-rs therefore refuses `TypeId::Variant` wherever it builds a
logical type (`LogicalType::try_new`, and so every builder's parameter,
return-type and result-column slots), with an error that names `ANY`. A function
that wants to accept a VARIANT declares the parameter as `TypeId::Any`.
`GEOMETRY` is unaffected. See [Pitfall P13](pitfalls.md#p13).

### A caught panic is still printed to stderr

The macros and typed builders catch a panic at the FFI boundary and report it as a
SQL error, but Rust's default panic hook has already run by then: it writes the
panic message to the host process's stderr (and a backtrace when `RUST_BACKTRACE`
is set), and does so even though the query only fails with an error. quack-rs
installs no hook, because the hook is process-wide and the host (Python, a CLI, a
server) owns it. An extension that wants these lines silent or routed to its own
log can call `std::panic::set_hook` in its entry point, knowing that it replaces
the host's hook for every thread in the process.

**Some panics cannot be caught at all.** A destructor that panics while another panic is
unwinding, a panic hook that itself panics, and an allocation failure while the panic
message is formatted (Rust's default handler aborts) all end the host process, whatever
the guards do. The first was reproduced: `panic in a destructor during cleanup … aborting`,
exit code 134. Keep `Drop` implementations of your own types from panicking. And under
`panic = "abort"` no guard works at all, which is why `entry_point!` and `entry_point_v2!`
stop the build there.

### Loading an extension scans the function catalog once

Every scalar and table registration through quack-rs first lists `duckdb_functions()`
to refuse a signature that would silently replace an existing function (a built-in
included) or make calls ambiguous. Registering through the entry point's
`Connection` does that scan once per `LOAD`, however many functions follow. The cost
is fixed, not per function: for an extension with one `map1` function, `LOAD` took
18 ms longer than the same entry point registering the function through the raw C
API on DuckDB 1.5.6, and 30 ms longer on 2.0.0 (medians of 30 interleaved CLI
runs each, one 4-vCPU virtual machine, release build; the minima differ by 17 and
27 ms). The C API has no catalog lookup for functions, so the scan cannot be
narrowed to a name, and filtering the query to the names being registered does not make
it cheaper (`duckdb_functions()` has no pushdown). The scan grows with the catalog: one
scan took about 20 ms on 1.5.6 and 13 ms on 2.0.0 in an empty database and about 110 ms
on both with 20,000 user macros (CLI measurements on a shared 4-vCPU machine), and a
`LOAD` that registers scalar and table functions scans twice. Registering through a
builder's `register(con)` on the raw connection scans once per call instead of once per
`LOAD`.

### SQL macros: no defaults or overloads, and they persist

`SqlMacro` takes plain parameter names; `y := 5` is rejected, and registering the
same name again replaces the first (`CREATE OR REPLACE`). For a default
parameter, run `CREATE OR REPLACE MACRO m(x, y := 5) AS x + y` through
`Connection::execute`. A macro is stored in the user's database, so one whose
body calls an extension function breaks when the database is opened without the
extension loaded ("Table Function with name … does not exist"); a custom type
registered through the C API is not stored.

## DuckDB defects an extension cannot prevent

### Aggregates crash under `OVER ()` and `ORDER BY` (DuckDB defect, Pitfall L11)

On DuckDB 1.4.x and 1.5.x, every aggregate registered through the C API —
quack-rs's or anyone else's — reads out of bounds when DuckDB runs it as a
whole-partition window or as an ordered aggregate, and the usual result is a
segmentation fault that takes the host process down. The book's own example
aggregate does it. Measured with `word_count` from [Your First
Extension](../getting-started/first-extension.md), one query shape per process, over 5,000
and over 300,000 rows:

| Query shape | 1.4.4 | 1.5.6 | 2.0.0 |
|-------------|-------|-------|-------|
| `agg(x) OVER ()` | crash | crash | works |
| `agg(x) OVER (ORDER BY i ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)` | crash | crash | works |
| the same with `RANGE`, or with a `PARTITION BY` | crash | crash | works |
| `agg(x ORDER BY y)` | crash | crash | works |
| `agg(x) OVER (PARTITION BY p)`, no `ORDER BY` | **crash** | works | works |
| `OVER (ORDER BY i)` (the default frame), `ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`, `CURRENT ROW AND UNBOUNDED FOLLOWING`, sliding frames, `EXCLUDE`, `DISTINCT`, `FILTER` | works | works | works |

```sql
-- with the book's word_count loaded: segfaults on DuckDB 1.4.4 and 1.5.6
SELECT max(w) FROM (SELECT word_count(s) OVER () AS w
                    FROM (SELECT 'a b c' AS s FROM range(5000)));
```

An explicit `UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING` frame is a whole-partition
frame, and crashes like `OVER ()`. `OVER (PARTITION BY p)` without an `ORDER BY` did not
crash on 1.5.6 in these runs; the code path that reads the state array is DuckDB's, and
whether it reads out of bounds there without a visible effect was not established, so do
not rely on it. The cause is in DuckDB (`CAPIAggregateUpdate` hands the callback a constant
state vector), there is no way for an extension to detect or prevent it, and it is
reported upstream as
[duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109). DuckDB
2.0.0 fixes it (`CAPIAggregateUpdate` flattens the state vector first); on
older releases, tell your users not to use your aggregates in those shapes. See
[Pitfall L11](pitfalls.md#l11-c-api-aggregates-crash-under-aggx-over--and-aggx-order-by-y).

### Aggregate states leak when `finalize` reports an error (DuckDB behaviour)

When an aggregate's `finalize` callback reports an error
(`AggregateFunctionInfo::set_error`), DuckDB does not call the destructor
for every state a grouped query created (4 initialised and 2 destroyed on
1.5.5; still fewer destroyed than initialised on 2.0.0). Before 2.0.0 an
ungrouped query is affected too (2 initialised, 1 destroyed), because a C API
aggregate is planned as a hash aggregate there; 2.0.0 runs it as an ungrouped
aggregate, which destroys every state after the failure. When `finalize`
succeeds, every state is destroyed. The extension cannot tell which states were abandoned, so whatever
they own is leaked: with `FfiState<T>`, whatever `T` owns on the heap, and the
box of a `T` too large to store inline (see the next section). The query still
fails with your message. If that leak matters (a long-lived process whose
queries often fail this way), keep what a state owns small.

Errors from the other callbacks were measured as well (a million rows, ungrouped
and grouped, one and four threads). An error from `update` destroys every state on
DuckDB 1.5.6 and 2.0.0. An error from `combine` leaks on DuckDB 1.x when several
threads built states for it to merge (1.5.6, four threads: 2 initialised and 1
destroyed ungrouped; 10 or 11 initialised and 3 or 4 destroyed grouped), not with
one thread, and not on 2.0.0. The behaviour is pinned by
`aggregate_states_are_not_all_destroyed_when_finalize_fails` and
`aggregate_states_after_update_and_combine_errors` in
`tests/ffi_roundtrip/lifecycle.rs`.

### Grouped-aggregate states the scan never reaches are never destroyed (DuckDB defect)

DuckDB 1.4.4 to 2.0.0 destroys a grouped aggregate's states as its result
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

### Window frames with `EXCLUDE` never destroy one state per row (DuckDB defect)

A C API aggregate in a window whose frame has `EXCLUDE CURRENT ROW`, `GROUP`
or `TIES` is evaluated by DuckDB's segment tree in two parts, and before
2.0.0 the second part initialises one state per row that is never destroyed:
over a 5000-row window, 5000 states on every release from 1.4.4 to 1.5.6
(none without `EXCLUDE`). DuckDB 2.0.0 destroys them after combining
(`right_part->Destroy` in `WindowSegmentTreeLocalState::Evaluate`). The answer is right; as above, a small `FfiState<T>` leaks only
what `T` owns on the heap. See `docs/upstream-duckdb-reports.md`, item 35;
pinned by `a_window_frame_with_exclude_leaves_states_undestroyed` in
`tests/ffi_roundtrip/agg_states.rs`.

### A failing state initialiser in a window frame crashes the host (DuckDB defect)

When an aggregate's `state_init` reports an error while the aggregate runs as a
window function with a `ROWS` or `RANGE` frame, DuckDB's
`WindowAggregateStates` destroys its whole table of state pointers, including
entries it never wrote. The destructor is handed garbage pointers. A plain C
aggregate shows it, with no Rust involved: on DuckDB 1.5.6 55 of 68 pointers
passed to the destructor were garbage, and on 2.0.0 65 of 68. quack-rs's
`FfiState` reports a panic in `T::default()` as a SQL error, which is exactly this
trigger; its tag check cannot help, since there is no slot to read. Ungrouped,
grouped, `DISTINCT` and `FILTER` aggregates report a clean error; a window shape
kills the process (`SIGSEGV` or `SIGBUS`). **`T::default()` must not panic** for an
aggregate that can run as a window function.

### An abandoned stream keeps its table-function state (DuckDB 1.x behaviour)

On DuckDB 1.x, dropping a streaming `QueryResult` part-way through, and then
its `PreparedStatement`, does not free the query's operator states: DuckDB
keeps the active query on the connection until the next statement runs there
(or the connection closes). A table function's state — for a typed table
function, the `with_state` value and its per-scan clone — therefore lives
until then. Nothing leaks, but a state that holds a file, a lock or a large
buffer holds it for that long; run any statement (`SELECT 1`) on the
connection to release it.

DuckDB 2.0.0 aborts the query as soon as the unfinished result is destroyed
(`QueryResult::Close`), so the states go with it. The flip side is that 2.0
refuses any other statement on the connection — a query, a `prepare`, an
appender's flush — with a "Resource In Use" error while a stream is still
open there, where 1.x ran it and ended the stream. Finish or drop a stream
before using its connection for anything else. Both engines' behaviour is
pinned by `an_abandoned_stream_keeps_its_table_state_until_the_next_statement`
and `another_statement_on_the_connection_ends_a_stream_with_an_error` in
`tests/ffi_roundtrip/query_stream.rs`.

### Running out of memory inside a callback aborts the process

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

One allocation failure is worse than an abort. When `duckdb_prepare` fails to
allocate while it records a statement's parameter names, it frees the
statement it has already handed back and reports an error; `prepare` (and
everything built on it) then reads and frees that statement again. This is
undefined behaviour inside DuckDB's C API that no caller can detect; see
`docs/upstream-duckdb-reports.md`, item 36.

### `duckdb_register_log_storage` frees its extra data twice when the name is taken (DuckDB defect, Pitfall L20)


When registering a log storage fails because the name is already registered, DuckDB runs
the extra-data destructor inside the failed call and leaves the pointer on the wrapper, so
the `duckdb_destroy_log_storage` the caller must run frees it again. Observed on DuckDB
1.5.6 and 2.0.0: the extra-data destructor ran twice on the same pointer. With a destructor
that frees its box that is a heap double free, which the eighth review saw abort with
`free(): double free detected in tcache 2`; the code is the same since 1.5.0 and on `main`
(read). It is not a quack-rs defect: quack-rs does not wrap the log-storage functions, for this reason.
An extension that calls them through the raw bindings must never register a name that can
already exist, and must treat a failed registration as having consumed the extra data. See
[Pitfall L20](pitfalls.md#l20-duckdb_register_log_storage-frees-the-extra-data-twice-when-the-name-is-taken).

### `duckdb_get_bit` and `duckdb_get_bignum` abort on NULL and on values that do not cast (DuckDB defect, Pitfall P16)


Called on a SQL `NULL` value, or on a value that cannot be cast to `BIT`, `duckdb_get_bit`
throws a C++ exception through the C boundary, and Rust cannot catch it:
`fatal runtime error: Rust cannot catch foreign exceptions`, `SIGABRT` (reproduced on
DuckDB 1.5.6 and 2.0.0 with a `NULL` and with `'abc'`). `duckdb_get_bignum` runs the same
code (read in the 2.0.0 source, not run). quack-rs's `Value::as_bit()` and
`Value::as_bignum()` check the value's type and its NULL-ness first and return `None`
instead. Code that calls the raw getters through the bindings must do the same. See
[Pitfall P16](pitfalls.md#p16).

## What the C API does not offer

### Window functions are not available

DuckDB **window functions** (`OVER (...)` clauses) are implemented entirely in
DuckDB's C++ layer and have **no counterpart in the public C extension API**.

This is not a gap in `quack-rs` or in `libduckdb-sys`: the C API has no function for
registering a window function, and no symbol such as `duckdb_create_window_function`
exists in `duckdb.h` or `duckdb_extension.h`. DuckDB's C++ API does support window
functions, through its own C++ classes rather than a function of that name.

| Kind of function | C API symbol | C API (1.4.4, 1.4.5) | C API (1.5.0+) | C++ API |
|------------------|--------------|----------------------|----------------|---------|
| Window function | *(none)* | **No** | **No** | Yes |
| `COPY` function | `duckdb_create_copy_function` | **No** | **Yes** | Yes |
| Scalar function | `duckdb_create_scalar_function` | Yes | Yes | Yes |
| Aggregate function | `duckdb_create_aggregate_function` | Yes | Yes | Yes |
| Table function | `duckdb_create_table_function` | Yes | Yes | Yes |
| Cast function | `duckdb_create_cast_function` | Yes | Yes | Yes |

**What this means for your extension:**

A custom window operator requires a C++ extension. An ordinary aggregate can be
used *as* a window (`agg(x) OVER (...)`), but see the next section before
recommending that to your users: two of those shapes crash every aggregate
registered through the C API.

If DuckDB exposes window registration in a future C API version, `quack-rs`
will add wrappers in the corresponding release.

### Table function arguments must be constants, not columns or subqueries

A table function registered through the C API cannot take a column or a
subquery as an argument. `LATERAL (SELECT * FROM f(t.n))`, a correlated call, a
recursive CTE that references the function and `f((SELECT 4))` all fail in the
binder, on 1.5.6 and 2.0.0, with "does not support lateral join column
parameters" or "cannot contain subqueries". What it accepts is a **constant
expression**: `f(1 + 2)`, `f(3::INTEGER)` and
`f(current_setting('threads')::BIGINT * 0 + 4)` were evaluated once, at bind
time, and passed in (measured on 1.5.6 and 2.0.0). A prepared-statement parameter
(`f($1)`) works. Pass a `LIST` literal, or write the function as a scalar.

### The C API v1 has no table-in-out functions, progress callbacks, filter pushdown, logging or secret access


Checked by searching the 1.5.6 and 2.0.0 `duckdb.h`, `duckdb_extension.h` and, for the
2.0.0 v2 headers, `duckdb_v2.h`:

- a table function cannot take rows from a subquery (no table-in-table-out function exists
  in either C API), has no progress callback, and receives no filters (no filter pushdown
  in v1);
- the only logging functions are the log *storage* (sink) functions, such as
  `duckdb_register_log_storage` (see the entry on its double free below); nothing writes a log
  record. There is no function that reads a secret. quack-rs offers `list_duckdb_secrets`,
  which asks SQL (`duckdb_secrets()`) and redacts the value;
- `COPY` has no column names, option schema or file extension (see below).

DuckDB 2.0's C API v2 has most of these (next entries).

### Custom COPY formats cannot declare a file extension

The C API v1 has no way to give a COPY function a file extension, so
`PER_THREAD_OUTPUT`, `PARTITION_BY` and `FILENAME_PATTERN` produce files that end
in a bare dot (`data_0.`, `part_0.`); the built-in formats give `data_0.csv`.
Append the extension to the path in your `global_init` if the output needs one.

DuckDB does choose a format from the file name, as long as the name is the format's:
`COPY t TO 'out.my_format'` uses a copy function called `my_format` (measured on 1.5.6 and
2.0.0), so name the format after the extension you want recognised. Two more
things a COPY format cannot do in the C API v1: it cannot see the **column names** of a
`COPY … TO` (the bind callback gets the count and the types), so a writer that wants a
header needs a `names` option from the user; and it cannot declare its **options**, so DuckDB
accepts any option name, value count or value type for a `COPY … TO` and the format must
reject what it does not understand (see [Copy
Functions](../functions/copy-functions.md#options)).

### DuckDB 2.0's C API v2 offers what v1 cannot

DuckDB 2.0.0 ships a second C API, `duckdb_extension_v2.h` / `duckdb_v2.h`, with
no stable surface yet: a binary built against it loads only into the exact
DuckDB release it was built for, and DuckDB 2.0 sends every
`C_STRUCT_UNSTABLE` binary to its `<name>_init_c_api_v2` entry point. quack-rs
does not generate that entry point, so none of the following is available
through it. Every name below exists in the 2.0.0 `duckdb_v2.h` /
`duckdb_extension_v2.h` and none exists in the v1 headers (checked by `grep`
against the headers of the 2.0.0 release archive); the v2 API has about 860
`duckdb_v2_*` declarations, and this list is the part an author of the extension kinds
quack-rs wraps would reach for:

- a scalar function's or aggregate's return type chosen at bind time
  (`duckdb_v2_scalar_function_resolve_types_set_return_type`,
  `duckdb_v2_aggregate_function_bind_set_return_type`) and the aggregate's row
  count (`duckdb_v2_aggregate_function_update_get_row_count`);
- function properties, among them a "consistent within query" stability;
- documentation for `duckdb_functions()`: a description, examples and categories
  (`duckdb_v2_function_docs_set_description`, `_add_example`, `_add_category`), which the
  community extensions site shows (see [Community Extensions](../publishing.md));
- an option's update callback (`duckdb_v2_custom_option_set_update_callback`), which
  can validate or replace a setting's value;
- for table functions: filter pushdown
  (`duckdb_v2_table_function_set_filter_pushdown_callback`), a progress callback
  (`duckdb_v2_table_function_progress_*`), order preservation
  (`duckdb_v2_table_function_bind_set_order_preservation`) and batch claiming
  (`duckdb_v2_table_function_claim_batch_*`); and multi-file readers
  (`duckdb_v2_multi_file_function_*`);
- for `COPY`: a file extension (`duckdb_v2_copy_function_set_file_extension`) and file-size
  statistics (`duckdb_v2_copy_to_set_statistics_callback`);
- writing log records (`duckdb_v2_context_log`) and registering log sinks;
- more file-system operations: `glob`, `exists`, `expand`, `move`, `is_remote`
  (`duckdb_v2_file_system_path_*`);
- query metrics (`duckdb_v2_query_metrics_*`), a SQL tokenizer (`duckdb_v2_token_iterator_*`),
  native macro functions, pragma functions and custom types;
- static registration, which also does not work with quack-rs's v1 entry point (next entry).

The official Rust template and `duckdb-neo` target this API. quack-rs stays on v1
for the portable binary; see the README's comparison.

### A quack-rs extension cannot be linked statically into a DuckDB 2.0 host


DuckDB 2.0.0 can register an extension compiled into the host program
(`duckdb_register_static_extension`, in `duckdb_static_extension.h`). For a C API v1 entry
point it calls the entry point with `get_api` unset, because the extension is expected to call
DuckDB's linked symbols directly instead of going through the function-pointer table
(`DuckDB::LoadStaticCAPIExtension`, `extension_load.cpp` in the v2.0.0 source). quack-rs's
entry points need the table to initialise the bindings, so they refuse with "DuckDB passed no
get_api function, so the C API cannot be initialised": no panic, no crash, and no extension.
A Rust program that embeds DuckDB 2.0 and wants a quack-rs extension in-process cannot do
that; it can `LOAD` the stamped shared library like any other client. Read in the source,
not run.

## Behaviour that changed in DuckDB 2.0

### Aggregates that take no argument count no rows on DuckDB 2.0

The C API gives an aggregate's `update` callback a chunk and the state array and
no row count: the number of rows is the chunk's size. DuckDB 2.0 no longer sets
that cardinality (`CAPIAggregateUpdate`, `aggregate_function-c.cpp`), and a chunk
with no column reports a size of 0. A zero-argument aggregate (`SELECT my_count()
FROM t`) therefore answers 100 over 100 rows on DuckDB 1.5.6 and **0** on 2.0.0
(measured on both, ungrouped and grouped), and a windowed call of such an
aggregate never reaches `update` on either. quack-rs refuses to register an
aggregate, or an overload in a set, that has no parameter. Give it one, and put a
macro in front of it for a zero-argument spelling:

```rust,ignore
// my_count_1(x) counts the rows it is called on; `x` is ignored.
AggregateFunctionBuilder::new("my_count_1").param(TypeId::BigInt) /* … */;
SqlMacro::scalar("my_count", &[], "my_count_1(1::BIGINT)")?; // CREATE MACRO my_count() AS my_count_1(1::BIGINT)
```

### A `STRUCT` with only empty field names is a `TUPLE` on DuckDB 2.0

DuckDB 2.0 has a `TUPLE` type, which the C API reports as a `STRUCT` with empty
child names, and `duckdb_create_struct_type` with all-empty names now builds one.
`LogicalType::struct_type(&[("", …), ("", …)])` is accepted and gives a `TUPLE` on 2.0
and a different type on 1.x. Name the fields.

### Other DuckDB 2.0 behaviour changes that reach C API extensions


Beyond the entries above, DuckDB 2.0 changes these things an extension can notice. The
first four were measured against the 1.4.4, 1.5.6 and 2.0.0 engines; the rest were read
by comparing `src/main/capi` at v1.5.6 with `src/main/capi/v1` at v2.0.0 and have not been
run.

- `VerifyNullHandling` is compiled into release builds and runs after
  `SET debug_verification_mode = 'VERIFY_FUNCTIONS'` ([Pitfall
  L8](pitfalls.md#l8-default_null_handling-does-not-propagate-nulls-for-scalar-functions)).
- A table function's `bind` runs again on every `EXECUTE` of a prepared statement
  ([Table Functions](../functions/table-functions.md#lifecycle)).
- `INTERVAL '1 month' - INTERVAL '1 day'` compares equal to `INTERVAL '29 days'`, where
  DuckDB 1.4.4 and 1.5.6 compare it greater ([INTERVAL](../data/intervals.md)).
- A bare identifier as a table function argument (`active_users(users)`) prints a
  deprecation warning; write `'users'`.
- `duckdb_appender_close` returns an error when an earlier flush or close failed, where 1.5.6
  did not report it again (`appender-c.cpp`).
- `duckdb_data_chunk_set_size` ignores a size DuckDB rejects in a release build instead of
  reporting it (`SetChildCardinality` inside `catch (...) { D_ASSERT(false); }`).
- Every C API scalar function is registered fallible, so an error from a function
  declared non-fallible is no longer turned into an internal error.
- A scalar function whose inputs are all constants is no longer marked a constant vector
  on the way out.
- A `COPY` option given several values of different types arrives as a `TUPLE`.

## Resolved

These were limits of earlier releases. They are kept so older changelog entries and
reports still make sense.

### COPY functions (resolved in DuckDB 1.5.0; both directions since)

DuckDB 1.5.0 added `duckdb_create_copy_function` and related symbols to the public
C extension API. quack-rs wraps these in the `copy_function` module behind the
`duckdb-1-5` feature flag. See `CopyFunctionBuilder` for usage.

This was previously listed as a known limitation (no C API counterpart prior to 1.5.0).

`COPY … FROM` was a second, narrower gap: quack-rs wrapped the writing half only.
`CopyFunctionBuilder::copy_from` now attaches a quack-rs table function as a
format's reader, and a copy function may implement either direction or both — a
read-only format leaves the writing callbacks unset entirely. See the
[Copy Functions](../functions/copy-functions.md) chapter.

### Arrow interop (resolved behind `duckdb-1-5-4`)

DuckDB's C API has a family of conversion functions (present since 1.4.4) that
move data directly between a `duckdb_data_chunk` and the Arrow C Data
Interface. quack-rs wraps all eight non-deprecated entries in the `arrow` module, with **no `arrow` crate
dependency** — see the [Arrow Interop](../duckdb-1-5/arrow.md) chapter.

The remaining fourteen Arrow entries in the C API struct are the older
`duckdb_query_arrow` result API, which lives inside
`#ifndef DUCKDB_API_NO_DEPRECATED`; they are deliberately not wrapped.

The feature is `duckdb-1-5-4` rather than `duckdb-1-5` because `libduckdb-sys`
declared the two Arrow ABI records as opaque zero-sized placeholders until
1.10504.0. The DuckDB functions themselves are present in every release
quack-rs supports, but because the feature implies `duckdb-1-5`, an extension
built with it needs a DuckDB 1.5.0+ engine.

### Callback accessor wrappers (resolved)

quack-rs wraps the **callback accessor** functions — the C API functions used
*inside* your callbacks to retrieve arguments, set errors, access bind data,
and so on:

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

Where the C API provides a client context for a callback — scalar bind and
init, table function bind, and the four copy-function callbacks — the wrapper
exposes it as `get_client_context`, which returns a `ClientContext` (see the
`client_context` module).

### Complex type creation (resolved)

`LogicalType` provides constructors for all complex parameterized types:

| Method | Type created |
|--------|-------------|
| `LogicalType::decimal(width, scale)` | `DECIMAL(p, s)` |
| `LogicalType::enum_type(members)` | `ENUM('a', 'b', ...)` |
| `LogicalType::array(child, size)` | `type[N]` |
| `LogicalType::union_type(members)` | `UNION(a INT, b VARCHAR)` |
| `LogicalType::list(child)` | `LIST(type)` |
| `LogicalType::struct_type(fields)` | `STRUCT(...)` |
| `LogicalType::map(key, value)` | `MAP(K, V)` |

The constructors that take child types (`list`, `array`, `map`, `struct_type`,
`union_type`) have `_from_logical` variants for nested complex types, and each
constructor has a `try_` form that returns an error instead of panicking.
Introspection methods (`get_type_id`, `list_child_type`, `struct_child_count`,
`decimal_width`, etc.) are also available.

### VARIANT and GEOMETRY types (resolved — exposed behind `duckdb-1-5-3`)

The `VARIANT` type (a self-describing nested value, used for example by
Iceberg v3) entered the C type enum as `DUCKDB_TYPE_VARIANT` (41) in
**DuckDB 1.5.3**. `GEOMETRY` (`DUCKDB_TYPE_GEOMETRY`, 40) was already present
earlier in the 1.5.x line.

quack-rs exposes these as `TypeId::Variant` and `TypeId::Geometry`, gated behind
the **`duckdb-1-5-3`** feature. That feature layers on top of `duckdb-1-5` and
requires `libduckdb-sys >= 1.10503.0` (DuckDB 1.5.3). The separate gate exists
because `VARIANT` postdates the `duckdb-1-5` feature's 1.5.0 floor; `GEOMETRY`
is gated with it so that one feature covers both values. Keeping them out of
`duckdb-1-5` preserves compatibility for consumers pinned to libduckdb-sys
1.10500–1.10502 (DuckDB 1.5.0–1.5.2).

```toml
[dependencies]
quack-rs = { version = "0.20", features = ["duckdb-1-5-3"] }
```

Neither type yet has dedicated `VectorReader`/`VectorWriter` helpers; access
their data via the raw pointer from `duckdb_vector_get_data` when needed.

A `VARIANT` still cannot be declared in a function signature: see
[the entry above](#a-variant-cannot-be-declared-in-a-function-signature).
