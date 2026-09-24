<!-- SPDX-License-Identifier: MIT -->

# Upstream DuckDB reports (pending)

Working notes for reports to the DuckDB project about C API behaviour found
while auditing quack-rs (see `AUDIT.md`, sections 8.5 and 9). None has been
filed.
Each entry is written so it can be copied into a DuckDB issue once reviewed.

## Status

| # | Subject | Reproducer | quack-rs mitigation |
|---|---|---|---|
| 1 | `duckdb_list_vector_reserve` lets an exception escape | C program, below | `ListBuilder` limits and element cap |
| 2 | `duckdb_catalog_get_entry` lets a failed autoload escape | C program, below | autoloading names refused in `catalog` |
| 3 | `duckdb_config_option_set_default_value` lets a cast exception escape | C program, below | default converted by SQL before the call |
| 4 | `duckdb_data_chunk_from_arrow` converts before its `try` | C program, below | negative length refused; a very large length cannot be checked and is documented |
| 5 | Scalar bind data does not take part in expression equality | quack-rs test | documented on `ScalarBindData` |
| 6 | Aggregate states not all destroyed after a failed `finalize` | quack-rs test | documented under Known Limitations |
| 7 | C API aggregate in a streaming window re-reads the first row | C program, below | every aggregate registers a destructor |
| 8 | Arrow import returns dictionary and constant vectors the C API cannot read | C program, below | columns flattened after import |
| 9 | Arrow dictionary import with NULLs and >2048 entries writes past a heap buffer | C program, below (valgrind) | such arrays refused before import |
| 10 | `destroy` receives states `init` never ran on | C program, below | tagged state wrappers |
| 11 | `COPY ... FROM` options uncast; a value-less option dropped | C program, below | documented |
| 12 | `bind_get_argument` fails the query for an uncopyable argument (aborts before v1.5.5) | C program, below | no inspection before v1.5.5; readable message |
| 13 | `duckdb_get_varchar` / `duckdb_value_to_string` let an exception escape | C program, below | payloads checked before rendering |
| 14 | Out-of-range TIME accepted by bind, append and vectors; rendering crashes | C program, below | bind and append refuse; Safety clause |
| 15 | Arrow import ignores the parent's offset for values | C program, below | nonzero offset refused |
| 16 | Arrow export corrupts large INTERVAL and UHUGEINT values | C program, below | documented |
| 17 | Literal types accepted; first use invalidates the database | C program, below | refused by `LogicalType::try_new` |
| 18 | Zero-size ARRAY / empty UNION depend on the build | C program, below | refused |
| 19 | `epoch_us(interval)` fails when one field overflows | SQL, below | exact total computed in `i128` |
| 20 | Grouped-aggregate states a stopped scan never reached are never destroyed | C program, below | small states stored inline; documented |
| 21 | Arrow import reads one byte past a validity bitmap at an unaligned bit offset | C program, below (valgrind) | Safety clause on `data_chunk_from_arrow` |

## Before filing

- Search the DuckDB issue tracker for existing reports of each item; this was
  not done when the notes were written.
- Re-run each reproducer against the current DuckDB release. The notes record
  DuckDB v1.5.5 behaviour, and a source reading of DuckDB `main` at
  `30c64e17` (2026-09-23), where the C API files moved to
  `src/main/capi/v1/`; `main` was not built.
- Item 6 has no plain-C reproducer yet.
- Items 7 to 19 come from the fourth audit (`AUDIT.md` section 9). Their
  reproducers were compiled and run against the prebuilt v1.4.4, v1.5.0 and
  v1.5.5 libraries, where each entry says so, on 2026-09-24; output is quoted
  as observed except where an entry says it abbreviates.
- Items 20 and later come from the fifth audit (`AUDIT.md` section 10). Their
  reproducers were compiled and run against the prebuilt libraries of every
  release each entry names, on 2026-09-24; output is quoted as observed.

## Removal

Remove an entry once DuckDB has fixed it in a release quack-rs supports (or
declined it) and any quack-rs workaround and documentation have been updated
to match. Delete this file when no entries remain.

## Background

When a C++ exception leaves an `extern "C"` function, a C caller gets
`std::terminate`, and a Rust caller aborts the process: Rust cannot unwind a
foreign exception. Other C API functions catch internally and return an error
state (for example `duckdb_register_config_option` and
`duckdb_schema_from_arrow`, in the same files as items 3 and 4); items 1 to 4
do not.

Reproducers 1 to 4 were compiled and run against the DuckDB v1.5.5 prebuilt
library; their output is quoted as observed, except that item 2 shortens a home
directory to `~` and item 4 omits `stack_trace_pointers`.

---

## 1. `duckdb_list_vector_reserve` aborts on a capacity above `MAX_VECTOR_SIZE`

`duckdb_list_vector_reserve` returns `duckdb_state`, but it calls
`ListVector::Reserve` outside any try block, and `VectorListBuffer::Reserve`
throws `OutOfRangeException` for `to_reserve > DConstants::MAX_VECTOR_SIZE`
(`src/common/types/vector_buffer.cpp`). An allocation failure during the
resize would escape the same way.

Source: `src/main/capi/data_chunk-c.cpp:207` (v1.5.5), `src/main/capi/v1/data_chunk-c.cpp:219` (main).

Environment: DuckDB v1.5.5 prebuilt `libduckdb` (linux_amd64), gcc 13.3.0, x86_64 Linux.
Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_list_vector_reserve lets a C++ exception escape the C API.
#include <stdio.h>
#include "duckdb.h"
int main(void) {
    duckdb_logical_type child = duckdb_create_logical_type(DUCKDB_TYPE_INTEGER);
    duckdb_logical_type list = duckdb_create_list_type(child);
    duckdb_vector vec = duckdb_create_vector(list, 1);
    // One more than MAX_VECTOR_SIZE (2^37): VectorListBuffer::Reserve throws.
    idx_t too_big = ((idx_t)1 << 37) + 1;
    fprintf(stderr, "calling duckdb_list_vector_reserve(%llu)\n", (unsigned long long)too_big);
    duckdb_state st = duckdb_list_vector_reserve(vec, too_big);
    fprintf(stderr, "returned %s\n", st == DuckDBSuccess ? "DuckDBSuccess" : "DuckDBError");
    duckdb_destroy_vector(&vec);
    duckdb_destroy_logical_type(&list);
    duckdb_destroy_logical_type(&child);
    return 0;
}
```

Observed:

```text
calling duckdb_list_vector_reserve(137438953473)
terminate called after throwing an instance of 'duckdb::OutOfRangeException'
  what():  {"exception_type":"Out of Range","exception_message":"Cannot resize vector to 137438953473 rows: maximum allowed vector size is 128.0 GiB"}
Aborted (exit 134)
```

Expected: `DuckDBError`. Suggested fix: wrap the call in `try { ... } catch (...) { return DuckDBError; }`.

---

## 2. `duckdb_catalog_get_entry` aborts when a lookup triggers a failed autoload

With `OnEntryNotFound::RETURN_NULL`, a missing entry should give `nullptr`.
But when the name is one that `EXTENSION_TYPES` / `EXTENSION_COLLATIONS` /
`EXTENSION_FUNCTIONS` map to an autoloadable extension, `Catalog::GetEntry`
calls `AutoLoadExtensionByCatalogEntry` → `ExtensionHelper::AutoLoadExtension`,
which throws `AutoloadException` when the extension can't be installed or loaded
(`src/main/extension/extension_helper.cpp:414-416`). The C function has no
try/catch, so e.g. an offline machine, or `autoinstall_known_extensions = false`,
turns a plain type lookup into a process abort.

Source: `src/main/capi/catalog-c.cpp:118` (v1.5.5), `src/main/capi/v1/catalog-c.cpp:118` (main).

Environment: DuckDB v1.5.5 prebuilt `libduckdb` (linux_amd64), gcc 13.3.0, x86_64 Linux.
Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_catalog_get_entry lets an AutoloadException escape the C API.
#include <stdio.h>
#include "duckdb.h"
static void run(duckdb_connection con, const char *sql) {
    duckdb_result r;
    if (duckdb_query(con, sql, &r) != DuckDBSuccess) fprintf(stderr, "%s: %s\n", sql, duckdb_result_error(&r));
    duckdb_destroy_result(&r);
}
int main(void) {
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    // inet is not statically linked into libduckdb; with autoinstall off,
    // autoloading it for the type name "inet" must fail.
    run(con, "SET autoinstall_known_extensions = false");
    run(con, "SET autoload_known_extensions = true");
    // duckdb_client_context_get_catalog needs an active transaction.
    run(con, "BEGIN TRANSACTION");
    duckdb_client_context ctx; duckdb_connection_get_client_context(con, &ctx);
    duckdb_catalog cat = duckdb_client_context_get_catalog(ctx, "memory");
    fprintf(stderr, "ctx=%p catalog=%p\n", (void *)ctx, (void *)cat);
    fprintf(stderr, "calling duckdb_catalog_get_entry(TYPE, main, inet)\n");
    duckdb_catalog_entry e = duckdb_catalog_get_entry(cat, ctx, DUCKDB_CATALOG_ENTRY_TYPE_TYPE, "main", "inet");
    fprintf(stderr, "returned %p\n", (void *)e);
    return 0;
}
```

Observed:

```text
ctx=0x562845a7c610 catalog=0x562845a7c9e0
calling duckdb_catalog_get_entry(TYPE, main, inet)
terminate called after throwing an instance of 'duckdb::AutoloadException'
  what():  {"exception_type":"Extension Autoloading","exception_message":"An error occurred while trying to automatically install the required extension 'inet':\nExtension \"~/.duckdb/extensions/v1.5.5/linux_amd64/inet.duckdb_extension\" not found.\nExtension \"inet\" is an existing extension.\n\nInstall it first using \"INSTALL inet\"."}
Aborted (exit 134)
```

Expected: `nullptr` (entry not found), or an error the caller can see. Suggested
fix: catch in `duckdb_catalog_get_entry` and return `nullptr`. The function has no
error channel, so a `duckdb_error_data`-returning variant may be worth adding.

---

## 3. `duckdb_config_option_set_default_value` aborts on a default that does not cast

When the option's type is already set and the value's type differs, the
function stores `cvalue->DefaultCastAs(coption->type, false)` with no try/catch.
`DefaultCastAs` throws `InvalidInputException` for a value that doesn't convert.
It also uses only the built-in casts, so a string that SQL on the same connection
accepts (e.g. a `TIMESTAMPTZ` with a named time zone once ICU is loaded) can
still throw here.

Source: `src/main/capi/config_options-c.cpp:48-66` (v1.5.5), `src/main/capi/v1/config_options-c.cpp:62` (main).

Environment: DuckDB v1.5.5 prebuilt `libduckdb` (linux_amd64), gcc 13.3.0, x86_64 Linux.
Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_config_option_set_default_value lets a cast exception escape the C API.
#include <stdio.h>
#include "duckdb.h"
int main(void) {
    duckdb_config_option opt = duckdb_create_config_option();
    duckdb_config_option_set_name(opt, "repro_option");
    duckdb_logical_type ty = duckdb_create_logical_type(DUCKDB_TYPE_INTEGER);
    duckdb_config_option_set_type(opt, ty);
    duckdb_value v = duckdb_create_varchar("not a number");
    fprintf(stderr, "calling duckdb_config_option_set_default_value(INTEGER option, 'not a number')\n");
    duckdb_config_option_set_default_value(opt, v);
    fprintf(stderr, "returned\n");
    duckdb_destroy_value(&v);
    duckdb_destroy_logical_type(&ty);
    duckdb_destroy_config_option(&opt);
    return 0;
}
```

Observed:

```text
calling duckdb_config_option_set_default_value(INTEGER option, 'not a number')
terminate called after throwing an instance of 'duckdb::InvalidInputException'
  what():  {"exception_type":"Invalid Input","exception_message":"Failed to cast value: Could not convert string 'not a number' to INT32"}
Aborted (exit 134)
```

Expected: the call fails without aborting. It returns `void`, so options are:
use `TryCastAs` and leave the default unset on failure, or make
`duckdb_register_config_option` fail later. A `duckdb_state`-returning variant
would let callers see the error.

---

## 4. `duckdb_data_chunk_from_arrow` converts and allocates before its try block

The function returns `duckdb_error_data` and catches exceptions from the column
conversion. But the first thing it does, `dchunk->Initialize(...,
NumericCast<idx_t>(arrow_array->length))`, sits before the `try`. A negative
`length` makes `NumericCast` throw `InternalException`, and a very large one
makes the allocation throw. Both escape the C API.

Source: `src/main/capi/arrow-c.cpp:106` (v1.5.5), `src/main/capi/v1/arrow-c.cpp:105,108` (main).

Environment: DuckDB v1.5.5 prebuilt `libduckdb` (linux_amd64), gcc 13.3.0, x86_64 Linux.
Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_data_chunk_from_arrow converts arrow_array->length before its try block.
#include <stdio.h>
#include <string.h>
#include "duckdb.h"
#ifndef ARROW_C_DATA_INTERFACE
#define ARROW_C_DATA_INTERFACE
struct ArrowSchema {
    const char *format; const char *name; const char *metadata; int64_t flags; int64_t n_children;
    struct ArrowSchema **children; struct ArrowSchema *dictionary;
    void (*release)(struct ArrowSchema *); void *private_data;
};
struct ArrowArray {
    int64_t length; int64_t null_count; int64_t offset; int64_t n_buffers; int64_t n_children;
    const void **buffers; struct ArrowArray **children; struct ArrowArray *dictionary;
    void (*release)(struct ArrowArray *); void *private_data;
};
#endif
static void noop_schema(struct ArrowSchema *s) { s->release = NULL; }
static void noop_array(struct ArrowArray *a) { a->release = NULL; }
int main(void) {
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    // A struct<a: int32> schema, as a record batch is exported.
    struct ArrowSchema child = {"i", "a", NULL, 2, 0, NULL, NULL, noop_schema, NULL};
    struct ArrowSchema *children[1] = {&child};
    struct ArrowSchema root = {"+s", "", NULL, 0, 1, children, NULL, noop_schema, NULL};
    duckdb_arrow_converted_schema converted = NULL;
    duckdb_error_data err = duckdb_schema_from_arrow(con, &root, &converted);
    if (err) { fprintf(stderr, "schema: %s\n", duckdb_error_data_message(err)); return 1; }
    int32_t values[1] = {7};
    const void *child_buffers[2] = {NULL, values};
    struct ArrowArray child_arr = {1, 0, 0, 2, 0, child_buffers, NULL, NULL, noop_array, NULL};
    struct ArrowArray *child_arrs[1] = {&child_arr};
    const void *root_buffers[1] = {NULL};
    // An invalid (negative) length: the function has an error channel for bad
    // input, but this check throws before its try block starts.
    struct ArrowArray arr = {-1, 0, 0, 1, 1, root_buffers, child_arrs, NULL, noop_array, NULL};
    duckdb_data_chunk chunk = NULL;
    fprintf(stderr, "calling duckdb_data_chunk_from_arrow(length = -1)\n");
    err = duckdb_data_chunk_from_arrow(con, &arr, converted, &chunk);
    fprintf(stderr, "returned %s\n", err ? duckdb_error_data_message(err) : "no error");
    return 0;
}
```

Observed:

```text
calling duckdb_data_chunk_from_arrow(length = -1)
terminate called after throwing an instance of 'duckdb::InternalException'
  what():  {"exception_type":"INTERNAL","exception_message":"Information loss on integer cast: value -1 outside of target range [0, 18446744073709551615]", ...}
Aborted (exit 134)
```

(`stack_trace_pointers` elided from the message.)

Expected: a `duckdb_error_data` with `DUCKDB_ERROR_INVALID_INPUT`. Suggested fix:
validate `length >= 0` and move `Initialize` inside the existing try block.
Related: `length == 0` reaches `Allocator::AllocateData(0)`, whose
`D_ASSERT(size > 0)` fails in a debug build.

---

## 5. Scalar bind data does not take part in expression equality (request)

`CScalarFunctionBindData::Equals` compares only `extra_info` and the callback
pointer, not the bind data the bind callback stored
(`src/main/capi/scalar_function-c.cpp`, v1.5.5; `src/main/capi/v1/scalar_function-c.cpp:53`
on main; the aggregate equivalent is at `aggregate_function-c.cpp:50`). So two
calls with identical arguments whose bind callbacks stored different data are
merged by common-subexpression elimination, and one call's bind data answers
for both. This is not a crash; the C API simply has no way for an extension to
say how its bind data compares.

Evidence: a quack-rs test registers a scalar whose bind callback stores an
incrementing counter. `SELECT f(i), f(i) FROM t` runs the bind callback twice
yet returns the same value in both columns. Marking the function volatile
keeps them apart (`tests/ffi_roundtrip/lifecycle.rs`,
`identical_calls_share_bind_data_unless_the_function_is_volatile`; passes
against 1.5.5).

Request: an optional `duckdb_scalar_function_set_bind_data_equals` callback
(and an aggregate twin). Or, if none is set, treat two non-null bind data
pointers as unequal unless a copy callback made one from the other. As it
stands, extension authors have to know to mark such functions volatile.

---

## 6. Aggregate states are not all destroyed when `finalize` reports an error

With a C API aggregate whose `init` and `destroy` callbacks only count, a query
whose `finalize` calls `duckdb_aggregate_function_set_error` initialised 2
states and destroyed 1 (`SELECT agg(i) FROM range(3) t(i)`), and 4 and 2 with
`GROUP BY i % 2`. The same queries with a succeeding `finalize` destroy every
state. Any resource a state owns therefore leaks on this path, and the
extension cannot tell which states were abandoned. Observed on 1.5.5 through
quack-rs (`tests/ffi_roundtrip/lifecycle.rs`,
`aggregate_states_are_not_all_destroyed_when_finalize_fails`); LeakSanitizer
reports the leaked Rust allocation. Not yet reduced to a plain-C reproducer,
and where DuckDB drops the states has not been located: `CAPIAggregateFinalize`
throws `InvalidInputException` after the callback, and
`~UngroupedAggregateState` would destroy the global state if it ran.

---

## 7. A C API aggregate in a streaming window re-reads the first row

A running window (`OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)`,
no `PARTITION BY` / `ORDER BY`) over a C API aggregate with **no destructor**
is streamed (`PhysicalStreamingWindow::IsStreamingFunction` refuses only
aggregates with one). The streaming path calls `update` once per row with a
count of 1 on a one-row dictionary slice whose selection it moves along
(`physical_streaming_window.cpp:490-505`). `CAPIAggregateUpdate` then calls
`inputs[c].Flatten(count)` on the caller's vector in place
(`aggregate_function-c.cpp:92-111`), which turns the slice into a flat vector
holding row 0's value, so every later row of the chunk re-reads row 0. With
any destructor the window is not streamed and the answer is right.

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5 (linux_amd64).
Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`. Pass any argument to register a no-op destructor.

```c
// A C API aggregate (no destructor) evaluated as a streaming running window
// sees the first row's argument on every update of a chunk.
#include <stdio.h>
#include <stdint.h>
#include "duckdb.h"

static idx_t st_size(duckdb_function_info info) { return sizeof(int64_t); }
static void st_init(duckdb_function_info info, duckdb_aggregate_state s) { *(int64_t *)s = 0; }
static void st_update(duckdb_function_info info, duckdb_data_chunk input, duckdb_aggregate_state *states) {
	idx_t n = duckdb_data_chunk_get_size(input);
	duckdb_vector v = duckdb_data_chunk_get_vector(input, 0);
	int64_t *data = (int64_t *)duckdb_vector_get_data(v);
	for (idx_t i = 0; i < n; i++) {
		*(int64_t *)states[i] += data[i];
	}
}
static void st_combine(duckdb_function_info info, duckdb_aggregate_state *src, duckdb_aggregate_state *dst, idx_t n) {
	for (idx_t i = 0; i < n; i++) {
		*(int64_t *)dst[i] += *(int64_t *)src[i];
	}
}
static void st_finalize(duckdb_function_info info, duckdb_aggregate_state *src, duckdb_vector result, idx_t n,
                        idx_t offset) {
	int64_t *out = (int64_t *)duckdb_vector_get_data(result);
	for (idx_t i = 0; i < n; i++) {
		out[offset + i] = *(int64_t *)src[i];
	}
}

static void st_destroy(duckdb_aggregate_state *s, idx_t n) {}

int main(int argc, char **argv) {
	duckdb_database db;
	duckdb_connection con;
	duckdb_open(NULL, &db);
	duckdb_connect(db, &con);
	duckdb_aggregate_function f = duckdb_create_aggregate_function();
	duckdb_aggregate_function_set_name(f, "capi_sum");
	duckdb_logical_type bigint = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
	duckdb_aggregate_function_add_parameter(f, bigint);
	duckdb_aggregate_function_set_return_type(f, bigint);
	duckdb_aggregate_function_set_functions(f, st_size, st_init, st_update, st_combine, st_finalize);
	if (argc > 1) {
		// With any destructor the planner no longer streams the window.
		duckdb_aggregate_function_set_destructor(f, st_destroy);
		printf("(with a no-op destructor)\n");
	}
	if (duckdb_register_aggregate_function(con, f) != DuckDBSuccess) {
		fprintf(stderr, "register failed\n");
		return 1;
	}
	const char *sql =
	    "SELECT x, capi_sum(x) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS capi, "
	    "sum(x) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS builtin "
	    "FROM (SELECT range + 1 AS x FROM range(5)) ORDER BY x";
	duckdb_result res;
	if (duckdb_query(con, sql, &res) != DuckDBSuccess) {
		fprintf(stderr, "query failed: %s\n", duckdb_result_error(&res));
		return 1;
	}
	printf("x capi_sum builtin_sum\n");
	for (idx_t r = 0; r < duckdb_row_count(&res); r++) {
		printf("%lld %lld %lld\n", (long long)duckdb_value_int64(&res, 0, r), (long long)duckdb_value_int64(&res, 1, r),
		       (long long)duckdb_value_int64(&res, 2, r));
	}
	duckdb_destroy_result(&res);
	// Control: the same aggregate as a plain (non-streaming) window.
	const char *sql2 = "SELECT x, capi_sum(x) OVER (ORDER BY x ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) "
	                   "FROM (SELECT range + 1 AS x FROM range(5)) ORDER BY x";
	duckdb_query(con, sql2, &res);
	printf("control (ORDER BY, not streamed): ");
	for (idx_t r = 0; r < duckdb_row_count(&res); r++) {
		printf("%lld ", (long long)duckdb_value_int64(&res, 1, r));
	}
	printf("\n");
	duckdb_destroy_result(&res);
	duckdb_destroy_logical_type(&bigint);
	duckdb_destroy_aggregate_function(&f);
	duckdb_disconnect(&con);
	duckdb_close(&db);
	return 0;
}
```

Observed, identical on all three releases (columns `x`, the C API aggregate,
the built-in `sum`):

```text
x capi_sum builtin_sum
1 1 1
2 2 3
3 3 6
4 4 10
5 5 15
control (ORDER BY, not streamed): 1 3 6 10 15
```

With the destructor argument, the C API column reads `1 3 6 10 15`.

Expected: the running sum. Suggested fix: flatten a copy in
`CAPIAggregateUpdate`, or have the streaming path hand the C API a flat
vector per row.

quack-rs mitigation: every aggregate registers a destructor (a no-op when the
user gives none), so none is streamed.

---

## 8. `duckdb_data_chunk_from_arrow` returns vectors the C API cannot read as flat

A dictionary-encoded Arrow column is imported as a DuckDB *dictionary* vector,
and a column of Arrow's null type as a *constant* vector. The C API has no
function that reports a vector's physical format or flattens it, and
`duckdb_vector_get_data` returns the underlying buffer, so row `i` is not
element `i`: here the base buffer holds the 2 dictionary entries plus one, and
reading row 3 onwards is out of bounds. (`duckdb_data_chunk_from_arrow`,
`arrow-c.cpp:94`; `ColumnArrowToDuckDBDictionary`, `arrow_conversion.cpp:1391`.)

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_data_chunk_from_arrow returns a dictionary vector for a
// dictionary-encoded column; duckdb_vector_get_data then does not map row i
// to element i, and the C API offers no way to detect or flatten it.
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#ifndef ARROW_C_DATA_INTERFACE
#define ARROW_C_DATA_INTERFACE
struct ArrowSchema {
	const char *format; const char *name; const char *metadata; int64_t flags; int64_t n_children;
	struct ArrowSchema **children; struct ArrowSchema *dictionary;
	void (*release)(struct ArrowSchema *); void *private_data;
};
struct ArrowArray {
	int64_t length; int64_t null_count; int64_t offset; int64_t n_buffers; int64_t n_children;
	const void **buffers; struct ArrowArray **children; struct ArrowArray *dictionary;
	void (*release)(struct ArrowArray *); void *private_data;
};
#endif
#include "duckdb.h"

int main(void) {
	duckdb_database db;
	duckdb_connection con;
	duckdb_open(NULL, &db);
	duckdb_connect(db, &con);
	duckdb_result res;
	if (duckdb_query(con,
	                 "SELECT (CASE WHEN range % 2 = 0 THEN 'bb' ELSE 'aa' END)::ENUM('aa','bb') AS e FROM range(2048)",
	                 &res) != DuckDBSuccess) {
		return 1;
	}
	duckdb_data_chunk chunk = duckdb_fetch_chunk(res);
	duckdb_logical_type ty = duckdb_column_logical_type(&res, 0);
	duckdb_arrow_options opts;
	duckdb_connection_get_arrow_options(con, &opts);
	struct ArrowSchema schema;
	const char *names[] = {"e"};
	duckdb_error_data err = duckdb_to_arrow_schema(opts, &ty, names, 1, &schema);
	if (err) { printf("to_arrow_schema: %s\n", duckdb_error_data_message(err)); return 1; }
	printf("exported column format '%s', dictionary %s\n", schema.children[0]->format,
	       schema.children[0]->dictionary ? "yes" : "no");
	struct ArrowArray array;
	err = duckdb_data_chunk_to_arrow(opts, chunk, &array);
	if (err) { printf("to_arrow: %s\n", duckdb_error_data_message(err)); return 1; }
	duckdb_arrow_converted_schema converted;
	err = duckdb_schema_from_arrow(con, &schema, &converted);
	if (err) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(err)); return 1; }
	duckdb_data_chunk back = NULL;
	err = duckdb_data_chunk_from_arrow(con, &array, converted, &back);
	if (err) { printf("from_arrow: %s\n", duckdb_error_data_message(err)); return 1; }
	duckdb_vector v = duckdb_data_chunk_get_vector(back, 0);
	duckdb_logical_type bt = duckdb_vector_get_column_type(v);
	printf("imported %llu rows, type id %d (VARCHAR = %d)\n", (unsigned long long)duckdb_data_chunk_get_size(back),
	       (int)duckdb_get_type_id(bt), (int)DUCKDB_TYPE_VARCHAR);
	duckdb_string_t *data = (duckdb_string_t *)duckdb_vector_get_data(v);
	int wrong = 0;
	for (idx_t i = 0; i < duckdb_data_chunk_get_size(back); i++) {
		// Rows 0..2 only: the base buffer holds dictionary length + 1 = 3 entries,
		// and reading further is out of bounds.
		if (i < 3) {
			printf("row %llu: len %u '%.*s' (expected '%s')\n", (unsigned long long)i, data[i].value.inlined.length,
			       (int)data[i].value.inlined.length, data[i].value.inlined.inlined, i % 2 == 0 ? "bb" : "aa");
		}
	}
	(void)wrong;
	duckdb_destroy_logical_type(&bt);
	duckdb_destroy_data_chunk(&back);
	duckdb_destroy_arrow_converted_schema(&converted);
	schema.release(&schema);
	duckdb_destroy_arrow_options(&opts);
	duckdb_destroy_logical_type(&ty);
	duckdb_destroy_data_chunk(&chunk);
	duckdb_destroy_result(&res);
	duckdb_disconnect(&con);
	duckdb_close(&db);
	return 0;
}
```

Observed (v1.5.5; v1.4.4 identical; v1.5.0 identical except that row 2
reads a length of 48, i.e. whatever the buffer holds):

```text
exported column format 'C', dictionary yes
imported 2048 rows, type id 17 (VARCHAR = 17)
row 0: len 2 'aa' (expected 'bb')
row 1: len 2 'bb' (expected 'aa')
row 2: len 0 '' (expected 'bb')
```

Expected: either flat vectors from this C function, or a C API way to flatten
(`duckdb_vector_flatten`) or to read a vector's format. quack-rs mitigation:
`data_chunk_from_arrow` copies dictionary and constant columns into flat
vectors (`copy_selected` with an identity selection) before returning them.

---

## 9. Importing a dictionary array with NULLs and more than 2048 entries writes past a heap buffer

`ColumnArrowToDuckDBDictionary` default-constructs its `indices_validity`
mask, which `EnsureWritable` sizes for `STANDARD_VECTOR_SIZE` (2048) rows, and
`GetValidityMask` (`arrow_conversion.cpp:47`) then copies the validity of all
of the dictionary array's entries into it. A `LIST` of 2048 two-element lists
is enough: its child holds 4096 entries.

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5; valgrind 3.22.
Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_data_chunk_from_arrow writes past a heap buffer when a
// dictionary-encoded array with NULLs holds more than STANDARD_VECTOR_SIZE
// (2048) entries: ColumnArrowToDuckDBDictionary default-constructs
// `indices_validity` (2048 bits after EnsureWritable) and GetValidityMask
// memcpys `size` bits into it. Here a LIST column's child holds 4096 entries.
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#ifndef ARROW_C_DATA_INTERFACE
#define ARROW_C_DATA_INTERFACE
struct ArrowSchema {
	const char *format; const char *name; const char *metadata; int64_t flags; int64_t n_children;
	struct ArrowSchema **children; struct ArrowSchema *dictionary;
	void (*release)(struct ArrowSchema *); void *private_data;
};
struct ArrowArray {
	int64_t length; int64_t null_count; int64_t offset; int64_t n_buffers; int64_t n_children;
	const void **buffers; struct ArrowArray **children; struct ArrowArray *dictionary;
	void (*release)(struct ArrowArray *); void *private_data;
};
#endif
#include "duckdb.h"

int main(void) {
	duckdb_database db;
	duckdb_connection con;
	duckdb_open(NULL, &db);
	duckdb_connect(db, &con);
	duckdb_result res;
	if (duckdb_query(con,
	                 "SELECT [e, e] AS l FROM (SELECT (CASE WHEN range % 7 = 3 THEN NULL ELSE 'aa' END)"
	                 "::ENUM('aa') AS e FROM range(2048))",
	                 &res) != DuckDBSuccess) {
		printf("query: %s\n", duckdb_result_error(&res));
		return 1;
	}
	duckdb_data_chunk chunk = duckdb_fetch_chunk(res);
	duckdb_logical_type ty = duckdb_column_logical_type(&res, 0);
	duckdb_arrow_options opts;
	duckdb_connection_get_arrow_options(con, &opts);
	struct ArrowSchema schema;
	const char *names[] = {"l"};
	duckdb_error_data err = duckdb_to_arrow_schema(opts, &ty, names, 1, &schema);
	if (err) { printf("to_arrow_schema: %s\n", duckdb_error_data_message(err)); return 1; }
	struct ArrowArray array;
	err = duckdb_data_chunk_to_arrow(opts, chunk, &array);
	if (err) { printf("to_arrow: %s\n", duckdb_error_data_message(err)); return 1; }
	printf("list child: %lld entries, %lld null, dictionary %s\n", (long long)array.children[0]->children[0]->length,
	       (long long)array.children[0]->children[0]->null_count,
	       array.children[0]->children[0]->dictionary ? "yes" : "no");
	duckdb_arrow_converted_schema converted;
	err = duckdb_schema_from_arrow(con, &schema, &converted);
	if (err) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(err)); return 1; }
	duckdb_data_chunk back = NULL;
	err = duckdb_data_chunk_from_arrow(con, &array, converted, &back);
	if (err) { printf("from_arrow: %s\n", duckdb_error_data_message(err)); return 1; }
	printf("imported %llu rows\n", (unsigned long long)duckdb_data_chunk_get_size(back));
	duckdb_destroy_data_chunk(&back);
	duckdb_destroy_arrow_converted_schema(&converted);
	schema.release(&schema);
	duckdb_destroy_arrow_options(&opts);
	duckdb_destroy_logical_type(&ty);
	duckdb_destroy_data_chunk(&chunk);
	duckdb_destroy_result(&res);
	duckdb_disconnect(&con);
	duckdb_close(&db);
	return 0;
}
```

Observed: the process dies (SIGSEGV on v1.4.4 and v1.5.5; on v1.5.0
`munmap_chunk(): invalid pointer` and SIGABRT). Under valgrind, on all three:

```text
Invalid write of size 8
   at 0x4852E0B: memmove (in /usr/libexec/valgrind/vgpreload_memcheck-amd64-linux.so)
   by duckdb::GetValidityMask(duckdb::ValidityMask&, ArrowArray&, unsigned long, unsigned long, long, long, bool)
   by duckdb::ArrowToDuckDBConversion::ColumnArrowToDuckDBDictionary(...)
   by duckdb_data_chunk_from_arrow
   by main (arrow_dict_validity_overflow.c:55)
```

Expected: the mask sized for the dictionary array's length. quack-rs
mitigation: `data_chunk_from_arrow` refuses, before calling DuckDB, any
dictionary-encoded array anywhere in the tree that has NULLs and more than
`duckdb_vector_size()` entries.

---

## 10. After a `state_init` error, `destroy` receives states `init` never ran on

When an aggregate's `state_init` reports an error
(`duckdb_aggregate_function_set_error`), the query fails as expected, but the
destructor is then called on every state allocated for the batch, including
those whose `init` never ran (their memory is uninitialised). A destructor
that frees what the state points to frees garbage. (`CAPIAggregateStateInit`,
`aggregate_function-c.cpp:82`.)

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5; `PRAGMA
threads=1`, because the counters are plain `static`s. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`. The optional
argument is the `init` call that fails (default 3000).

```c
// Plain C: an aggregate whose state_init reports an error on the Nth call.
// Counts how many states DuckDB passes to destroy vs how many init initialised.
#include <duckdb.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#define MAGIC 0x5157414bULL
typedef struct { uint64_t magic; int64_t sum; } st_t;
static long inits = 0, fail_at = -1, destroyed = 0, destroyed_uninit = 0;
static idx_t size_cb(duckdb_function_info i) { return sizeof(st_t); }
static void init_cb(duckdb_function_info info, duckdb_aggregate_state s) {
    if (inits++ == fail_at) { duckdb_aggregate_function_set_error(info, "init failed"); return; }
    st_t *p = (st_t *)s; p->magic = MAGIC; p->sum = 0;
}
static void update_cb(duckdb_function_info info, duckdb_data_chunk in, duckdb_aggregate_state *states) {
    idx_t n = duckdb_data_chunk_get_size(in);
    int64_t *d = (int64_t *)duckdb_vector_get_data(duckdb_data_chunk_get_vector(in, 0));
    for (idx_t i = 0; i < n; i++) ((st_t *)states[i])->sum += d[i];
}
static void combine_cb(duckdb_function_info info, duckdb_aggregate_state *src, duckdb_aggregate_state *tgt, idx_t n) {
    for (idx_t i = 0; i < n; i++) ((st_t *)tgt[i])->sum += ((st_t *)src[i])->sum;
}
static void fin_cb(duckdb_function_info info, duckdb_aggregate_state *src, duckdb_vector res, idx_t n, idx_t off) {
    int64_t *d = (int64_t *)duckdb_vector_get_data(res);
    for (idx_t i = 0; i < n; i++) d[off + i] = ((st_t *)src[i])->sum;
}
static void destroy_cb(duckdb_aggregate_state *states, idx_t n) {
    for (idx_t i = 0; i < n; i++) {
        destroyed++;
        st_t *p = (st_t *)states[i];
        if (p->magic != MAGIC) destroyed_uninit++;
        p->magic = 0;
    }
}
int main(int argc, char **argv) {
    fail_at = argc > 1 ? atol(argv[1]) : 3000;
    duckdb_database db; duckdb_connection con; duckdb_result r;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_query(con, "PRAGMA threads=1", NULL);
    duckdb_aggregate_function f = duckdb_create_aggregate_function();
    duckdb_aggregate_function_set_name(f, "csum");
    duckdb_logical_type bi = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
    duckdb_aggregate_function_add_parameter(f, bi);
    duckdb_aggregate_function_set_return_type(f, bi);
    duckdb_aggregate_function_set_functions(f, size_cb, init_cb, update_cb, combine_cb, fin_cb);
    duckdb_aggregate_function_set_destructor(f, destroy_cb);
    printf("register: %d\n", duckdb_register_aggregate_function(con, f));
    duckdb_destroy_aggregate_function(&f);
    duckdb_destroy_logical_type(&bi);
    duckdb_state st = duckdb_query(con, "SELECT sum(a) FROM (SELECT i%5000 g, csum(i) a FROM range(200000) t(i) GROUP BY g)", &r);
    printf("query state=%d error=%s\n", st, st ? duckdb_result_error(&r) : "none");
    duckdb_destroy_result(&r);
    printf("init calls=%ld (successful %ld), destroy saw %ld states, %ld never initialised\n",
           inits, inits - 1, destroyed, destroyed_uninit);
    duckdb_disconnect(&con); duckdb_close(&db);
}
```

Observed, identical on all three releases:

```text
register: 0
query state=1 error=Invalid Input Error: init failed
init calls=3001 (successful 3000), destroy saw 4096 states, 1096 never initialised
```

"Never initialised" is detected by a magic word `init` writes; a state whose
uninitialised memory happened to hold it would be miscounted, which does not
change the finding. Expected: `destroy` only for states `init` completed, or
a documented contract that says otherwise. quack-rs mitigation: each state's
wrapper carries a tag derived from its address that `init` writes and
`destroy` checks, so a never-initialised state is skipped.

---

## 11. `COPY ... FROM` passes options to the reader uncast, and drops a value-less option

The copy-from bind turns each option into a named parameter of the table
function (`copy_function-c.cpp:653-690`) without casting it to the type the
function declared, and skips an option given with no value (`FLAG`) with
`continue`. The same named parameters, passed in a table function call, are
cast (or rejected) as declared.

Environment: prebuilt `libduckdb` v1.5.0 and v1.5.5 (v1.4.4 has no copy
functions in its C API). Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// Plain C: COPY ... FROM passes options to the reader's named parameters
// without casting them to the declared type (and drops value-less options).
#include <stdio.h>
#include <string.h>
#include "duckdb.h"
static int done = 0;
static void bind(duckdb_bind_info info) {
    const char *names[] = {"skip", "flag"};
    for (int k = 0; k < 2; k++) {
        duckdb_value v = duckdb_bind_get_named_parameter(info, names[k]);
        if (!v) { printf("  named %s: <absent>\n", names[k]); continue; }
        duckdb_logical_type t = duckdb_get_value_type(v);
        char *s = duckdb_value_to_string(v);
        printf("  named %s: declared BIGINT/BOOLEAN, arrived as type id %d, value %s\n", names[k], (int)duckdb_get_type_id(t), s);
        duckdb_free(s);
        duckdb_destroy_value(&v);
    }
}
static void init(duckdb_init_info info) { (void)info; }
static void scan(duckdb_function_info info, duckdb_data_chunk out) { (void)info; duckdb_data_chunk_set_size(out, 0); }
static void run(duckdb_connection con, const char *sql) {
    duckdb_result r;
    if (duckdb_query(con, sql, &r) == DuckDBError) printf("ERR %s -> %s\n", sql, duckdb_result_error(&r));
    else printf("OK  %s\n", sql);
    duckdb_destroy_result(&r);
}
int main(void) {
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_table_function tf = duckdb_create_table_function();
    duckdb_table_function_set_name(tf, "cfmt");
    duckdb_logical_type vc = duckdb_create_logical_type(DUCKDB_TYPE_VARCHAR);
    duckdb_logical_type bi = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
    duckdb_logical_type bo = duckdb_create_logical_type(DUCKDB_TYPE_BOOLEAN);
    duckdb_table_function_add_parameter(tf, vc);
    duckdb_table_function_add_named_parameter(tf, "skip", bi);
    duckdb_table_function_add_named_parameter(tf, "flag", bo);
    duckdb_table_function_set_bind(tf, bind);
    duckdb_table_function_set_init(tf, init);
    duckdb_table_function_set_function(tf, scan);
    duckdb_copy_function cf = duckdb_create_copy_function();
    duckdb_copy_function_set_name(cf, "cfmt");
    duckdb_copy_function_set_copy_from_function(cf, tf);
    printf("register: %d\n", duckdb_register_copy_function(con, cf));
    duckdb_register_table_function(con, tf);
    run(con, "CREATE TABLE t(a BIGINT)");
    run(con, "COPY t FROM 'f' (FORMAT cfmt, SKIP 'abc', FLAG)");
    run(con, "SELECT * FROM cfmt('f', skip := 'abc')");
    run(con, "SELECT * FROM cfmt('f', skip := '12', flag := 'yes')");
    duckdb_destroy_copy_function(&cf); duckdb_destroy_table_function(&tf);
    duckdb_destroy_logical_type(&vc); duckdb_destroy_logical_type(&bi); duckdb_destroy_logical_type(&bo);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed, identical on v1.5.0 and v1.5.5 (type id 17 is VARCHAR, 5 BIGINT,
1 BOOLEAN):

```text
register: 0
OK  CREATE TABLE t(a BIGINT)
  named skip: declared BIGINT/BOOLEAN, arrived as type id 17, value 'abc'
  named flag: <absent>
OK  COPY t FROM 'f' (FORMAT cfmt, SKIP 'abc', FLAG)
ERR SELECT * FROM cfmt('f', skip := 'abc') -> Invalid Input Error: Failed to cast value: Could not convert string 'abc' to INT64
  named skip: declared BIGINT/BOOLEAN, arrived as type id 5, value 12
  named flag: declared BIGINT/BOOLEAN, arrived as type id 1, value true
```

Expected: options cast to the declared type (an error when they do not cast),
and a value-less option passed as `true` for a BOOLEAN parameter, as
DuckDB's own formats treat it. quack-rs mitigation: documented on
`CopyFunctionBuilder::copy_from`; the reader must check each option's type.

---

## 12. `duckdb_scalar_function_bind_get_argument` fails the query for an argument it cannot copy

To hand out an argument, the function copies its expression. For a scalar
subquery (`f((SELECT 7))`), or any argument containing one, the copy throws
("Cannot copy BoundSubqueryExpression"). From v1.5.5 the function catches that
and calls `duckdb_scalar_function_bind_set_error` itself with the raw
exception JSON (`scalar_function-c.cpp:352-365`), so the query fails whatever
the callback does next, and there is no way to ask first whether an argument
can be inspected. **Before v1.5.5** (checked in the source at tags v1.5.0 to
v1.5.4) there is no `try` at all: the exception leaves the C API through the
extension's bind callback, and a Rust extension aborts ("Rust cannot catch
foreign exceptions"; reproduced with quack-rs against v1.5.0).

Environment: prebuilt `libduckdb` v1.5.0 and v1.5.5. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_scalar_function_bind_get_argument fails the whole query when the
// argument is a scalar subquery: copying it throws, and the function itself
// calls duckdb_scalar_function_bind_set_error with the raw exception JSON.
#include <stdio.h>
#include <duckdb.h>
static void bind_cb(duckdb_bind_info info) {
    duckdb_expression e = duckdb_scalar_function_bind_get_argument(info, 0);
    printf("  bind: argument handle %s\n", e ? "returned" : "NULL");
    if (e) duckdb_destroy_expression(&e);
}
static void fn(duckdb_function_info info, duckdb_data_chunk in, duckdb_vector out) {
    int64_t *o = duckdb_vector_get_data(out);
    int64_t *x = duckdb_vector_get_data(duckdb_data_chunk_get_vector(in, 0));
    for (idx_t i = 0; i < duckdb_data_chunk_get_size(in); i++) o[i] = x[i];
}
static void q(duckdb_connection con, const char *sql) {
    duckdb_result r;
    printf("%s\n", sql);
    if (duckdb_query(con, sql, &r) == DuckDBSuccess) printf("  -> %lld\n", (long long)duckdb_value_int64(&r, 0, 0));
    else printf("  -> ERROR %s\n", duckdb_result_error(&r));
    duckdb_destroy_result(&r);
}
int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_logical_type big = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
    duckdb_scalar_function f = duckdb_create_scalar_function();
    duckdb_scalar_function_set_name(f, "inspect");
    duckdb_scalar_function_add_parameter(f, big);
    duckdb_scalar_function_set_return_type(f, big);
    duckdb_scalar_function_set_function(f, fn);
    duckdb_scalar_function_set_bind(f, bind_cb);
    printf("register: %d\n", duckdb_register_scalar_function(con, f));
    q(con, "SELECT inspect(7::BIGINT)");
    q(con, "SELECT inspect((SELECT 7::BIGINT))");
    q(con, "SELECT 42");
    duckdb_destroy_scalar_function(&f); duckdb_destroy_logical_type(&big);
    duckdb_disconnect(&con); duckdb_close(&db);
}
```

Observed on v1.5.5:

```text
register: 0
SELECT inspect(7::BIGINT)
  bind: argument handle returned
  -> 7
SELECT inspect((SELECT 7::BIGINT))
  bind: argument handle NULL
  -> ERROR Binder Error: {"exception_type":"Serialization","exception_message":"Cannot copy BoundSubqueryExpression"}
SELECT 42
  -> 42
```

On v1.5.0 the second query prints no `bind:` line — the exception unwinds
through the C callback (which survives only because C frames carry no
destructors) — and fails with `Serialization Error: Cannot copy
BoundSubqueryExpression`.

Expected: a null return without failing the bind, so the callback can decide,
or a way to test whether an argument is copyable. quack-rs mitigation:
`ScalarBindInfo::argument` asks for nothing on engines before v1.5.5 (it fails
the bind with an explanation instead), and on v1.5.5 replaces the JSON message.

---

## 13. `duckdb_get_varchar` and `duckdb_value_to_string` let a C++ exception escape

Both render the value with no `try` (`duckdb_value-c.cpp:309` and `:621`). A
TIMESTAMP whose payload is outside the range DuckDB converts — which plain SQL
produces, `make_timestamp(-9223372036854775808)` — throws `ConversionException`
("Date out of range in timestamp conversion"), and the process terminates.

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`. With the
argument `sql` it calls `duckdb_value_to_string`, otherwise
`duckdb_get_varchar`.

```c
// duckdb_get_varchar / duckdb_value_to_string let a C++ exception escape for an out-of-range TIMESTAMP
// that SQL itself produces (make_timestamp(-9223372036854775808)).
#include <stdio.h>
#include <string.h>
#include <duckdb.h>
int main(int argc, char **argv) {
    duckdb_database db; duckdb_connection con; duckdb_result r;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    // Get the value from SQL through a prepared statement's parameter-free path: fetch as raw timestamp.
    duckdb_query(con, "SELECT epoch_us(make_timestamp(-9223372036854775808))", &r);
    int64_t us = duckdb_value_int64(&r, 0, 0);
    printf("SQL produced TIMESTAMP with epoch_us=%lld\n", (long long)us); fflush(stdout);
    duckdb_value v = duckdb_create_timestamp((duckdb_timestamp){us});
    if (argc > 1 && strcmp(argv[1], "sql") == 0) {
        char *s = duckdb_value_to_string(v);
        printf("to_string: %s\n", s);
    } else {
        char *s = duckdb_get_varchar(v);
        printf("varchar: %s\n", s);
    }
    return 0;
}
```

Observed, identical for both functions on all three releases (exit 134):

```text
SQL produced TIMESTAMP with epoch_us=-9223372036854775808
terminate called after throwing an instance of 'duckdb::ConversionException'
  what():  {"exception_type":"Conversion","exception_message":"Date out of range in timestamp conversion"}
```

Expected: `nullptr` (or an error string), as for other failures of these
functions. quack-rs mitigation: `Value::as_str`, `display_string` and `Debug`
check every timestamp and time payload in the value (recursively through
LIST, STRUCT and MAP; an ARRAY or UNION of a temporal type is refused, since
the C API cannot read its elements) before calling either function.

---

## 14. An out-of-range TIME payload is accepted, then crashes when rendered

`duckdb_bind_time`, `duckdb_append_time` and a scalar function writing to a
TIME output vector all accept any `int64` payload. `Time::Convert`
(`time.cpp:317`) only `D_ASSERT`s the range, and the digit formatting then
indexes out of bounds: `i64::MIN` segfaults when the value is rendered (at the
append itself when the column is VARCHAR), `i64::MAX` raises "INTERNAL Error:
Information loss on integer cast", and `-1` renders as `00:00:00.00000/`. SQL
cannot build such a TIME (`make_time(1000000, 0, 0)` is "Time out of range").

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`. Run as
`repro b <micros>` (bind) or `repro a <micros>` (append).

```c
// duckdb_bind_time / duckdb_append_time with an out-of-range TIME payload.
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <duckdb.h>
int main(int argc, char **argv) {
    int64_t micros = strtoll(argv[2], NULL, 10);
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_result r;
    if (argv[1][0] == 'b') {
        duckdb_prepared_statement s;
        duckdb_prepare(con, "SELECT CAST($1 AS VARCHAR)", &s);
        duckdb_time t = {micros};
        printf("bind: %d\n", duckdb_bind_time(s, 1, t)); fflush(stdout);
        if (duckdb_execute_prepared(s, &r) == DuckDBSuccess) printf("value: %s\n", duckdb_value_varchar(&r, 0, 0));
        else printf("error: %.120s\n", duckdb_result_error(&r));
    } else {
        duckdb_query(con, "CREATE TABLE t(x TIME)", NULL);
        duckdb_appender a; duckdb_appender_create(con, NULL, "t", &a);
        duckdb_time t = {micros};
        int s1 = duckdb_append_time(a, t); int s2 = duckdb_appender_end_row(a); int s3 = duckdb_appender_close(a); printf("append: %d end_row: %d close: %d\n", s1, s2, s3); fflush(stdout);
        duckdb_appender_destroy(&a);
        if (duckdb_query(con, "SELECT x::VARCHAR FROM t", &r) == DuckDBSuccess) printf("value: %s\n", duckdb_value_varchar(&r, 0, 0));
        else printf("error: %.120s\n", duckdb_result_error(&r));
        duckdb_result r2;
        if (duckdb_query(con, "SELECT 42", &r2) == DuckDBSuccess) printf("next query ok\n");
        else printf("next query error: %.140s\n", duckdb_result_error(&r2));
    }
    return 0;
}
```

Observed with `-9223372036854775808` on all three releases: `bind: 0`, then
SIGSEGV (exit 139); `append: 0 end_row: 0 close: 0`, then SIGSEGV. The same
payload written by a scalar function into its TIME output and cast to VARCHAR
also segfaults on all three; `9223372036854775807` gives the internal error
above and `-1` the garbled text.

Expected: an error for a payload outside `0..=86400000000`. quack-rs
mitigation: `PreparedStatement::bind_time` / `bind_timestamp*` and
`Appender::append_time` / `append_timestamp` refuse out-of-range payloads;
`VectorWriter::write_time` states the range in its Safety section.

---

## 15. `duckdb_data_chunk_from_arrow` ignores the parent struct array's offset for values

The import applies the parent's `offset` to each column's validity
(`SetValidityMask(..., parent_array.offset, -1)`) but converts the values
starting at child row 0 (`ColumnArrowToDuckDB(..., 0, ...)`,
`arrow-c.cpp:118-140`). A valid struct array of 3 rows at offset 2 therefore
imports child rows 0..3 instead of 2..5, with no error, and each row's value
and validity come from different rows.

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_data_chunk_from_arrow ignores the parent struct array's offset.
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }
int main(void) {
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    int32_t data[10]; for (int i = 0; i < 10; i++) data[i] = i * 10;
    const void *cbuf[2] = {NULL, data};
    struct ArrowArray child = {10, 0, 0, 2, 0, cbuf, NULL, NULL, noop_release, NULL};
    struct ArrowArray *children[1] = {&child};
    const void *pbuf[1] = {NULL};
    struct ArrowArray parent = {3, 0, 2, 1, 1, pbuf, children, NULL, noop_release, NULL}; // offset 2, length 3
    struct ArrowSchema cs = {"i", "v", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema *cschildren[1] = {&cs};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, cschildren, NULL, noop_srelease, NULL};
    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    duckdb_data_chunk out;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&parent, conv, &out);
    printf("errors: %s / %s\n", e1 ? duckdb_error_data_message(e1) : "none", e2 ? duckdb_error_data_message(e2) : "none");
    int32_t *v = duckdb_vector_get_data(duckdb_data_chunk_get_vector(out, 0));
    printf("rows=%llu values:", (unsigned long long)duckdb_data_chunk_get_size(out));
    for (idx_t i = 0; i < duckdb_data_chunk_get_size(out); i++) printf(" %d", v[i]);
    printf("   (Arrow C Data Interface: 20 30 40)\n");
    duckdb_destroy_data_chunk(&out); duckdb_destroy_arrow_converted_schema(&conv);
    duckdb_disconnect(&con); duckdb_close(&db);
}
```

Observed, identical on all three releases:

```text
errors: none / none
rows=3 values: 0 10 20   (Arrow C Data Interface: 20 30 40)
```

Expected: `20 30 40`. quack-rs mitigation: `data_chunk_from_arrow` refuses a
nonzero top-level offset.

---

## 16. Arrow export silently corrupts large INTERVALs and UHUGEINTs of 2^127 or more

- INTERVAL: Arrow's `month_day_nano` counts nanoseconds in an `int64`, and the
  export multiplies microseconds by 1000 unchecked, so a microsecond field
  beyond ±`i64::MAX / 1000` (2,562,047 hours) wraps.
- UHUGEINT: exported (from v1.5) as a signed `decimal128(38, 0)`, so a value
  of 2^127 or more comes back negative; importing it and reading it fails
  later with "Negation of HUGEINT is out of range!". (v1.4.4 exports it as a
  16-byte fixed-size binary, which keeps the bits.)

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_data_chunk_to_arrow wraps INTERVAL micros * 1000 on export (no error).
#include <stdio.h>
#include <stdint.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct MDN { int32_t months, days; int64_t nanos; };
int main(void) {
    duckdb_database db; duckdb_connection con; duckdb_result r;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_query(con, "SELECT * FROM (VALUES (INTERVAL 2562047 HOUR), (INTERVAL 2562048 HOUR), (INTERVAL (-9223372036854775807) MICROSECOND)) t(v)", &r);
    duckdb_data_chunk chunk = duckdb_fetch_chunk(r);
    duckdb_arrow_options opts; duckdb_connection_get_arrow_options(con, &opts);
    struct ArrowArray arr = {0};
    duckdb_error_data err = duckdb_data_chunk_to_arrow(opts, chunk, (void *)&arr);
    printf("error: %s\n", err ? duckdb_error_data_message(err) : "none");
    const struct MDN *v = arr.children[0]->buffers[1];
    int64_t *src = (int64_t *)((char *)duckdb_vector_get_data(duckdb_data_chunk_get_vector(chunk, 0)) + 8);
    for (int i = 0; i < arr.length; i++)
        printf("row %d: duckdb micros=%lld  arrow nanos=%lld  (nanos/1000=%lld)\n", i, (long long)src[i*2], (long long)v[i].nanos, (long long)(v[i].nanos / 1000));
    arr.release(&arr);
    duckdb_destroy_arrow_options(&opts); duckdb_destroy_data_chunk(&chunk); duckdb_destroy_result(&r);
    duckdb_disconnect(&con); duckdb_close(&db);
}
```

```c
#include <stdio.h>
#include <stdint.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
int main(void) {
    duckdb_database db; duckdb_connection con; duckdb_result r;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_query(con, "SELECT * FROM (VALUES (340282366920938463463374607431768211455::UHUGEINT), (170141183460469231731687303715884105728::UHUGEINT), (5::UHUGEINT)) t(v)", &r);
    duckdb_data_chunk chunk = duckdb_fetch_chunk(r);
    duckdb_arrow_options opts; duckdb_connection_get_arrow_options(con, &opts);
    duckdb_logical_type t = duckdb_column_logical_type(&r, 0); const char *name = "v";
    struct ArrowSchema sch = {0};
    duckdb_error_data e1 = duckdb_to_arrow_schema(opts, &t, &name, 1, (void *)&sch);
    printf("schema err=%s format=%s\n", e1 ? duckdb_error_data_message(e1) : "none", sch.children[0]->format);
    struct ArrowArray arr = {0};
    duckdb_error_data err = duckdb_data_chunk_to_arrow(opts, chunk, (void *)&arr);
    printf("array err=%s\n", err ? duckdb_error_data_message(err) : "none");
    const int64_t *v = arr.children[0]->buffers[1];
    for (int i = 0; i < arr.length; i++) printf("row %d: arrow decimal128 words lo=%llu hi=%lld\n", i, (unsigned long long)v[2*i], (long long)v[2*i+1]);
    // import back and render
    duckdb_arrow_converted_schema conv; duckdb_error_data e2 = duckdb_schema_from_arrow(con, (void *)&sch, &conv);
    duckdb_data_chunk back; duckdb_error_data e3 = duckdb_data_chunk_from_arrow(con, (void *)&arr, conv, &back);
    printf("import errs=%s/%s\n", e2 ? duckdb_error_data_message(e2) : "none", e3 ? duckdb_error_data_message(e3) : "none");
    duckdb_query(con, "CREATE TABLE d(v DECIMAL(38,0))", NULL);
    duckdb_appender a; duckdb_appender_create(con, NULL, "d", &a);
    printf("append=%d close=%d %s\n", duckdb_append_data_chunk(a, back), duckdb_appender_close(a), duckdb_appender_error(a) ? duckdb_appender_error(a) : "");
    duckdb_appender_destroy(&a);
    duckdb_result r2; if (duckdb_query(con, "SELECT v::VARCHAR FROM d", &r2) != DuckDBSuccess) printf("select err: %s\n", duckdb_result_error(&r2));
    for (idx_t i = 0; i < duckdb_row_count(&r2); i++) printf("imported row %llu: %s\n", (unsigned long long)i, duckdb_value_varchar(&r2, 0, i));
    return 0;
}
```

Observed, INTERVAL, identical on all three releases:

```text
error: none
row 0: duckdb micros=9223369200000000  arrow nanos=9223369200000000000  (nanos/1000=9223369200000000)
row 1: duckdb micros=9223372800000000  arrow nanos=-9223371273709551616  (nanos/1000=-9223371273709551)
row 2: duckdb micros=-9223372036854775808  arrow nanos=0  (nanos/1000=0)
```

Observed, UHUGEINT, on v1.5.0 and v1.5.5 (`2^128 - 1`, `2^127`, `5`):

```text
schema err=none format=d:38,0
array err=none
row 0: arrow decimal128 words lo=18446744073709551615 hi=-1
row 1: arrow decimal128 words lo=0 hi=-9223372036854775808
row 2: arrow decimal128 words lo=5 hi=0
import errs=none/none
append=0 close=0
select err: Out of Range Error: Negation of HUGEINT is out of range!
```

Expected: an error for a value the target type cannot hold. quack-rs
mitigation: documented on `data_chunk_to_arrow` and in the book.

---

## 17. Literal types are accepted in type registration and function signatures, and their first use invalidates the database

`duckdb_register_logical_type` refuses only types containing `INVALID` or
`ANY` (`logical_types-c.cpp:399-402`), and scalar function registration
accepts any parameter or return type. `STRING_LITERAL` and `INTEGER_LITERAL`
are the binder's types for unbound literals, not column types: registered as
a type, the first INSERT raises "INTERNAL Error: Invalid PhysicalType for
GetTypeIdSize", after which **every** query on the database fails with
"FATAL Error: ... database has been invalidated". As a scalar return type the
first call does the same; as a parameter type the function can never be called
(`p_lit('abc')` raises an internal error of its own).

Environment: prebuilt `libduckdb` v1.4.4, v1.5.0 and v1.5.5. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_register_logical_type accepts STRING_LITERAL; first use invalidates the database.
#include <stdio.h>
#include <duckdb.h>
static void q(duckdb_connection con, const char *sql) {
    duckdb_result r;
    if (duckdb_query(con, sql, &r) == DuckDBSuccess) printf("%-40s ok\n", sql);
    else printf("%-40s ERROR %.110s\n", sql, duckdb_result_error(&r));
    duckdb_destroy_result(&r);
}
int main(void) {
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_logical_type t = duckdb_create_logical_type(DUCKDB_TYPE_STRING_LITERAL);
    duckdb_logical_type_set_alias(t, "lit_t");
    printf("register: %d\n", duckdb_register_logical_type(con, t, NULL));
    q(con, "CREATE TABLE tt(x lit_t)");
    q(con, "INSERT INTO tt VALUES (NULL)");
    q(con, "SELECT x::VARCHAR FROM tt");
    q(con, "SELECT 42");
    q(con, "CREATE TABLE other(i INTEGER)");
    duckdb_destroy_logical_type(&t);
    duckdb_disconnect(&con); duckdb_close(&db);
}
```

Observed, identical on all three releases (stack traces omitted):

```text
register: 0
CREATE TABLE tt(x lit_t)                 ok
INSERT INTO tt VALUES (NULL)             ERROR INTERNAL Error: Invalid PhysicalType for GetTypeIdSize
SELECT x::VARCHAR FROM tt                ERROR INTERNAL Error: Invalid PhysicalType for GetTypeIdSize
SELECT 42                                ERROR FATAL Error: Failed: database has been invalidated because of a previous fatal error. ...
CREATE TABLE other(i INTEGER)            ERROR FATAL Error: Failed: database has been invalidated because of a previous fatal error. ...
```

The same types in a function signature (v1.4.4 and v1.5.5; `p_lit` takes the
literal type, `r_lit` returns it):

```c
// Does any function slot work with STRING_LITERAL / INTEGER_LITERAL?
#include <stdio.h>
#include <duckdb.h>
static void q(duckdb_connection con, const char *sql) {
    duckdb_result r;
    if (duckdb_query(con, sql, &r) == DuckDBSuccess) printf("  %-44s ok rows=%llu\n", sql, (unsigned long long)duckdb_row_count(&r));
    else printf("  %-44s ERROR %.90s\n", sql, duckdb_result_error(&r));
    duckdb_destroy_result(&r);
}
static void fn(duckdb_function_info info, duckdb_data_chunk in, duckdb_vector out) {
    int64_t *o = duckdb_vector_get_data(out); for (idx_t i = 0; i < duckdb_data_chunk_get_size(in); i++) o[i] = 7;
}
static void fn2(duckdb_function_info info, duckdb_data_chunk in, duckdb_vector out) {
    for (idx_t i = 0; i < duckdb_data_chunk_get_size(in); i++) duckdb_vector_assign_string_element(out, i, "x");
}
int main(void) {
    duckdb_type lits[2] = {DUCKDB_TYPE_INTEGER_LITERAL, DUCKDB_TYPE_STRING_LITERAL};
    const char *names[2] = {"INTEGER_LITERAL", "STRING_LITERAL"};
    for (int k = 0; k < 2; k++) {
        printf("== %s\n", names[k]);
        duckdb_database db; duckdb_connection con;
        duckdb_open(NULL, &db); duckdb_connect(db, &con);
        duckdb_logical_type lit = duckdb_create_logical_type(lits[k]);
        duckdb_logical_type big = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
        duckdb_logical_type vc = duckdb_create_logical_type(DUCKDB_TYPE_VARCHAR);
        printf("  type id round trip: %d\n", (int)duckdb_get_type_id(lit));
        duckdb_scalar_function f = duckdb_create_scalar_function();
        duckdb_scalar_function_set_name(f, "p_lit");
        duckdb_scalar_function_add_parameter(f, lit);
        duckdb_scalar_function_set_return_type(f, big);
        duckdb_scalar_function_set_function(f, fn);
        printf("  register param: %d\n", duckdb_register_scalar_function(con, f));
        duckdb_destroy_scalar_function(&f);
        f = duckdb_create_scalar_function();
        duckdb_scalar_function_set_name(f, "r_lit");
        duckdb_scalar_function_add_parameter(f, big);
        duckdb_scalar_function_set_return_type(f, lit);
        duckdb_scalar_function_set_function(f, fn2);
        printf("  register return: %d\n", duckdb_register_scalar_function(con, f));
        duckdb_destroy_scalar_function(&f);
        q(con, "SELECT p_lit(42)");
        q(con, "SELECT p_lit('abc')");
        q(con, "SELECT p_lit(x) FROM range(3) t(x)");
        q(con, "SELECT r_lit(1)");
        q(con, "SELECT r_lit(x) FROM range(3) t(x)");
        q(con, "SELECT 42");
        duckdb_destroy_logical_type(&lit); duckdb_destroy_logical_type(&big); duckdb_destroy_logical_type(&vc);
        duckdb_disconnect(&con); duckdb_close(&db);
    }
}
```

Observed, identical for both literal types on both releases (stack traces and
long messages cut):

```text
  register param: 0
  register return: 0
  SELECT p_lit(42)                             ERROR Binder Error: No function matches the given name and argument types 'p_lit(INTEGER_LITERAL...
  SELECT p_lit('abc')                          ERROR INTERNAL Error: Function p_lit returned a STRING_LITERAL or INTEGER_LITERAL type - return ...
  SELECT p_lit(x) FROM range(3) t(x)           ERROR Binder Error: No function matches the given name and argument types 'p_lit(BIGINT)'...
  SELECT r_lit(1)                              ERROR INTERNAL Error: Invalid PhysicalType for GetTypeIdSize
  SELECT r_lit(x) FROM range(3) t(x)           ERROR INTERNAL Error: Unsupported type INVALID for ColumnDataCollection::GetCopyFunction
  SELECT 42                                    ERROR FATAL Error: Failed: database has been invalidated because of a previous fatal error. ...
```

Expected: registration refuses both literal types. quack-rs mitigation:
`LogicalType::try_new` (and so every builder slot) refuses them.

---

## 18. A zero-size ARRAY or a UNION with no members: created by release builds, refused by debug builds

SQL rejects both ("ARRAY type size must be at least 1"; `UNION()` does not
parse). `duckdb_create_array_type(t, 0)` and `duckdb_create_union_type(..., 0)`
(`logical_types-c.cpp:63`, `:79`) return a working handle from a release
build — registered, such a type can hold only NULL — while a build with
assertions returns null for the array (and UBSan reports a reference bound to
null in the reproducer, which then uses it). Behaviour depends on the build.

Environment: prebuilt `libduckdb` v1.4.4 and v1.5.5; v1.5.5 built from source
with assertions and ASan. Build: `gcc -I<libduckdb dir> repro.c -L<libduckdb dir> -lduckdb -o repro`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
#include <stdio.h>
#include <duckdb.h>
static void q(duckdb_connection con, const char *sql) {
    duckdb_result r;
    if (duckdb_query(con, sql, &r) == DuckDBSuccess) printf("  %-40s ok\n", sql);
    else printf("  %-40s ERROR %.100s\n", sql, duckdb_result_error(&r));
    duckdb_destroy_result(&r);
}
int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_logical_type i = duckdb_create_logical_type(DUCKDB_TYPE_INTEGER);
    duckdb_logical_type a0 = duckdb_create_array_type(i, 0);
    printf("array0 handle %p id %d\n", (void*)a0, a0 ? (int)duckdb_get_type_id(a0) : -1);
    duckdb_logical_type_set_alias(a0, "a0_t");
    printf("register a0: %d\n", duckdb_register_logical_type(con, a0, NULL));
    q(con, "CREATE TABLE ta(x a0_t)");
    q(con, "INSERT INTO ta VALUES (NULL)");
    q(con, "SELECT * FROM ta");
    q(con, "SELECT NULL::a0_t");
    q(con, "SELECT [1]::a0_t");
    duckdb_logical_type mt[1] = {i}; const char *mn[1] = {"m"};
    duckdb_logical_type u0 = duckdb_create_union_type(mt, mn, 0);
    printf("union0 handle %p id %d\n", (void*)u0, u0 ? (int)duckdb_get_type_id(u0) : -1);
    if (u0) {
    duckdb_logical_type_set_alias(u0, "u0_t");
    printf("register u0: %d\n", duckdb_register_logical_type(con, u0, NULL));
    q(con, "CREATE TABLE tu(x u0_t)");
    q(con, "INSERT INTO tu VALUES (NULL)");
    q(con, "SELECT * FROM tu");
    q(con, "SELECT NULL::u0_t");
    }
    duckdb_value v0 = duckdb_create_array_value(i, (duckdb_value[1]){0}, 0);
    printf("array_value(empty) %p\n", (void*)v0);
    if (v0) { char *s = duckdb_value_to_string(v0); printf("  renders %s\n", s ? s : "(null)"); duckdb_free(s); duckdb_destroy_value(&v0); }
    q(con, "SELECT 42");
    duckdb_destroy_logical_type(&a0); if (u0) duckdb_destroy_logical_type(&u0); duckdb_destroy_logical_type(&i);
    duckdb_disconnect(&con); duckdb_close(&db);
}
```

Observed on the release builds (v1.4.4 and v1.5.5 identical, handles
abbreviated):

```text
array0 handle 0x... id 33
register a0: 0
  CREATE TABLE ta(x a0_t)                  ok
  INSERT INTO ta VALUES (NULL)             ok
  SELECT * FROM ta                         ok
  SELECT NULL::a0_t                        ok
  SELECT [1]::a0_t                         ERROR Conversion Error: Cannot cast list with length 1 to array with length 0
union0 handle 0x... id 28
register u0: 0
  ...                                      ok
array_value(empty) 0x...
  renders []
  SELECT 42                                ok
```

On the assertion build: `array0 handle (nil) id -1`. Expected: the release
build refuses both, as SQL does. quack-rs mitigation: `try_array` with size 0,
an empty `union_type` and an empty `Value::array_value` are refused.

---

## 19. `epoch_us(interval)` fails when one field overflows although the total fits (minor)

`Interval::GetMicro` converts months, then days, to microseconds one at a time
in `int64` and throws when one of them overflows on its own, so
`epoch_us(to_days(106751992) + to_microseconds(-86400000000))`, whose total
9223372022400000000 fits, fails. Observed with the v1.4.4 and v1.5.5 CLIs:

```text
Conversion Error: Could not convert Day to Microseconds
```

Expected: the total, when it fits. quack-rs's `interval_to_micros` computes
the exact total in `i128` and returns it; its documentation notes where it and
`epoch_us` differ.

---

## 20. Grouped-aggregate states that a stopped scan never reached are never destroyed

A grouped aggregate's states live in the radix-partitioned hash table and are
destroyed as the result scan passes them. When the scan stops early (a
`LIMIT` above the aggregate, an error raised above it, an interrupt), the
states it had not reached are never destroyed: not when the query ends, and
not when the connection or the database closes. Any memory a state owns leaks.
This affects DuckDB's own aggregates too: `mode()` under `LIMIT 10` leaks
about 100 MB per query in the program below.

Where, from a source reading of v1.5.5 (not confirmed in a debugger):
`RadixHTGlobalSinkState::Destroy` (`radix_partitioned_hashtable.cpp`, from
line 245) returns at once while `scan_pin_properties` is
`DESTROY_AFTER_DONE`, its initial value (line 218), relying on the scan to
destroy each state after reading it (line 938). A scan that stops early
leaves the rest.

```c
/* Grouped-aggregate states the scan never reaches are never destroyed. */
#include <duckdb.h>
#include <malloc.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static long inits, destroys;
static idx_t sz(duckdb_function_info i) { (void)i; return sizeof(long); }
static void init(duckdb_function_info i, duckdb_aggregate_state s) { (void)i; *(long *)s = 0; __atomic_add_fetch(&inits, 1, __ATOMIC_SEQ_CST); }
static void update(duckdb_function_info i, duckdb_data_chunk c, duckdb_aggregate_state *s) {
	(void)i; idx_t n = duckdb_data_chunk_get_size(c);
	for (idx_t r = 0; r < n; r++) *(long *)s[r] += 1;
}
static void combine(duckdb_function_info i, duckdb_aggregate_state *src, duckdb_aggregate_state *dst, idx_t n) {
	(void)i; for (idx_t r = 0; r < n; r++) *(long *)dst[r] += *(long *)src[r];
}
static void finalize(duckdb_function_info i, duckdb_aggregate_state *s, duckdb_vector out, idx_t n, idx_t off) {
	(void)i; long *d = duckdb_vector_get_data(out);
	for (idx_t r = 0; r < n; r++) d[off + r] = *(long *)s[r];
}
static void destroy(duckdb_aggregate_state *s, idx_t n) { (void)s; __atomic_add_fetch(&destroys, (long)n, __ATOMIC_SEQ_CST); }

static void run(duckdb_connection con, const char *label, const char *sql) {
	long i0 = inits, d0 = destroys;
	duckdb_result res;
	duckdb_state st = duckdb_query(con, sql, &res);
	printf("%-44s %s  init %ld  destroy %ld\n", label, st == DuckDBSuccess ? "ok   " : "error", inits - i0, destroys - d0);
	duckdb_destroy_result(&res);
}

static void heap(duckdb_connection con, const char *label, const char *sql) {
	duckdb_result res;
	duckdb_query(con, sql, &res); duckdb_destroy_result(&res); /* warm-up */
	malloc_trim(0);
	size_t before = mallinfo2().uordblks;
	for (int k = 0; k < 10; k++) { duckdb_query(con, sql, &res); duckdb_destroy_result(&res); }
	malloc_trim(0);
	printf("%-44s heap in use grew by %zu bytes over 10 runs\n", label, mallinfo2().uordblks - before);
}

int main(void) {
	duckdb_database db; duckdb_connection con;
	duckdb_open(NULL, &db); duckdb_connect(db, &con);
	duckdb_aggregate_function f = duckdb_create_aggregate_function();
	duckdb_aggregate_function_set_name(f, "counted");
	duckdb_logical_type big = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
	duckdb_aggregate_function_add_parameter(f, big);
	duckdb_aggregate_function_set_return_type(f, big);
	duckdb_aggregate_function_set_functions(f, sz, init, update, combine, finalize);
	duckdb_aggregate_function_set_destructor(f, destroy);
	if (duckdb_register_aggregate_function(con, f) != DuckDBSuccess) { puts("register failed"); return 1; }
	duckdb_query(con, "SET threads = 1", NULL);
	const char *inner = "(SELECT i % 300000 AS g, counted(i) AS s FROM range(1000000) t(i) GROUP BY g)";
	char sql[512];
	snprintf(sql, sizeof sql, "SELECT count(s) FROM %s", inner);
	run(con, "whole result", sql);
	snprintf(sql, sizeof sql, "SELECT count(s) FROM (SELECT s FROM %s LIMIT 10)", inner);
	run(con, "LIMIT 10 above the aggregate", sql);
	snprintf(sql, sizeof sql, "SELECT count(CASE WHEN s > -1 AND g = 150000 THEN error('stop') END) FROM %s", inner);
	run(con, "error raised above the aggregate", sql);
	const char *m = "(SELECT i % 300000 AS g, mode(i) AS m FROM range(1000000) t(i) GROUP BY g)";
	snprintf(sql, sizeof sql, "SELECT count(m) FROM %s", m);
	heap(con, "built-in mode(), whole result", sql);
	snprintf(sql, sizeof sql, "SELECT count(m) FROM (SELECT m FROM %s LIMIT 10)", m);
	heap(con, "built-in mode(), LIMIT 10", sql);
	duckdb_destroy_logical_type(&big); duckdb_destroy_aggregate_function(&f);
	duckdb_disconnect(&con); duckdb_close(&db);
	return 0;
}
```

Built with `gcc -O1 -Wall -I<dir> abandoned_states.c -L<dir> -lduckdb` against
each prebuilt library. v1.4.4:

```text
whole result                                 ok     init 300000  destroy 300000
LIMIT 10 above the aggregate                 ok     init 300000  destroy 4096
error raised above the aggregate             error  init 300000  destroy 151552
built-in mode(), whole result                heap in use grew by 547312 bytes over 10 runs
built-in mode(), LIMIT 10                    heap in use grew by 993475152 bytes over 10 runs
```

v1.4.5 printed the same destroy counts as v1.4.4. v1.5.0 to v1.5.5 printed
the same destroy counts as each other; their `mode()` figures differed by at
most 16,400 bytes in these runs, and vary from run to run. v1.5.5:

```text
whole result                                 ok     init 300000  destroy 300000
LIMIT 10 above the aggregate                 ok     init 300000  destroy 2048
error raised above the aggregate             error  init 300000  destroy 151552
built-in mode(), whole result                heap in use grew by 584400 bytes over 10 runs
built-in mode(), LIMIT 10                    heap in use grew by 1001047984 bytes over 10 runs
```

Expected: `destroy` equal to `init` in every row, and no heap growth for
`mode()` beyond the whole-result case. quack-rs cannot destroy states DuckDB
abandons; `FfiState<T>` stores a `T` of at most 256 bytes, aligned no more
strictly than `usize`, inside the state bytes (which DuckDB frees with the
hash table) instead of boxing it, so only what such a `T` owns leaks. Pinned
by `states_a_grouped_scan_never_reaches_leak_no_rust_heap` in
`tests/aggregate_leaks.rs`.

---

## 21. Arrow import reads one byte past a validity bitmap whose bit offset is not a multiple of 8

`GetValidityMask` (`src/function/table/arrow_conversion.cpp`, from line 47 in
v1.5.5) copies `ceil(size / 8) + 1` bytes from the first byte it needs when the
effective bit offset is not a multiple of 8 (line 66). The rows need only
`ceil((bit_offset % 8 + size) / 8)` bytes, so when those fit, the copy reads
one byte past them. A producer whose bitmap is exactly as long as its rows
need (the Arrow format recommends, but does not require, padding buffers to 8
or 64 bytes) is read out of bounds. The extra byte only supplies bits for rows
past the end, so the values imported are right.

```c
// duckdb_data_chunk_from_arrow reads one byte past a validity bitmap whose
// bit offset is not a multiple of 8.
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }
int main(void) {
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    int32_t data[8]; for (int i = 0; i < 8; i++) data[i] = i * 10;
    uint8_t *validity = malloc(1);   // bits 1..7 cover rows 0..6: one byte is enough
    *validity = 0xFF & ~(1u << 3);   // row 2 (bit 3) is NULL
    const void *cbuf[2] = {validity, data};
    struct ArrowArray child = {7, 1, 1, 2, 0, cbuf, NULL, NULL, noop_release, NULL}; // offset 1, length 7
    struct ArrowArray *children[1] = {&child};
    const void *pbuf[1] = {NULL};
    struct ArrowArray parent = {7, 0, 0, 1, 1, pbuf, children, NULL, noop_release, NULL};
    struct ArrowSchema cs = {"i", "v", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema *cschildren[1] = {&cs};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, cschildren, NULL, noop_srelease, NULL};
    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    duckdb_data_chunk out;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&parent, conv, &out);
    printf("errors: %s / %s\n", e1 ? duckdb_error_data_message(e1) : "none", e2 ? duckdb_error_data_message(e2) : "none");
    duckdb_vector vec = duckdb_data_chunk_get_vector(out, 0);
    int32_t *v = duckdb_vector_get_data(vec);
    uint64_t *valid = duckdb_vector_get_validity(vec);
    printf("rows=%llu values:", (unsigned long long)duckdb_data_chunk_get_size(out));
    for (idx_t i = 0; i < duckdb_data_chunk_get_size(out); i++) {
        if (valid && !duckdb_validity_row_is_valid(valid, i)) printf(" NULL"); else printf(" %d", v[i]);
    }
    printf("\n");
    duckdb_destroy_data_chunk(&out); duckdb_destroy_arrow_converted_schema(&conv);
    free(validity);
    duckdb_disconnect(&con); duckdb_close(&db);
}
```

Built with `gcc -O0 -g -Wall -I<dir> validity_overread.c -L<dir> -lduckdb`
against each prebuilt library and run under valgrind. v1.5.5 (the process id
replaced by `PID`):

```text
==PID== Invalid read of size 2
==PID==    at 0x4852EB0: memmove (in /usr/libexec/valgrind/vgpreload_memcheck-amd64-linux.so)
==PID==    by 0x5B6AA6F: duckdb::GetValidityMask(duckdb::ValidityMask&, ArrowArray&, unsigned long, unsigned long, long, long, bool) (in /opt/duckdb/1.5.5/libduckdb.so)
==PID==    by 0x6483244: duckdb_data_chunk_from_arrow (in /opt/duckdb/1.5.5/libduckdb.so)
==PID==    by 0x10968E: main (validity_overread.c:30)
==PID==  Address 0xb6eb230 is 0 bytes inside a block of size 1 alloc'd
==PID==    at 0x4846828: malloc (in /usr/libexec/valgrind/vgpreload_memcheck-amd64-linux.so)
==PID==    by 0x10944D: main (validity_overread.c:17)
errors: none / none
rows=7 values: 10 20 NULL 40 50 60 70
```

v1.4.4 and v1.5.0 report the same invalid read from `GetValidityMask`, and
print the same two lines. Expected: no read past byte 0 of the bitmap.
quack-rs cannot see a buffer's allocated size through the C Data Interface;
`data_chunk_from_arrow`'s Safety section requires every bitmap `DuckDB` reads
to be readable for one byte past the last byte its rows occupy.
