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
| 16 | Arrow export corrupts large INTERVAL and UHUGEINT values, and exports 39-digit HUGEINTs as `decimal128(38, 0)` | C program, below | such values refused by `data_chunk_to_arrow` |
| 17 | Literal types accepted; first use invalidates the database | C program, below | refused by `LogicalType::try_new` |
| 18 | Zero-size ARRAY / empty UNION depend on the build | C program, below | refused |
| 19 | `epoch_us(interval)` fails when one field overflows | SQL, below | exact total computed in `i128` |
| 20 | Grouped-aggregate states a stopped scan never reached are never destroyed | C program, below | small states stored inline; documented |
| 21 | Arrow import reads one byte past a validity bitmap at an unaligned bit offset | C program, below (valgrind) | Safety clause on `data_chunk_from_arrow` |
| 22 | `duckdb_get_varchar` / `duckdb_value_to_string` fail on DECIMAL, VARIANT and GEOMETRY values SQL builds | C program, below | render guard is an allow-list; DECIMAL checked against its width; VARIANT and GEOMETRY refused |
| 23 | `duckdb_list_vector_reserve` aborts below `MAX_VECTOR_SIZE` elements when the child buffer passes 2^37 bytes | C program, below | `ListBuilder` limit: the largest power of two whose bytes fit |
| 24 | Arrow import applies offsets below the top level wrongly (struct fields, union members, run-end arrays) | C program, below | offsets below the top level, and run-end arrays under an offset, refused |
| 25 | Arrow dictionary import reads validity without the list's start offset, and copies an enclosing struct's NULLs into a 2048-row mask past a heap buffer | C program, below (valgrind) | such dictionaries refused before import |
| 26 | Arrow import of a dictionary whose values are dictionary-encoded shares one dictionary cache and returns garbage | C program, below | nested dictionaries refused |
| 27 | Arrow list-view import scans `sum(sizes)` child rows from the lowest offset, reading past the child | C program, below (valgrind) | overlapping or gapped list views refused |
| 28 | Arrow sparse-union import uses the type codes as member indices, ignoring the `+us:` code list | C program, below | non-identity union type codes refused |
| 29 | Arrow dictionary import points NULL indices one past a zero-copied values buffer; copying the vector reads past it | C program, below (valgrind) | Safety clause on `data_chunk_from_arrow` |
| 30 | Table-description column accessors abort on index `idx_t(-1)` from 1.5.0 | C program, below | that index refused before the call |
| 31 | Arrow dictionary with `null_count = -1` imports its NULL rows as values | C program, below | refused |
| 32 | Arrow sparse union with nonzero `null_count` has its type ids read as a validity bitmap | C program, below | refused |
| 33 | Arrow import of a `geoarrow.wkb` column of more than 2048 rows writes past a vector | C program, below | refused past 2048 rows |
| 34 | Arrow export declares plain binary for BIGNUM/GEOMETRY but writes binary views under `arrow_output_version = '1.4'` | C program, below | such an export refused |
| 35 | Window frames with `EXCLUDE` never destroy the aggregate states of their second segment-tree part | C program, below | documented; small states stored inline |

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

HUGEINT has the same problem unless `arrow_lossless_conversion` is set:
`DuckDB` exports it as `decimal128(38, 0)`, whose values have at most 38
digits, even when it holds 39 (`i128::MIN` and `i128::MAX` do). The bits
survive, but the array violates its declared precision, and importing it back
gives a `DECIMAL(38, 0)` wider than its type (item 22).

```c
// duckdb_data_chunk_to_arrow exports a 39-digit HUGEINT as decimal128(38, 0).
// Usage: hugeint_export [lossless]
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    duckdb_database db; duckdb_connection con; duckdb_result r;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);
    if (argc > 1 && strcmp(argv[1], "lossless") == 0) duckdb_query(con, "SET arrow_lossless_conversion = true", NULL);
    duckdb_query(con, "SELECT * FROM (VALUES ('-170141183460469231731687303715884105728'::HUGEINT), ('170141183460469231731687303715884105727'::HUGEINT), ('99999999999999999999999999999999999999'::HUGEINT)) t(v)", &r);
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
    for (int i = 0; i < arr.length; i++) {
        __int128 x = ((__int128)v[2*i+1] << 64) | (unsigned __int128)(uint64_t)v[2*i];
        unsigned __int128 m = x < 0 ? -(unsigned __int128)x : (unsigned __int128)x; int digits = 0;
        do { digits++; m /= 10; } while (m);
        printf("row %d: stored value has %d digits\n", i, digits);
    }
    return 0;
}
```

Observed on v1.4.4, v1.5.0 and v1.5.5, identical (without, then with,
`lossless`):

```text
schema err=none format=d:38,0
array err=none
row 0: stored value has 39 digits
row 1: stored value has 39 digits
row 2: stored value has 38 digits
```

```text
schema err=none format=w:16
array err=none
row 0: stored value has 39 digits
row 1: stored value has 39 digits
row 2: stored value has 38 digits
```

Expected: an error for a value the target type cannot hold. quack-rs
mitigation: `data_chunk_to_arrow` checks the chunk first and refuses an
`INTERVAL` whose nanoseconds overflow, and a `HUGEINT` or `UHUGEINT` with more
than 38 digits when its format is a decimal, at any nesting depth
(`src/arrow/export_check.rs`, `tests/ffi_roundtrip/arrow_export.rs`).

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

---

## 22. `duckdb_get_varchar` and `duckdb_value_to_string` fail on DECIMAL, VARIANT and GEOMETRY values that SQL builds

Item 13 covers out-of-range TIMESTAMP payloads. The same two functions, which
render with no `try` (`src/main/capi/duckdb_value-c.cpp:309` and `:621` in
v1.5.5, `:301` and `:613` in v1.4.4), also fail on these values, each built
by plain SQL:

1. **`DECIMAL(38,0)` holding `i128::MIN`.** `sum` over `DECIMAL(38, s)`
   returns `DECIMAL(38, s)`
   (`extension/core_functions/aggregate/distributive/sum.cpp:210`) and checks
   only for `HUGEINT` overflow, so the sum of
   `-99999999999999999999999999999999999999` and
   `-70141183460469231731687303715884105729` is `i128::MIN`, 39 digits.
   `DecimalToString::DecimalLength` negates it (`cast_helpers.cpp:217`; the
   `D_ASSERT` above it is compiled out of release builds) and
   `Hugeint::NegateInPlace` throws "Negation of HUGEINT is out of range!"
   (`hugeint.hpp:53`).
2. **`DECIMAL(38,38)` holding 1.2**, the `sum` of `0.6` and `0.6`. When
   `width == scale`, `DecimalLength` reserves one character for the integer
   part (`cast_helpers.cpp:233`: 40 characters here) but `FormatDecimal`
   writes no integer digit (`cast_helpers.cpp:308`), so the first character
   of the text is never written and keeps whatever the string heap held
   before. When that byte is not valid UTF-8, the `Value` constructor throws
   "Invalid unicode (byte sequence mismatch) detected in value construction"
   (`src/common/types/value.cpp:161-163`); otherwise the function returns
   the text with a stray first character, or with a NUL there, which a C
   caller reads as an empty string. Which of the three happens varied by
   release and by call in the runs below. valgrind (memcheck) reported no
   error for this case on v1.4.4 or v1.5.5; the prebuilt library allocates
   through its bundled jemalloc (`duckdb_je_*` symbols), which memcheck does
   not track, so it cannot see that the byte was never written.
3. **VARIANT holding an out-of-range TIMESTAMP**,
   `make_timestamp(-9223372036854775808)::VARIANT`. The cast to VARCHAR
   (`CastFromVARIANT`, `src/function/cast/variant/from_variant.cpp:743`)
   renders the timestamp and `Timestamp::Convert` throws "Date out of range
   in timestamp conversion" (`timestamp.cpp:413`), as in item 13. SQL
   accepts `::VARIANT` from v1.4.4, but `duckdb.h` has no
   `DUCKDB_TYPE_VARIANT` before v1.5.3, where the value's type id reads as
   0 (`DUCKDB_TYPE_INVALID`). The C API cannot read what a VARIANT holds, so
   a caller cannot check the payload first.
4. **GEOMETRY from malformed WKB.** `ST_GeomFromWKB` is built in from v1.5.0
   (v1.4.x reports that it exists in the spatial extension) and accepts a
   MULTIPOINT whose part is a LINESTRING. `Geometry::ToString` throws
   "Expected POINT in MULTIPOINT but got 2" when it renders it
   (`src/common/types/geometry.cpp:702`). `DUCKDB_TYPE_GEOMETRY` is in
   `duckdb.h` from v1.5.2; on v1.5.0 and v1.5.1 the type id reads as 0.

In each case `SELECT CAST(x AS VARCHAR)` in SQL reports the same exception as
an ordinary error (or, for case 2, returns the same text); only the C API lets
it escape. The program reaches the value the way an extension does: a table
function with an `ANY` parameter reads it with `duckdb_bind_get_parameter` and
renders it inside its bind callback, and `main` renders a second copy after
the query has returned. Inside the callback, the exception unwinds through
the C frames (skipping the rest of the callback, which then leaks `v`) and
DuckDB reports it as the query's error; an extension whose callback cannot be
unwound through, such as one written in Rust, aborts there instead. Outside
any DuckDB frame, in `main`, the process terminates.

```c
// duckdb_get_varchar / duckdb_value_to_string on values that plain SQL builds,
// reached the way an extension reaches them: a table function taking ANY
// renders its bind parameter (duckdb_bind_get_parameter) inside its bind
// callback, and main() renders a second copy of it after the query returns.
// Usage: render dec_min|dec_scale|variant|geometry [sql]
#include <stdio.h>
#include <string.h>
#include <duckdb.h>

static int use_to_string;
static duckdb_value kept; // a second copy of the parameter, rendered again from main()

static void render(const char *where, duckdb_value v) {
    duckdb_logical_type t = duckdb_get_value_type(v);
    printf("%s: type id %d; calling %s\n", where, (int)duckdb_get_type_id(t),
           use_to_string ? "duckdb_value_to_string" : "duckdb_get_varchar");
    char *s = use_to_string ? duckdb_value_to_string(v) : duckdb_get_varchar(v);
    if (s) {
        printf("%s: strlen %zu, text \"%s\"\n", where, strlen(s), s);
        duckdb_free(s);
    } else {
        printf("%s: NULL\n", where);
    }
}

static void bind(duckdb_bind_info info) {
    kept = duckdb_bind_get_parameter(info, 0);
    duckdb_value v = duckdb_bind_get_parameter(info, 0);
    render("bind", v);
    duckdb_destroy_value(&v);
    duckdb_logical_type big = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
    duckdb_bind_add_result_column(info, "n", big);
    duckdb_destroy_logical_type(&big);
}
static void init(duckdb_init_info info) { (void)info; }
static void scan(duckdb_function_info info, duckdb_data_chunk out) { (void)info; duckdb_data_chunk_set_size(out, 0); }

static void run(duckdb_connection con, const char *sql) {
    duckdb_result r;
    if (duckdb_query(con, sql, &r) == DuckDBSuccess) {
        char *s = duckdb_value_varchar(&r, 0, 0);
        printf("%s\n  -> %s\n", sql, s ? s : "(no value)");
        duckdb_free(s);
    } else {
        printf("%s\n  -> error: %s\n", sql, duckdb_result_error(&r));
    }
    duckdb_destroy_result(&r);
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    const char *mode = argc > 1 ? argv[1] : "dec_min";
    use_to_string = argc > 2 && strcmp(argv[2], "sql") == 0;
    duckdb_database db; duckdb_connection con;
    duckdb_open(NULL, &db); duckdb_connect(db, &con);

    duckdb_table_function f = duckdb_create_table_function();
    duckdb_table_function_set_name(f, "render_probe");
    duckdb_logical_type any = duckdb_create_logical_type(DUCKDB_TYPE_ANY);
    duckdb_table_function_add_parameter(f, any);
    duckdb_table_function_set_bind(f, bind);
    duckdb_table_function_set_init(f, init);
    duckdb_table_function_set_function(f, scan);
    duckdb_register_table_function(con, f);

    const char *value;
    if (strcmp(mode, "dec_min") == 0) {
        // sum() of two in-range DECIMAL(38,0) values: i128::MIN, 39 digits.
        run(con, "SET VARIABLE v = (SELECT sum(x) FROM (VALUES "
                 "((-99999999999999999999999999999999999999)::DECIMAL(38,0)), "
                 "((-70141183460469231731687303715884105729)::DECIMAL(38,0))) t(x))");
        value = "getvariable('v')";
    } else if (strcmp(mode, "dec_scale") == 0) {
        // sum() of 0.6 + 0.6 in DECIMAL(38,38): 1.2, which needs 39 digits.
        run(con, "SET VARIABLE v = (SELECT sum(x) FROM (VALUES "
                 "('0.6'::DECIMAL(38,38)), ('0.6'::DECIMAL(38,38))) t(x))");
        value = "getvariable('v')";
    } else if (strcmp(mode, "variant") == 0) {
        value = "make_timestamp(-9223372036854775808)::VARIANT";
    } else {
        // WKB MULTIPOINT (type 4) with one part whose type is LINESTRING (2), 0 points.
        value = "ST_GeomFromWKB('\\x01\\x04\\x00\\x00\\x00\\x01\\x00\\x00\\x00"
                "\\x01\\x02\\x00\\x00\\x00\\x00\\x00\\x00\\x00'::BLOB)";
    }
    char sql[512];
    snprintf(sql, sizeof sql, "SELECT typeof(%s)", value);
    run(con, sql);
    snprintf(sql, sizeof sql, "SELECT CAST(%s AS VARCHAR)", value);
    run(con, sql);
    snprintf(sql, sizeof sql, "SELECT count(*) FROM render_probe(%s)", value);
    run(con, sql);
    if (kept) {
        render("main", kept);
        duckdb_destroy_value(&kept);
    }

    duckdb_destroy_logical_type(&any);
    duckdb_destroy_table_function(&f);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Built with `gcc -O1 -Wall -I<dir> render.c -L<dir> -lduckdb` against each
prebuilt library from v1.4.4 to v1.5.5 (v1.4.4, v1.4.5, v1.5.0 to v1.5.5), and
run as `render <case>` (`duckdb_get_varchar`) and `render <case> sql`
(`duckdb_value_to_string`).

Case 1, `render dec_min` on v1.5.5 (exit 134):

```text
SET VARIABLE v = (SELECT sum(x) FROM (VALUES ((-99999999999999999999999999999999999999)::DECIMAL(38,0)), ((-70141183460469231731687303715884105729)::DECIMAL(38,0))) t(x))
  -> (no value)
SELECT typeof(getvariable('v'))
  -> DECIMAL(38,0)
SELECT CAST(getvariable('v') AS VARCHAR)
  -> error: Out of Range Error: Negation of HUGEINT is out of range!
bind: type id 19; calling duckdb_get_varchar
SELECT count(*) FROM render_probe(getvariable('v'))
  -> error: Out of Range Error: Negation of HUGEINT is out of range!

LINE 1: SELECT count(*) FROM render_probe(getvariable('v'))
                             ^
main: type id 19; calling duckdb_get_varchar
terminate called after throwing an instance of 'duckdb::OutOfRangeException'
  what():  {"exception_type":"Out of Range","exception_message":"Negation of HUGEINT is out of range!"}
```

All eight releases printed exactly this (compared with `diff`), and
`render dec_min sql` printed the same with `duckdb_value_to_string` in place
of `duckdb_get_varchar`, also exiting with 134 on all eight.

Case 2, `render dec_scale` on v1.4.4 (exit 0):

```text
SET VARIABLE v = (SELECT sum(x) FROM (VALUES ('0.6'::DECIMAL(38,38)), ('0.6'::DECIMAL(38,38))) t(x))
  -> (no value)
SELECT typeof(getvariable('v'))
  -> DECIMAL(38,38)
SELECT CAST(getvariable('v') AS VARCHAR)
  -> D.20000000000000000000000000000000000000
bind: type id 19; calling duckdb_get_varchar
bind: strlen 40, text "D.20000000000000000000000000000000000000"
SELECT count(*) FROM render_probe(getvariable('v'))
  -> 0
main: type id 19; calling duckdb_get_varchar
main: strlen 40, text "8.20000000000000000000000000000000000000"
```

and on v1.5.4 (exit 134):

```text
SET VARIABLE v = (SELECT sum(x) FROM (VALUES ('0.6'::DECIMAL(38,38)), ('0.6'::DECIMAL(38,38))) t(x))
  -> (no value)
SELECT typeof(getvariable('v'))
  -> DECIMAL(38,38)
SELECT CAST(getvariable('v') AS VARCHAR)
  -> D.20000000000000000000000000000000000000
bind: type id 19; calling duckdb_get_varchar
bind: strlen 40, text "D.20000000000000000000000000000000000000"
SELECT count(*) FROM render_probe(getvariable('v'))
  -> 0
main: type id 19; calling duckdb_get_varchar
terminate called after throwing an instance of 'duckdb::InvalidInputException'
  what():  {"exception_type":"Invalid Input","exception_message":"Invalid unicode (byte sequence mismatch) detected in value construction"}
```

Over five runs of `render dec_scale` per release: v1.4.4 printed the output
above every time, and v1.5.5 the same except that `main` read
`x.20000000000000000000000000000000000000`; v1.4.5, v1.5.3 and v1.5.4 printed
the v1.5.4 output above every time; on v1.5.0, v1.5.1 and v1.5.2 SQL's own
cast and the bind-time rendering came back with a NUL first byte
(`bind: strlen 0, text ""`), and the rendering in `main` terminated as above
in 13 of the 15 runs (in two of the v1.5.1 runs the bind-time rendering threw
too, and the query reported it as its error). Under valgrind, v1.4.4 and v1.5.5 printed the same lines as without
it and reported `ERROR SUMMARY: 0 errors from 0 contexts`.

Case 3, `render variant` on v1.5.5 (exit 134):

```text
SELECT typeof(make_timestamp(-9223372036854775808)::VARIANT)
  -> VARIANT
SELECT CAST(make_timestamp(-9223372036854775808)::VARIANT AS VARCHAR)
  -> error: Conversion Error: Date out of range in timestamp conversion
bind: type id 41; calling duckdb_get_varchar
SELECT count(*) FROM render_probe(make_timestamp(-9223372036854775808)::VARIANT)
  -> error: Conversion Error: Date out of range in timestamp conversion

LINE 1: SELECT count(*) FROM render_probe(make_timestamp(-9223372036854775808)::VARIANT)
                             ^
main: type id 41; calling duckdb_get_varchar
terminate called after throwing an instance of 'duckdb::ConversionException'
  what():  {"exception_type":"Conversion","exception_message":"Date out of range in timestamp conversion"}
```

All eight releases printed the same apart from `type id 0` before v1.5.3
(compared with `diff` after replacing the type id). `render variant sql`
printed the same with `duckdb_value_to_string` on v1.5.0 to v1.5.5. On v1.4.4
and v1.4.5 `duckdb_value_to_string` does not throw: it renders the VARIANT's
internal STRUCT (exit 0):

```text
bind: strlen 125, text "{'keys': [], 'children': [], 'values': [{'type_id': 24, 'byte_offset': 0}], 'data': '\x00\x00\x00\x00\x00\x00\x00\x80'::BLOB}"
```

Case 4, `render geometry` on v1.5.5 (exit 134):

```text
SELECT typeof(ST_GeomFromWKB('\x01\x04\x00\x00\x00\x01\x00\x00\x00\x01\x02\x00\x00\x00\x00\x00\x00\x00'::BLOB))
  -> GEOMETRY
SELECT CAST(ST_GeomFromWKB('\x01\x04\x00\x00\x00\x01\x00\x00\x00\x01\x02\x00\x00\x00\x00\x00\x00\x00'::BLOB) AS VARCHAR)
  -> error: Invalid Input Error: Expected POINT in MULTIPOINT but got 2
bind: type id 40; calling duckdb_get_varchar
SELECT count(*) FROM render_probe(ST_GeomFromWKB('\x01\x04\x00\x00\x00\x01\x00\x00\x00\x01\x02\x00\x00\x00\x00\x00\x00\x00'::BLOB))
  -> error: Invalid Input Error: Expected POINT in MULTIPOINT but got 2

LINE 1: SELECT count(*) FROM render_probe(ST_GeomFromWKB('\x01\x04\x00\x00\x00\x01\x00...
                             ^
main: type id 40; calling duckdb_get_varchar
terminate called after throwing an instance of 'duckdb::InvalidInputException'
  what():  {"exception_type":"Invalid Input","exception_message":"Expected POINT in MULTIPOINT but got 2"}
```

v1.5.0 to v1.5.4 printed the same apart from `type id 0` on v1.5.0 and
v1.5.1, and `render geometry sql` the same with `duckdb_value_to_string`
(compared with `diff`); every one exited with 134. v1.4.4 and v1.4.5 have no
`ST_GeomFromWKB` without the spatial extension.

Expected: `nullptr` (or an error string) from both functions for cases 1, 3
and 4, as for other failures of these functions, and for case 2 either the
text `1.20000000000000000000000000000000000000` or an error, never an
unwritten byte. quack-rs mitigation: the render guard behind `Value::as_str`,
`display_string` and `Debug` is an allow-list. A value renders unchecked only
if every payload of its type renders; TIMESTAMP and TIME payloads are checked
against the range DuckDB renders (item 13), and a DECIMAL payload against its
width, so cases 1 and 2 are refused. VARIANT and GEOMETRY are always refused,
as is a type the crate does not know (VARIANT before v1.5.3, whose type id
reads as invalid); LIST, STRUCT and MAP are checked element by element, and an
ARRAY or UNION whose type contains anything that needs a check is refused,
since the C API cannot read its elements.

---

## 23. `duckdb_list_vector_reserve` aborts below `MAX_VECTOR_SIZE` elements when the child buffer passes 2^37 bytes

Item 1 covers a capacity above `MAX_VECTOR_SIZE` (2^37) elements, which
`VectorListBuffer::Reserve` refuses. A smaller capacity can still throw
through the same uncaught call (`src/main/capi/data_chunk-c.cpp:207-213`).
`VectorListBuffer::Reserve` rounds the capacity up to a power of two
(`src/common/types/vector_buffer.cpp:79`) and calls `Vector::Resize`
(`src/common/types/vector.cpp:399`), which for every buffer of the child
(the child's own, each STRUCT field's, and an ARRAY child's, whose multiplier
is the array size) computes `new_size * GetTypeIdSize(type) * multiplier`
bytes and throws `OutOfRangeException` when that exceeds
`DConstants::MAX_VECTOR_SIZE`, the same constant 2^37 now read as bytes
(`vector.cpp:424`). The limit in elements therefore depends on the child
type: 2^33 for a 16-byte HUGEINT, and for `INTEGER[1000]` (4000 bytes per
element) 2^25, since a capacity of 2^25 + 1 is rounded up to 2^26, although
2^25 + 1 elements would take only 134,217,732,000 bytes, less than 2^37.
v1.4.4 has the same code (`Vector::Resize` identical; the C API call at
`data_chunk-c.cpp:182`).

For each buffer the check comes after the validity mask is resized (line 412)
and before the data allocation (line 432). The validity resize allocates only
when the mask has already been materialised, which it has not for a new
vector, so the program below allocates nothing large. Buffers are processed
in order, though, so for a STRUCT child whose narrower field comes first,
that field's buffer is allocated at the rounded capacity before the wider
field's check throws (source reading; not run, since it would allocate at
least 16 GiB).

```c
// duckdb_list_vector_reserve lets a C++ exception escape for a capacity below
// MAX_VECTOR_SIZE (2^37) elements whose child buffer would exceed 2^37 bytes.
// Usage: reserve_bytes hugeint|array
//   hugeint: LIST(HUGEINT), 16 bytes per element, capacity 2^33 + 1
//   array:   LIST(INTEGER[1000]), 4000 bytes per element, capacity 2^25 + 1
#include <stdio.h>
#include <string.h>
#include <duckdb.h>
int main(int argc, char **argv) {
    int array = argc > 1 && strcmp(argv[1], "array") == 0;
    duckdb_logical_type elem = duckdb_create_logical_type(array ? DUCKDB_TYPE_INTEGER : DUCKDB_TYPE_HUGEINT);
    duckdb_logical_type child = array ? duckdb_create_array_type(elem, 1000) : elem;
    duckdb_logical_type list = duckdb_create_list_type(child);
    duckdb_vector vec = duckdb_create_vector(list, 1);
    idx_t bytes_per_element = array ? 4000 : 16;
    idx_t capacity = array ? ((idx_t)1 << 25) + 1 : ((idx_t)1 << 33) + 1;
    fprintf(stderr, "calling duckdb_list_vector_reserve(%llu): %llu elements, %llu bytes; ceiling 2^37 = %llu bytes\n",
            (unsigned long long)capacity, (unsigned long long)capacity,
            (unsigned long long)(capacity * bytes_per_element), (unsigned long long)((idx_t)1 << 37));
    duckdb_state st = duckdb_list_vector_reserve(vec, capacity);
    fprintf(stderr, "returned %s\n", st == DuckDBSuccess ? "DuckDBSuccess" : "DuckDBError");
    duckdb_destroy_vector(&vec);
    duckdb_destroy_logical_type(&list);
    if (array) duckdb_destroy_logical_type(&child);
    duckdb_destroy_logical_type(&elem);
    return 0;
}
```

Built with `gcc -O1 -Wall -I<dir> reserve_bytes.c -L<dir> -lduckdb` against
each prebuilt library and run with the address space limited to 4 GiB
(`ulimit -v 4194304`). v1.4.4, `reserve_bytes hugeint` (exit 134):

```text
calling duckdb_list_vector_reserve(8589934593): 8589934593 elements, 137438953488 bytes; ceiling 2^37 = 137438953472 bytes
terminate called after throwing an instance of 'duckdb::OutOfRangeException'
  what():  {"exception_type":"Out of Range","exception_message":"Cannot resize vector to 256.0 GiB: maximum allowed vector size is 128.0 GiB"}
```

and `reserve_bytes array` (exit 134):

```text
calling duckdb_list_vector_reserve(33554433): 33554433 elements, 134217732000 bytes; ceiling 2^37 = 137438953472 bytes
terminate called after throwing an instance of 'duckdb::OutOfRangeException'
  what():  {"exception_type":"Out of Range","exception_message":"Cannot resize vector to 250.0 GiB: maximum allowed vector size is 128.0 GiB"}
```

v1.4.5 and v1.5.0 to v1.5.5 printed exactly the same for both (compared with
`md5sum`), each exiting with 134. There is no control run under the ceiling:
the largest reservation that passes the check (2^33 HUGEINTs, or 2^25
`INTEGER[1000]`s) allocates 128 GiB or 125 GiB.

Expected: `DuckDBError`, as in item 1; the fix there (a `try` around the call)
covers this too. quack-rs mitigation: `ListBuilder` limits a list vector's
child to `max_child_capacity`, computed from the widest buffer the child
type grows (`resize_bytes_per_element`, which follows STRUCT fields and
UNION members and multiplies through ARRAY sizes), and writes a row that
would pass it as NULL. The limit allows for the rounding: it is the
largest power of two whose elements fit, 2^25 for `INTEGER[1000]`. (The
fifth audit's first version divided 2^37 by the element size, 34,359,738
for `INTEGER[1000]`, so the reservation in the program above still aborted;
`tests/ffi_roundtrip/list_limits.rs` now covers it.)

---

## 24. `duckdb_data_chunk_from_arrow` applies offsets below the top level to the wrong rows

DuckDB's conversion (`ArrowToDuckDBConversion`, `arrow_conversion.cpp`, the
same in v1.4.4 and v1.5.5 where cited) locates each node's rows from two
parameters its caller passes: a `parent_offset` and a `nested_offset`.
`GetEffectiveOffset` (`arrow_conversion.cpp:29`) returns `array.offset +
nested_offset` inside a `LIST` and `array.offset + parent_offset + chunk_offset`
otherwise. Several node kinds pass the wrong value to their children, so a
valid array other producers make imports rows from the wrong place with no
error:

- **A `STRUCT` passes only its own `offset` to its children.** In the `STRUCT`
  case (`arrow_conversion.cpp:1155`) each field is converted with
  `parent_offset = array.offset` (this struct's own offset), not the offset
  inherited from above (`SetValidityMask(child_entry, child_array, chunk_offset,
  size, array.offset, nested_offset)`, ~`:1165`). So a struct with a nonzero
  offset *inside a struct* drops the outer struct's offset, and a struct with a
  nonzero offset *inside a list* drops the struct's own offset (the
  `nested_offset` path at `arrow_conversion.cpp:33` ignores `parent_offset`).
- **A sparse `UNION`'s members are read from the wrong row.** The type ids are
  read at the effective offset, but each member is converted without the
  union's own `offset` (`arrow_conversion.cpp:1199`; members dispatched at
  ~`:1222`-`1231` with `parent_offset` unset), so the type ids and the member
  values come from different rows.
- **A run-end-encoded array reads its values' validity from the logical
  offset.** `ColumnArrowToDuckDBRunEndEncoded` sets the *values'* validity mask
  with `parent_offset`/`nested_offset` (`SetValidityMask(values, values_array,
  chunk_offset, compressed_size, parent_offset, nested_offset)`,
  `arrow_conversion.cpp:723`), although the run values are indexed by run, not
  by logical row, so a REE under a list or an offset struct reads their
  validity from the wrong bits.
- **A run-end-encoded array under a fixed-size list is read as a plain array.**
  `ArrowToDuckDBArray` (`arrow_conversion.cpp:257`) dispatches only on
  dictionary-encoded and default children (~`:302`-`309`), so a
  `RUN_END_ENCODED` child falls through to `ColumnArrowToDuckDB` and is read as
  a plain array from buffers it does not have (the same happens for a REE inside
  another encoded array's values).

The line numbers in v1.4.4 are `29`, `257`, `672`, `1076`, `1120`; the code is
otherwise identical.

Built with `gcc -O1 -g -Wall -I/opt/duckdb/<ver> item24.c -L/opt/duckdb/<ver> -lduckdb -Wl,-rpath,/opt/duckdb/<ver>` against each prebuilt library, and run under valgrind 3.22 for the `fixedlist_ree` crash.
```c
// duckdb_data_chunk_from_arrow applies offsets below the top level to the
// wrong rows. One program, one mode argument:
//   struct_struct  a struct at offset 2 inside a struct (outer offset dropped)
//   list_struct    a struct at offset 3 inside a list (struct offset dropped)
//   union          a sparse union at offset 1 (members / type ids off)
//   list_ree       a run-end-encoded array under a list (values' validity off)
//   struct_ree     the same under a struct at offset 2
//   fixedlist_ree  a run-end-encoded array under a fixed-size list (read plain)
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }

static duckdb_connection con;

// Import record-batch struct `parent` described by `ps`, put the chunk in *out.
static int import(struct ArrowArray *parent, struct ArrowSchema *ps, duckdb_data_chunk *out) {
    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return 1; }
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)parent, conv, out);
    if (e2) { printf("from_arrow: %s\n", duckdb_error_data_message(e2)); duckdb_destroy_arrow_converted_schema(&conv); return 1; }
    duckdb_destroy_arrow_converted_schema(&conv);
    return 0;
}

static void print_int(duckdb_vector v, idx_t n) {
    int32_t *d = duckdb_vector_get_data(v);
    uint64_t *val = duckdb_vector_get_validity(v);
    for (idx_t i = 0; i < n; i++) {
        if (i) printf(" / ");
        if (val && !duckdb_validity_row_is_valid(val, i)) printf("NULL");
        else printf("%d", d[i]);
    }
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc < 2) { printf("usage: %s <mode>\n", argv[0]); return 2; }
    const char *mode = argv[1];
    duckdb_database db; duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_data_chunk out = NULL;

    if (!strcmp(mode, "struct_struct")) {
        // outer STRUCT(offset 2) { inner STRUCT { a INT } }; 3 rows.
        int32_t data[5]; for (int i = 0; i < 5; i++) data[i] = i * 10; // 0 10 20 30 40
        const void *ibuf[2] = {NULL, data};
        struct ArrowArray ci = {5, 0, 0, 2, 0, ibuf, NULL, NULL, noop_release, NULL};
        struct ArrowArray *inner_ch[1] = {&ci};
        const void *nb[1] = {NULL};
        struct ArrowArray inner = {5, 0, 0, 1, 1, nb, inner_ch, NULL, noop_release, NULL};
        struct ArrowArray *outer_ch[1] = {&inner};
        const void *nb2[1] = {NULL};
        struct ArrowArray outer = {3, 0, 2, 1, 1, nb2, outer_ch, NULL, noop_release, NULL}; // offset 2
        struct ArrowArray *cols[1] = {&outer};
        const void *nb3[1] = {NULL};
        struct ArrowArray rb = {3, 0, 0, 1, 1, nb3, cols, NULL, noop_release, NULL};
        struct ArrowSchema si = {"i", "a", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema *sinner_ch[1] = {&si};
        struct ArrowSchema s_inner = {"+s", "t", NULL, 0, 1, sinner_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *souter_ch[1] = {&s_inner};
        struct ArrowSchema s_outer = {"+s", "s", NULL, 0, 1, souter_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *scols[1] = {&s_outer};
        struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
        if (import(&rb, &ps, &out)) return 1;
        duckdb_vector ov = duckdb_data_chunk_get_vector(out, 0);
        duckdb_vector iv = duckdb_struct_vector_get_child(ov, 0);
        duckdb_vector av = duckdb_struct_vector_get_child(iv, 0);
        printf("struct_struct: "); print_int(av, duckdb_data_chunk_get_size(out)); printf("\n");
    } else if (!strcmp(mode, "list_struct")) {
        // LIST< STRUCT(offset 3){ a INT } >, 2 list rows, offsets [0,2,4].
        int32_t data[10]; for (int i = 0; i < 10; i++) data[i] = i * 10;
        const void *ibuf[2] = {NULL, data};
        struct ArrowArray ci = {10, 0, 0, 2, 0, ibuf, NULL, NULL, noop_release, NULL};
        struct ArrowArray *st_ch[1] = {&ci};
        const void *nb[1] = {NULL};
        struct ArrowArray st = {7, 0, 3, 1, 1, nb, st_ch, NULL, noop_release, NULL}; // struct offset 3
        struct ArrowArray *list_ch[1] = {&st};
        int32_t offs[3] = {0, 2, 4};
        const void *lbuf[2] = {NULL, offs};
        struct ArrowArray list = {2, 0, 0, 2, 1, lbuf, list_ch, NULL, noop_release, NULL};
        struct ArrowArray *cols[1] = {&list};
        const void *nb3[1] = {NULL};
        struct ArrowArray rb = {2, 0, 0, 1, 1, nb3, cols, NULL, noop_release, NULL};
        struct ArrowSchema si = {"i", "a", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema *sst_ch[1] = {&si};
        struct ArrowSchema s_st = {"+s", "item", NULL, 0, 1, sst_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *sl_ch[1] = {&s_st};
        struct ArrowSchema s_list = {"+l", "l", NULL, 0, 1, sl_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *scols[1] = {&s_list};
        struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
        if (import(&rb, &ps, &out)) return 1;
        duckdb_vector lv = duckdb_data_chunk_get_vector(out, 0);
        duckdb_list_entry *ent = duckdb_vector_get_data(lv);
        duckdb_vector sv = duckdb_list_vector_get_child(lv);
        duckdb_vector av = duckdb_struct_vector_get_child(sv, 0);
        int32_t *d = duckdb_vector_get_data(av);
        idx_t n = duckdb_data_chunk_get_size(out);
        printf("list_struct: ");
        for (idx_t r = 0; r < n; r++) {
            if (r) printf(" / ");
            printf("[");
            for (idx_t k = 0; k < ent[r].length; k++) printf("%s{'a':%d}", k ? "," : "", d[ent[r].offset + k]);
            printf("]");
        }
        printf("\n");
    } else if (!strcmp(mode, "union")) {
        // sparse UNION +us:0,1 at offset 1, 3 rows. members m0=100+i, m1=10*i.
        int32_t m0[5] = {100, 101, 102, 103, 104};
        int32_t m1[5] = {0, 10, 20, 30, 40};
        const void *b0[2] = {NULL, m0};
        const void *b1[2] = {NULL, m1};
        struct ArrowArray a0 = {5, 0, 0, 2, 0, b0, NULL, NULL, noop_release, NULL};
        struct ArrowArray a1 = {5, 0, 0, 2, 0, b1, NULL, NULL, noop_release, NULL};
        struct ArrowArray *members[2] = {&a0, &a1};
        int8_t tids[5] = {1, 0, 1, 0, 0};
        const void *ubuf[1] = {tids};
        struct ArrowArray uni = {3, 0, 1, 1, 2, ubuf, members, NULL, noop_release, NULL}; // offset 1
        struct ArrowArray *cols[1] = {&uni};
        const void *nb3[1] = {NULL};
        struct ArrowArray rb = {3, 0, 0, 1, 1, nb3, cols, NULL, noop_release, NULL};
        struct ArrowSchema s0 = {"i", "a", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema s1 = {"i", "b", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema *um[2] = {&s0, &s1};
        struct ArrowSchema s_uni = {"+us:0,1", "u", NULL, 0, 2, um, NULL, noop_srelease, NULL};
        struct ArrowSchema *scols[1] = {&s_uni};
        struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
        if (import(&rb, &ps, &out)) return 1;
        duckdb_vector uv = duckdb_data_chunk_get_vector(out, 0);
        duckdb_vector tagv = duckdb_struct_vector_get_child(uv, 0);
        uint8_t *tag = duckdb_vector_get_data(tagv);
        duckdb_vector mv0 = duckdb_struct_vector_get_child(uv, 1);
        duckdb_vector mv1 = duckdb_struct_vector_get_child(uv, 2);
        int32_t *dd0 = duckdb_vector_get_data(mv0);
        int32_t *dd1 = duckdb_vector_get_data(mv1);
        idx_t n = duckdb_data_chunk_get_size(out);
        printf("union: ");
        for (idx_t i = 0; i < n; i++) { if (i) printf(" / "); printf("%d", tag[i] == 0 ? dd0[i] : dd1[i]); }
        printf("\n");
    } else if (!strcmp(mode, "list_ree") || !strcmp(mode, "struct_ree")) {
        // REE: run_ends=[2,4,5] int32, values=[NULL,20,30]. Logical rows
        // 0,1=NULL 2,3=20 4=30. Seen (list offsets [2,4]) / (struct offset 2)
        // as logical rows 2,3 -> value 20,20.
        int32_t rends[3] = {2, 4, 5};
        const void *rbuf[2] = {NULL, rends};
        struct ArrowArray ra = {3, 0, 0, 2, 0, rbuf, NULL, NULL, noop_release, NULL};
        int32_t vals[3] = {10, 20, 30};
        // Runs 0..2 are all valid (bits 0,1,2 = 1). Bit 3 is 0: DuckDB copies
        // this mask from the logical offset (bit 2 for both modes below), so
        // run 1's validity is read from bit 3 (NULL) instead of bit 1 (valid).
        uint8_t vvalid = 0xF7; // 1111 0111 : only bit 3 is 0
        const void *vbuf[2] = {&vvalid, vals};
        struct ArrowArray va = {3, 1, 0, 2, 0, vbuf, NULL, NULL, noop_release, NULL};
        struct ArrowArray *ree_ch[2] = {&ra, &va};
        struct ArrowArray ree = {5, 0, 0, 0, 2, NULL, ree_ch, NULL, noop_release, NULL};
        struct ArrowSchema s_re = {"i", "run_ends", NULL, 0, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema s_va = {"i", "values", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema *sree_ch[2] = {&s_re, &s_va};
        struct ArrowSchema s_ree = {"+r", "ree", NULL, 0, 2, sree_ch, NULL, noop_srelease, NULL};
        if (!strcmp(mode, "list_ree")) {
            struct ArrowArray *list_ch[1] = {&ree};
            int32_t offs[2] = {2, 4};
            const void *lbuf[2] = {NULL, offs};
            struct ArrowArray list = {1, 0, 0, 2, 1, lbuf, list_ch, NULL, noop_release, NULL};
            struct ArrowArray *cols[1] = {&list};
            const void *nb3[1] = {NULL};
            struct ArrowArray rb = {1, 0, 0, 1, 1, nb3, cols, NULL, noop_release, NULL};
            struct ArrowSchema *sl_ch[1] = {&s_ree};
            struct ArrowSchema s_list = {"+l", "l", NULL, 0, 1, sl_ch, NULL, noop_srelease, NULL};
            struct ArrowSchema *scols[1] = {&s_list};
            struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
            if (import(&rb, &ps, &out)) return 1;
            duckdb_vector lv = duckdb_data_chunk_get_vector(out, 0);
            duckdb_list_entry *ent = duckdb_vector_get_data(lv);
            duckdb_vector cv = duckdb_list_vector_get_child(lv);
            int32_t *d = duckdb_vector_get_data(cv);
            uint64_t *val = duckdb_vector_get_validity(cv);
            printf("list_ree: [");
            for (idx_t k = 0; k < ent[0].length; k++) {
                if (k) printf(", ");
                idx_t j = ent[0].offset + k;
                if (val && !duckdb_validity_row_is_valid(val, j)) printf("NULL"); else printf("%d", d[j]);
            }
            printf("]\n");
        } else {
            struct ArrowArray *st_ch[1] = {&ree};
            const void *nb[1] = {NULL};
            struct ArrowArray st = {2, 0, 2, 1, 1, nb, st_ch, NULL, noop_release, NULL}; // struct offset 2
            struct ArrowArray *cols[1] = {&st};
            const void *nb3[1] = {NULL};
            struct ArrowArray rb = {2, 0, 0, 1, 1, nb3, cols, NULL, noop_release, NULL};
            struct ArrowSchema *sst_ch[1] = {&s_ree};
            struct ArrowSchema s_st = {"+s", "r", NULL, 0, 1, sst_ch, NULL, noop_srelease, NULL};
            struct ArrowSchema *scols[1] = {&s_st};
            struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
            if (import(&rb, &ps, &out)) return 1;
            duckdb_vector sv = duckdb_data_chunk_get_vector(out, 0);
            duckdb_vector rv = duckdb_struct_vector_get_child(sv, 0);
            int32_t *d = duckdb_vector_get_data(rv);
            uint64_t *val = duckdb_vector_get_validity(rv);
            idx_t n = duckdb_data_chunk_get_size(out);
            printf("struct_ree: ");
            for (idx_t i = 0; i < n; i++) {
                if (i) printf(" / ");
                if (val && !duckdb_validity_row_is_valid(val, i)) printf("{'r': NULL}"); else printf("{'r': %d}", d[i]);
            }
            printf("\n");
        }
    } else if (!strcmp(mode, "fixedlist_ree")) {
        // FIXED-SIZE LIST INT[2] whose child is run-end encoded (run_ends=[2],
        // values=[NULL]); 1 row -> logical [NULL, NULL]. DuckDB reads the REE
        // child as a plain INT array from buffers it does not have.
        int32_t rends[1] = {2};
        const void *rbuf[2] = {NULL, rends};
        struct ArrowArray ra = {1, 0, 0, 2, 0, rbuf, NULL, NULL, noop_release, NULL};
        int32_t vals[1] = {0};
        uint8_t vvalid = 0x00; // values[0] NULL
        const void *vbuf[2] = {&vvalid, vals};
        struct ArrowArray va = {1, 1, 0, 2, 0, vbuf, NULL, NULL, noop_release, NULL};
        struct ArrowArray *ree_ch[2] = {&ra, &va};
        struct ArrowArray ree = {2, 0, 0, 0, 2, NULL, ree_ch, NULL, noop_release, NULL};
        struct ArrowArray *fl_ch[1] = {&ree};
        const void *nb[1] = {NULL};
        struct ArrowArray fl = {1, 0, 0, 1, 1, nb, fl_ch, NULL, noop_release, NULL};
        struct ArrowArray *cols[1] = {&fl};
        const void *nb3[1] = {NULL};
        struct ArrowArray rb = {1, 0, 0, 1, 1, nb3, cols, NULL, noop_release, NULL};
        struct ArrowSchema s_re = {"i", "run_ends", NULL, 0, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema s_va = {"i", "values", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema *sree_ch[2] = {&s_re, &s_va};
        struct ArrowSchema s_ree = {"+r", "item", NULL, 0, 2, sree_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *sfl_ch[1] = {&s_ree};
        struct ArrowSchema s_fl = {"+w:2", "fl", NULL, 0, 1, sfl_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *scols[1] = {&s_fl};
        struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
        if (import(&rb, &ps, &out)) return 1;
        printf("fixedlist_ree: imported ok, reading child...\n");
        duckdb_vector av = duckdb_data_chunk_get_vector(out, 0);
        duckdb_vector cv = duckdb_array_vector_get_child(av);
        int32_t *d = duckdb_vector_get_data(cv);
        uint64_t *val = duckdb_vector_get_validity(cv);
        printf("fixedlist_ree: [");
        for (idx_t k = 0; k < 2; k++) {
            if (k) printf(", ");
            if (val && !duckdb_validity_row_is_valid(val, k)) printf("NULL"); else printf("%d", d[k]);
        }
        printf("]\n");
    } else {
        printf("unknown mode %s\n", mode); return 2;
    }
    if (out) duckdb_destroy_data_chunk(&out);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed. `struct_struct` (a struct at offset 2 inside a struct), identical on
all eight releases v1.4.4, v1.4.5 and v1.5.0-v1.5.5:
```text
struct_struct: 0 / 10 / 20
```

`list_struct` (a struct with offset 3 inside a list), identical on all eight:
```text
list_struct: [{'a':0},{'a':10}] / [{'a':20},{'a':30}]
```

`union` (a sparse union with offset 1). Two behaviours, both wrong. On v1.4.4,
v1.5.0 and v1.5.1 the type ids are read from row 0 as well:
```text
union: 0 / 101 / 20
```

On v1.4.5, v1.5.2, v1.5.3, v1.5.4 and v1.5.5 the type ids get the offset but the members do not:
```text
union: 100 / 10 / 102
```

`list_ree` (a run-end-encoded array whose runs are all valid, seen through a
list as logical rows 2..4) and `struct_ree` (the same under a struct at offset
2), each identical on all eight -- the values are right but their validity is
read from the wrong run, so every row reads NULL:
```text
list_ree: [NULL, NULL]
```
```text
struct_ree: {'r': NULL} / {'r': NULL}
```

`fixedlist_ree` (a fixed-size list `INT[2]` whose child is run-end encoded)
crashes during the import itself on all eight (`SIGSEGV`, exit 139, no output),
because the run-end-encoded child is read as a plain `INT` array from the REE
array's absent data buffer. Under valgrind the read is byte-for-byte identical
on all eight releases (only the library path in each frame differs):
```text
==PID== Invalid read of size 8
==PID==    at 0x…: duckdb::DirectConversion(duckdb::Vector&, ArrowArray&, unsigned long, long, unsigned long) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb_data_chunk_from_arrow (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: import (item24.c:28)
==PID==    by 0x…: main (item24.c:243)
==PID==  Address 0x… is not stack'd, malloc'd or (recently) free'd
==PID== 
```

Expected: `20 / 30 / 40`; `[{'a':30},{'a':40}] / [{'a':50},{'a':60}]`;
`101 / 20 / 103`; `[20, 20]`; `{'r': 20}` twice; and, for `fixedlist_ree`,
`[NULL, NULL]` read back without a crash. quack-rs mitigation:
`data_chunk_from_arrow` walks the array beside its schema and refuses a struct
or union with an offset below the top level, a run-end-encoded array read under
an offset, and a run-end-encoded array in a place DuckDB reads as a plain array
(a fixed-size list's child or another encoded array's values);
`src/arrow/import_layout.rs`, exercised end-to-end by
`tests/ffi_roundtrip/arrow_layout.rs`.

---
## 25. Arrow dictionary import reads validity without the list's offset, and overflows a 2048-row mask from an enclosing struct's NULLs

`ColumnArrowToDuckDBDictionary` (`arrow_conversion.cpp:1391`; `:1310` in v1.4.4)
reads a dictionary-encoded array's index validity with
`GetValidityMask(indices_validity, array, chunk_offset, size, parent_offset)`
(~`:1440`; `:1354` in v1.4.4) -- using `parent_offset` but not the list's
`nested_offset` (the `LIST` path passes `start_offset` as the dictionary's
`nested_offset` but the validity read ignores it; "TODO: add support for
offsets", `arrow_conversion.cpp:238`). Two consequences, both without an error:

- **Under a list that starts past row 0, the indices are read from the list's
  offset but their validity from row 0**, so rows come back NULL that are not.
- **The `indices_validity` mask is default-constructed and sized for
  `STANDARD_VECTOR_SIZE` (2048) rows** by `EnsureWritable`. When the dictionary
  is converted as more than 2048 rows and NULLs must be applied -- whether the
  array's own or, via the `parent_mask` loop, an enclosing struct's NULL rows
  (`CanContainNull`, `arrow_conversion.cpp:1381`) -- `SetInvalid(i)` for
  `i > 2048` reads and writes past the end of that heap allocation. Item 9
  covers the dictionary's *own* NULLs (its `GetValidityMask` `memcpy`); this
  extends it to NULLs *inherited from an enclosing struct*, which reach the
  overflow through the `parent_mask` loop instead.

A dictionary column imports as a DuckDB dictionary vector the flat C API cannot
resolve (item 8), so `list_dict` appends the imported chunk to a table and
reads it back through a result, which materialises the logical rows.

Built with `gcc -O1 -g -Wall -I/opt/duckdb/<ver> item25.c -L/opt/duckdb/<ver> -lduckdb -Wl,-rpath,/opt/duckdb/<ver>` against each prebuilt library, and run under valgrind 3.22 for the overflow.
```c
// duckdb_data_chunk_from_arrow, dictionary import. Two modes:
//   list_dict            a dictionary under a list starting past row 0: the
//                        index validity is read from row 0, not the list's
//                        offset, so rows come back NULL that are not.
//   struct_dict_big <n>  a dictionary with no NULLs of its own under a struct
//                        whose every third row is NULL, n (default 4096) rows:
//                        the enclosing struct's NULLs are written into a
//                        2048-row index-validity mask, past its heap block.
// A dictionary column imports as a DuckDB dictionary vector the flat C API
// cannot resolve (item 8), so list_dict appends the imported chunk to a table
// and reads it back through a result, which materialises the logical rows.
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }

static duckdb_connection con;
static int import(struct ArrowArray *parent, struct ArrowSchema *ps, duckdb_data_chunk *out) {
    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return 1; }
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)parent, conv, out);
    if (e2) { printf("from_arrow: %s\n", duckdb_error_data_message(e2)); duckdb_destroy_arrow_converted_schema(&conv); return 2; }
    duckdb_destroy_arrow_converted_schema(&conv);
    return 0;
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc < 2) { printf("usage: %s <mode>\n", argv[0]); return 2; }
    const char *mode = argv[1];
    duckdb_database db; duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_data_chunk out = NULL;

    if (!strcmp(mode, "list_dict")) {
        int32_t dvals[6] = {100, 200, 300, 400, 500, 600};
        const void *dbuf[2] = {NULL, dvals};
        struct ArrowArray dict = {6, 0, 0, 2, 0, dbuf, NULL, NULL, noop_release, NULL};
        int32_t idx[6] = {0, 1, 2, 3, 4, 5};
        uint8_t ivalid = 0xFC; // 1111 1100 : child rows 0,1 NULL, 2..5 valid
        const void *ibuf[2] = {&ivalid, idx};
        struct ArrowArray indices = {6, 2, 0, 2, 0, ibuf, NULL, &dict, noop_release, NULL};
        struct ArrowArray *list_ch[1] = {&indices};
        int32_t offs[3] = {2, 4, 6}; // list starts at child 2
        const void *lbuf[2] = {NULL, offs};
        struct ArrowArray list = {2, 0, 0, 2, 1, lbuf, list_ch, NULL, noop_release, NULL};
        struct ArrowArray *cols[1] = {&list};
        const void *nb[1] = {NULL};
        struct ArrowArray rb = {2, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};
        struct ArrowSchema s_dict = {"i", "item", NULL, 0, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema s_idx = {"i", "item", NULL, 2, 0, NULL, &s_dict, noop_srelease, NULL};
        struct ArrowSchema *sl_ch[1] = {&s_idx};
        struct ArrowSchema s_list = {"+l", "l", NULL, 0, 1, sl_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *scols[1] = {&s_list};
        struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
        if (import(&rb, &ps, &out)) return 1;
        // Materialise: the imported LIST<dict> child is a dictionary vector; the
        // flat C API returns its unresolved base buffer and a NULL validity
        // pointer (item 8). Append to a table and read it back as a result.
        duckdb_result r0; duckdb_query(con, "CREATE TABLE t(l INTEGER[])", &r0); duckdb_destroy_result(&r0);
        duckdb_appender ap; duckdb_appender_create(con, NULL, "t", &ap);
        duckdb_state st = duckdb_append_data_chunk(ap, out);
        if (st != DuckDBSuccess) { printf("append: %s\n", duckdb_appender_error(ap)); return 1; }
        duckdb_appender_close(ap); duckdb_appender_destroy(&ap);
        duckdb_result res; duckdb_query(con, "SELECT l::VARCHAR FROM t", &res);
        idx_t n = duckdb_row_count(&res);
        printf("list_dict: ");
        for (idx_t i = 0; i < n; i++) {
            char *s = duckdb_value_varchar(&res, 0, i);
            printf("%s%s", i ? " / " : "", s ? s : "NULL");
            duckdb_free(s);
        }
        printf("\n");
        duckdb_destroy_result(&res);
    } else if (!strcmp(mode, "struct_dict_big")) {
        int64_t rows = argc >= 3 ? strtoll(argv[2], NULL, 10) : 4096;
        int32_t dvals[1] = {42};
        const void *dbuf[2] = {NULL, dvals};
        struct ArrowArray dict = {1, 0, 0, 2, 0, dbuf, NULL, NULL, noop_release, NULL};
        int32_t *idx = calloc((size_t)rows, sizeof(int32_t)); // all index 0
        const void *ibuf[2] = {NULL, idx};                    // no own validity
        struct ArrowArray indices = {rows, 0, 0, 2, 0, ibuf, NULL, &dict, noop_release, NULL};
        struct ArrowArray *st_ch[1] = {&indices};
        size_t vbytes = (size_t)((rows + 7) / 8);
        uint8_t *svalid = malloc(vbytes);
        memset(svalid, 0xFF, vbytes);
        int64_t nulls = 0;
        for (int64_t i = 0; i < rows; i++) if (i % 3 == 0) { svalid[i / 8] &= ~(1u << (i % 8)); nulls++; }
        const void *sbuf[1] = {svalid};
        struct ArrowArray st = {rows, nulls, 0, 1, 1, sbuf, st_ch, NULL, noop_release, NULL};
        struct ArrowArray *cols[1] = {&st};
        const void *nb[1] = {NULL};
        struct ArrowArray rb = {rows, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};
        struct ArrowSchema s_dict = {"i", "d", NULL, 0, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema s_idx = {"i", "d", NULL, 2, 0, NULL, &s_dict, noop_srelease, NULL};
        struct ArrowSchema *sst_ch[1] = {&s_idx};
        struct ArrowSchema s_st = {"+s", "s", NULL, 2, 1, sst_ch, NULL, noop_srelease, NULL};
        struct ArrowSchema *scols[1] = {&s_st};
        struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
        int rc = import(&rb, &ps, &out);
        if (rc == 0) printf("struct_dict_big %lld: import: ok, %llu rows\n", (long long)rows,
                            (unsigned long long)duckdb_data_chunk_get_size(out));
        // No free() of idx/svalid: after the overflow the heap is corrupt and
        // touching it can abort; the import result is what matters.
        if (rc) return 1;
        return 0; // exit before any further heap activity
    } else { printf("unknown mode %s\n", mode); return 2; }
    if (out) duckdb_destroy_data_chunk(&out);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed. `list_dict` (a dictionary under a list starting at child row 2, whose
index validity marks child rows 0 and 1 NULL and 2..5 valid), identical on all
eight releases:
```text
list_dict: [NULL, NULL] / [500, 600]
```

`struct_dict_big 4096` (a dictionary with no NULLs of its own under a struct
whose every third row is NULL, 4096 rows). The overflow happens inside the
import. Without valgrind the outcome depends on the heap layout: on v1.4.4 and
v1.4.5 the process prints `import: ok` and then aborts in `free()`
(`free(): corrupted unsorted chunks`, exit 134):
```text
struct_dict_big 4096: import: ok, 4096 rows
free(): corrupted unsorted chunks
```

on v1.5.0 and v1.5.1 it crashes during the import (`SIGSEGV`, exit 139, no
output); on v1.5.2-v1.5.5 it prints `import: ok` and exits cleanly:
```text
struct_dict_big 4096: import: ok, 4096 rows
```

Under valgrind, however, all eight releases report the same out-of-bounds
access, byte-for-byte identical except the library path in each frame:
```text
==PID== Invalid read of size 8
==PID==    at 0x…: duckdb::TemplatedValidityMask<unsigned long>::SetInvalid(unsigned long) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb::ArrowToDuckDBConversion::ColumnArrowToDuckDBDictionary(duckdb::Vector&, ArrowArray&, unsigned long, duckdb::ArrowArrayScanState&, unsigned long, duckdb::ArrowType const&, long, duckdb::ValidityMask const*, unsigned long) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb::ArrowToDuckDBConversion::ColumnArrowToDuckDB(duckdb::Vector&, ArrowArray&, unsigned long, duckdb::ArrowArrayScanState&, unsigned long, duckdb::ArrowType const&, long, duckdb::ValidityMask*, unsigned long, bool) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb_data_chunk_from_arrow (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: import (item25.c:29)
==PID==    by 0x…: main (item25.c:107)
==PID==  Address 0x… is 0 bytes after a block of size 256 alloc'd
==PID==    at 0x…: operator new[](unsigned long) (in /usr/libexec/valgrind/vgpreload_memcheck-amd64-linux.so)
==PID==    by 0x…: duckdb::TemplatedValidityMask<unsigned long>::SetInvalid(unsigned long) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb::ArrowToDuckDBConversion::ColumnArrowToDuckDBDictionary(duckdb::Vector&, ArrowArray&, unsigned long, duckdb::ArrowArrayScanState&, unsigned long, duckdb::ArrowType const&, long, duckdb::ValidityMask const*, unsigned long) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb::ArrowToDuckDBConversion::ColumnArrowToDuckDB(duckdb::Vector&, ArrowArray&, unsigned long, duckdb::ArrowArrayScanState&, unsigned long, duckdb::ArrowType const&, long, duckdb::ValidityMask*, unsigned long, bool) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb_data_chunk_from_arrow (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: import (item25.c:29)
==PID==    by 0x…: main (item25.c:107)
==PID== 
```

(The 256-byte block is the 2048-bit mask; the struct's NULLs at rows past 2048
are written into it. This path flags an invalid *read* -- `SetInvalid` is a
read-modify-write of a mask word -- where item 9's own-NULL `memcpy` path flags
an invalid write.) Expected: `[300, 400] / [500, 600]`, and a mask sized for the
number of rows converted. quack-rs mitigation: `data_chunk_from_arrow` refuses a
dictionary-encoded array whose indices are read under a list offset with NULLs,
and any dictionary converted as more than `duckdb_vector_size()` rows that can
hold NULLs (its own or an enclosing struct's); `src/arrow/import_layout.rs`,
exercised end-to-end by `tests/ffi_roundtrip/arrow_layout.rs`.

---
## 26. Arrow import of a dictionary whose values are dictionary-encoded shares one dictionary cache and returns garbage

When a dictionary-encoded array's *values* are themselves dictionary-encoded,
`ColumnArrowToDuckDBDictionary` recurses into
`ColumnArrowToDuckDBDictionary(*base_vector, *array.dictionary, chunk_offset,
array_state, ...)` (`arrow_conversion.cpp:1415`; `:1329` in v1.4.4) passing the
**same `ArrowArrayScanState`**. Both levels share that state's one dictionary
cache (`array_state.CacheOutdated` / `GetDictionary` / `AddDictionary`,
`arrow_conversion.cpp:1400`), so the inner dictionary overwrites the cache the
outer level then slices against, and the rows come back as garbage. The outer
column imports as a dictionary vector; here it is appended to a table and read
back so the logical rows are materialised.

Built with `gcc -O1 -g -Wall -I/opt/duckdb/<ver> item26.c -L/opt/duckdb/<ver> -lduckdb -Wl,-rpath,/opt/duckdb/<ver>` against each prebuilt library.
```c
// duckdb_data_chunk_from_arrow: a dictionary whose values are themselves
// dictionary-encoded. ColumnArrowToDuckDBDictionary recurses into the values'
// dictionary with the SAME ArrowArrayScanState, so both levels share one
// dictionary cache; the inner dictionary overwrites the cache the outer level
// then slices against and the rows come back as garbage.
//
//   inner strings : ['p','q']                (index 0='p', 1='q')
//   mid dict      : indices [1,0] -> ['q','p']  (dictionary = inner strings)
//   outer dict    : indices [0,1,0,0] -> mid   (dictionary = mid dict)
//   logical rows  : q / p / q / q
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
    setvbuf(stdout, NULL, _IONBF, 0);
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);

    // inner VARCHAR values "p","q"
    char sdata[] = "pq";
    int32_t soff[3] = {0, 1, 2};
    const void *sbuf[3] = {NULL, soff, sdata};
    struct ArrowArray inner = {2, 0, 0, 3, 0, sbuf, NULL, NULL, noop_release, NULL};
    // mid dictionary: indices [1,0], dictionary=inner
    int32_t midx[2] = {1, 0};
    const void *mbuf[2] = {NULL, midx};
    struct ArrowArray mid = {2, 0, 0, 2, 0, mbuf, NULL, &inner, noop_release, NULL};
    // outer dictionary: indices [0,1,0,0], dictionary=mid
    int32_t oidx[4] = {0, 1, 0, 0};
    const void *obuf[2] = {NULL, oidx};
    struct ArrowArray outer = {4, 0, 0, 2, 0, obuf, NULL, &mid, noop_release, NULL};
    struct ArrowArray *cols[1] = {&outer};
    const void *nb[1] = {NULL};
    struct ArrowArray rb = {4, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};

    struct ArrowSchema s_inner = {"u", "v", NULL, 0, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema s_mid = {"i", "v", NULL, 0, 0, NULL, &s_inner, noop_srelease, NULL};
    struct ArrowSchema s_outer = {"i", "v", NULL, 0, 0, NULL, &s_mid, noop_srelease, NULL};
    struct ArrowSchema *scols[1] = {&s_outer};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};

    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return 1; }
    duckdb_data_chunk out = NULL;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&rb, conv, &out);
    if (e2) { printf("from_arrow: %s\n", duckdb_error_data_message(e2)); return 1; }

    // The outer column is a dictionary vector; append to a VARCHAR table and
    // read it back so the logical rows are materialised.
    duckdb_result r0; duckdb_query(con, "CREATE TABLE t(v VARCHAR)", &r0); duckdb_destroy_result(&r0);
    duckdb_appender ap; duckdb_appender_create(con, NULL, "t", &ap);
    duckdb_state st = duckdb_append_data_chunk(ap, out);
    if (st != DuckDBSuccess) { printf("append: %s\n", duckdb_appender_error(ap)); return 1; }
    duckdb_appender_close(ap); duckdb_appender_destroy(&ap);
    duckdb_result res; duckdb_query(con, "SELECT v FROM t", &res);
    idx_t n = duckdb_row_count(&res);
    printf("nested_dict: ");
    for (idx_t i = 0; i < n; i++) {
        char *s = duckdb_value_varchar(&res, 0, i);
        printf("%s%s", i ? " / " : "", s ? s : "NULL");
        duckdb_free(s);
    }
    printf("\n");
    duckdb_destroy_result(&res);
    duckdb_destroy_data_chunk(&out);
    duckdb_destroy_arrow_converted_schema(&conv);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed, identical on all eight releases -- the outer dictionary's values come
back as the raw mid-level index numbers (mid = `[1,0]`, indexed by the outer
indices `[0,1,0,0]`), not the strings they encode:
```text
nested_dict: 1 / 0 / 1 / 1
```

Expected: `q / p / q / q`. quack-rs mitigation: `data_chunk_from_arrow` refuses
a dictionary whose values are themselves dictionary-encoded;
`src/arrow/import_layout.rs`, exercised end-to-end by
`tests/ffi_roundtrip/arrow_layout.rs`.

---
## 27. Arrow list-view import scans `sum(sizes)` child rows from the lowest offset, reading past the child

`ConvertArrowListViewOffsetsTemplated` (`arrow_conversion.cpp:139`) sets
`list_size` to the sum of the view's `sizes` and `start_offset` to the lowest
`offset` among rows with a nonzero size, then `ArrowToDuckDBList` converts
`list_size` child elements starting at `start_offset`. A list view's entries
may overlap or leave gaps (unlike a plain list, whose offsets are sequential),
so `start_offset + list_size` can exceed the child's real length and can name
child rows that no view actually covers. DuckDB converts the child over that
range and reads past its buffers, and individual entries whose `offset` lies
outside `[start_offset, start_offset + list_size)` read the wrong rows.

Built with `gcc -O1 -g -Wall -I/opt/duckdb/<ver> item27.c -L/opt/duckdb/<ver> -lduckdb -Wl,-rpath,/opt/duckdb/<ver>` against each prebuilt library, and run under valgrind 3.22.
```c
// duckdb_data_chunk_from_arrow, list-view import. A LIST-VIEW (+vl) has an
// offsets buffer and a sizes buffer; its entries may overlap or leave gaps.
// ConvertArrowListViewOffsetsTemplated sets list_size = sum(sizes) and
// start_offset = lowest offset among nonzero-size rows, then converts
// list_size child elements from start_offset. Modes:
//   overlap  two rows each viewing all 3 strings: sum(sizes)=6 over a 3-string
//            child, so DuckDB reads child rows 3..6 that do not exist.
//   gap      offsets {0,2}, sizes {1,1}: the second row's element (child 2) is
//            outside the scanned range [0,2), so it reads the wrong row.
//   ok       sequential (control): offsets {0,1}, sizes {1,2}.
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc < 2) { printf("usage: %s <overlap|gap|ok>\n", argv[0]); return 2; }
    const char *mode = argv[1];
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);

    // child VARCHAR ['aaa','bbb','ccc']. Heap-allocated and sized exactly for
    // the 3 strings, so an over-read of the offsets buffer is a heap error
    // valgrind can see.
    char *cdata = malloc(9); memcpy(cdata, "aaabbbccc", 9);
    int32_t *coff = malloc(4 * sizeof(int32_t)); coff[0] = 0; coff[1] = 3; coff[2] = 6; coff[3] = 9;
    const void *cbuf[3] = {NULL, coff, cdata};
    struct ArrowArray child = {3, 0, 0, 3, 0, cbuf, NULL, NULL, noop_release, NULL};

    int32_t offs[2], sizes[2];
    if (!strcmp(mode, "overlap")) { offs[0] = 0; offs[1] = 0; sizes[0] = 3; sizes[1] = 3; }
    else if (!strcmp(mode, "gap")) { offs[0] = 0; offs[1] = 2; sizes[0] = 1; sizes[1] = 1; }
    else if (!strcmp(mode, "ok")) { offs[0] = 0; offs[1] = 1; sizes[0] = 1; sizes[1] = 2; }
    else { printf("unknown mode %s\n", mode); return 2; }

    const void *lbuf[3] = {NULL, offs, sizes};
    struct ArrowArray *lch[1] = {&child};
    struct ArrowArray lv = {2, 0, 0, 3, 1, lbuf, lch, NULL, noop_release, NULL};
    struct ArrowArray *cols[1] = {&lv};
    const void *nb[1] = {NULL};
    struct ArrowArray rb = {2, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};

    struct ArrowSchema s_child = {"u", "item", NULL, 0, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema *slc[1] = {&s_child};
    struct ArrowSchema s_lv = {"+vl", "l", NULL, 0, 1, slc, NULL, noop_srelease, NULL};
    struct ArrowSchema *scols[1] = {&s_lv};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};

    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return 1; }
    duckdb_data_chunk out = NULL;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&rb, conv, &out);
    if (e2) { printf("from_arrow: %s\n", duckdb_error_data_message(e2)); return 1; }

    duckdb_result r0; duckdb_query(con, "CREATE TABLE t(l VARCHAR[])", &r0); duckdb_destroy_result(&r0);
    duckdb_appender ap; duckdb_appender_create(con, NULL, "t", &ap);
    duckdb_state st = duckdb_append_data_chunk(ap, out);
    if (st != DuckDBSuccess) { printf("append: %s\n", duckdb_appender_error(ap)); return 1; }
    duckdb_appender_close(ap); duckdb_appender_destroy(&ap);
    duckdb_result res; duckdb_query(con, "SELECT l::VARCHAR FROM t", &res);
    idx_t n = duckdb_row_count(&res);
    printf("%s: ", mode);
    for (idx_t i = 0; i < n; i++) {
        char *s = duckdb_value_varchar(&res, 0, i);
        printf("%s%s", i ? " / " : "", s ? s : "NULL");
        duckdb_free(s);
    }
    printf("\n");
    duckdb_destroy_result(&res);
    duckdb_destroy_data_chunk(&out);
    duckdb_destroy_arrow_converted_schema(&conv);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed. `overlap` (two rows both viewing all three strings, so
`sum(sizes) = 6` over a three-string child). Whether the process crashes or
returns the (still correct-looking) rows depends on the heap layout: it crashes
on v1.4.4 and v1.4.5 (`SIGSEGV`, exit 139) and returns on v1.5.0-v1.5.5:
```text
overlap: [aaa, bbb, ccc] / [aaa, bbb, ccc]
```

but under valgrind all eight releases report the same out-of-bounds read of the
child's offsets buffer, byte-for-byte identical except the library path:
```text
==PID== Invalid read of size 4
==PID==    at 0x…: duckdb::ArrowToDuckDBConversion::ColumnArrowToDuckDB(duckdb::Vector&, ArrowArray&, unsigned long, duckdb::ArrowArrayScanState&, unsigned long, duckdb::ArrowType const&, long, duckdb::ValidityMask*, unsigned long, bool) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb::ArrowToDuckDBList(duckdb::Vector&, ArrowArray&, unsigned long, duckdb::ArrowArrayScanState&, unsigned long, duckdb::ArrowType const&, long, duckdb::ValidityMask const*, long) (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: duckdb_data_chunk_from_arrow (in /opt/duckdb/<ver>/libduckdb.so)
==PID==    by 0x…: main (item27.c:60)
==PID==  Address 0x… is 0 bytes after a block of size 16 alloc'd
==PID==    at 0x…: malloc (in /usr/libexec/valgrind/vgpreload_memcheck-amd64-linux.so)
==PID==    by 0x…: main (item27.c:33)
==PID== 
```

`gap` (offsets `{0,2}`, sizes `{1,1}`, so the second row's element lies outside
the scanned range `[0, 2)`) imports with no error and returns the wrong second
row. v1.4.4, v1.4.5, v1.5.3, v1.5.4 and v1.5.5 print `[aaa]` for both rows:
```text
gap: [aaa] / [aaa]
```

while v1.5.0, v1.5.1 and v1.5.2 crash reading the out-of-range string element
(`SIGSEGV`, exit 139). The control `ok` (sequential offsets `{0,1}`, sizes
`{1,2}`) is correct on all eight:
```text
ok: [aaa] / [bbb, ccc]
```

Expected: for `gap`, `[aaa] / [ccc]`. quack-rs mitigation:
`data_chunk_from_arrow` refuses a list view whose views overlap or leave gaps
(the furthest child element in use exceeds `lowest_offset + sum(sizes)`);
`src/arrow/import_layout.rs`, exercised end-to-end by
`tests/ffi_roundtrip/arrow_layout.rs`.

---
## 28. Arrow sparse-union import uses the type codes as member indices, ignoring the `+us:` code list

An Arrow union type is `+us:<codes>`, where `<codes>` lists the type code that
labels each member position, in order. A value's type id in the union's buffer
is one of those codes, not a member index, so a producer may use any codes it
likes. DuckDB reads the type id and uses it directly as the member index,
ignoring the code list (`UNION` case, `arrow_conversion.cpp:1199`; the tag is
used as `children[tag]` and range-checked against `array.n_children` at
`:1245`, `:1163` in v1.4.4). So a union whose codes are not `0, 1, ...` in order
imports the wrong members, and a code at or above the member count is reported
as "Arrow union tag out of range" even though it is a valid code.

Built with `gcc -O1 -g -Wall -I/opt/duckdb/<ver> item28.c -L/opt/duckdb/<ver> -lduckdb -Wl,-rpath,/opt/duckdb/<ver>` against each prebuilt library.
```c
// duckdb_data_chunk_from_arrow, sparse-union import. The Arrow union format
// "+us:<codes>" lists the type code for each member position; a value's type
// id in the buffer is one of those codes, not a member index. DuckDB uses the
// type id directly as the member index and ignores the code list. Modes:
//   swap  "+us:1,0": member 0 has code 1, member 1 has code 0. A row whose
//         type id is 0 means "member with code 0" = member position 1, but
//         DuckDB reads member position 0. The two members are swapped.
//   oob   "+us:5,2": codes 5 and 2; a type id of 5 is not a member index
//         (only 2 members), so DuckDB throws "Arrow union tag out of range".
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

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc < 2) { printf("usage: %s <swap|oob>\n", argv[0]); return 2; }
    const char *mode = argv[1];
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);

    int32_t m0[2] = {10, 11};
    int32_t m1[2] = {20, 21};
    const void *b0[2] = {NULL, m0};
    const void *b1[2] = {NULL, m1};
    struct ArrowArray a0 = {2, 0, 0, 2, 0, b0, NULL, NULL, noop_release, NULL};
    struct ArrowArray a1 = {2, 0, 0, 2, 0, b1, NULL, NULL, noop_release, NULL};
    struct ArrowArray *members[2] = {&a0, &a1};

    const char *fmt;
    int8_t tids[2];
    if (!strcmp(mode, "swap")) { fmt = "+us:1,0"; tids[0] = 0; tids[1] = 1; }
    else if (!strcmp(mode, "oob")) { fmt = "+us:5,2"; tids[0] = 5; tids[1] = 2; }
    else { printf("unknown mode %s\n", mode); return 2; }

    const void *ubuf[1] = {tids};
    struct ArrowArray uni = {2, 0, 0, 1, 2, ubuf, members, NULL, noop_release, NULL};
    struct ArrowArray *cols[1] = {&uni};
    const void *nb[1] = {NULL};
    struct ArrowArray rb = {2, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};

    struct ArrowSchema s0 = {"i", "a", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema s1 = {"i", "b", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema *um[2] = {&s0, &s1};
    struct ArrowSchema s_uni = {fmt, "u", NULL, 0, 2, um, NULL, noop_srelease, NULL};
    struct ArrowSchema *scols[1] = {&s_uni};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};

    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return 1; }
    duckdb_data_chunk out = NULL;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&rb, conv, &out);
    if (e2) { printf("%s: from_arrow error: %s\n", mode, duckdb_error_data_message(e2)); return 1; }

    duckdb_vector uv = duckdb_data_chunk_get_vector(out, 0);
    duckdb_vector tagv = duckdb_struct_vector_get_child(uv, 0);
    uint8_t *tag = duckdb_vector_get_data(tagv);
    duckdb_vector mv0 = duckdb_struct_vector_get_child(uv, 1);
    duckdb_vector mv1 = duckdb_struct_vector_get_child(uv, 2);
    int32_t *dd0 = duckdb_vector_get_data(mv0);
    int32_t *dd1 = duckdb_vector_get_data(mv1);
    idx_t n = duckdb_data_chunk_get_size(out);
    printf("%s: ", mode);
    for (idx_t i = 0; i < n; i++) { if (i) printf(" / "); printf("%d", tag[i] == 0 ? dd0[i] : dd1[i]); }
    printf("\n");
    duckdb_destroy_data_chunk(&out);
    duckdb_destroy_arrow_converted_schema(&conv);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed. `swap` (`+us:1,0`, two INT members, type ids `[0,1]`), identical on
all eight releases -- DuckDB reads the type id `0` as member index 0 and `1` as
index 1, so the members are swapped relative to their codes:
```text
swap: 10 / 21
```

`oob` (`+us:5,2`, two INT members, type ids `[5,2]`), identical on all eight --
the type id `5` is a valid code but not a member index, and the import fails
(exit 1, no crash):
```text
oob: from_arrow error: {"exception_type":"Invalid Input","exception_message":"Arrow union tag out of range: 5"}
```

Expected: for `swap`, `20 / 11` (the value with code `0` is member position 1);
for `oob`, member position 0 (the member whose code is `5`). quack-rs
mitigation: `data_chunk_from_arrow` refuses a sparse union whose type codes are
not `0, 1, ...` in order (a "recoded" union);
`src/arrow/import_layout.rs`, exercised end-to-end by
`tests/ffi_roundtrip/arrow_layout.rs`.

---

## 29. Arrow dictionary import points NULL indices one element past the producer's values buffer

`ColumnArrowToDuckDBDictionary` (`arrow_conversion.cpp`) allocates the
dictionary vector with `dictionary->length + 1` entries and marks the last
one invalid, as a sentinel: `SetMaskedSelectionVectorLoop` points every NULL
index at it. The values are then converted into that vector, but for a
fixed-width type (the integer and float types, `DATE` in days, `TIMESTAMP`,
and the others `DirectConversion` handles) the conversion replaces the
vector's data with a pointer into the Arrow values buffer
(`FlatVector::SetData`), so the sentinel entry lies one element past the
producer's buffer. `VectorOperations::Copy` (`TemplatedCopy`,
`vector_copy.cpp`) copies values without consulting validity, so copying
the imported vector, for example with `duckdb_vector_copy_sel`, reads that
element. The rows it fills are NULL, so the result is right; the read is
the defect. `duckdb_append_data_chunk` does not reach it (valgrind is clean
for that path).

Environment: prebuilt `libduckdb` v1.4.4, v1.4.5 and v1.5.0 to v1.5.5,
x86_64 Linux, valgrind 3.22.0. Build: `gcc -I<libduckdb dir> item29.c
-L<libduckdb dir> -lduckdb -o item29`, then run with
`LD_LIBRARY_PATH=<libduckdb dir>` (under `valgrind` for the read).

```c
// duckdb_data_chunk_from_arrow: a dictionary-encoded INTEGER column with a
// NULL index. ColumnArrowToDuckDBDictionary points every NULL row's selection
// at a sentinel entry one past the dictionary (dictionary->length), but for a
// fixed-width type the dictionary vector's data is the Arrow values buffer
// itself (DirectConversion), so the sentinel lies past the producer's buffer.
// Copying the vector with duckdb_vector_copy_sel reads it; DuckDB's own
// appender does not (valgrind is clean for that path).
// Build: gcc -I<libduckdb dir> item29.c -L<libduckdb dir> -lduckdb -o item29
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    printf("Built with DuckDB %s\n", duckdb_library_version());
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);

    // Dictionary values [5, 6] in a heap buffer of exactly 8 bytes.
    int32_t *values = malloc(2 * sizeof(int32_t));
    values[0] = 5; values[1] = 6;
    const void *vbuf[2] = {NULL, values};
    struct ArrowArray dict = {2, 0, 0, 2, 0, vbuf, NULL, NULL, noop_release, NULL};
    // Indices [1, NULL, 0]: bit 1 of the validity bitmap is clear.
    uint8_t valid = 0x05;
    int32_t idx[3] = {1, 0, 0};
    const void *ibuf[2] = {&valid, idx};
    struct ArrowArray col = {3, 1, 0, 2, 0, ibuf, NULL, &dict, noop_release, NULL};
    struct ArrowArray *cols[1] = {&col};
    const void *nb[1] = {NULL};
    struct ArrowArray rb = {3, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};

    struct ArrowSchema s_values = {"i", "v", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema s_col = {"i", "v", NULL, 2, 0, NULL, &s_values, noop_srelease, NULL};
    struct ArrowSchema *scols[1] = {&s_col};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};

    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return 1; }
    duckdb_data_chunk out = NULL;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&rb, conv, &out);
    if (e2) { printf("from_arrow: %s\n", duckdb_error_data_message(e2)); return 1; }

    duckdb_vector src = duckdb_data_chunk_get_vector(out, 0);
    duckdb_logical_type ty = duckdb_create_logical_type(DUCKDB_TYPE_INTEGER);
    duckdb_vector dst = duckdb_create_vector(ty, 3);
    duckdb_selection_vector sel = duckdb_create_selection_vector(3);
    sel_t *s = duckdb_selection_vector_get_data_ptr(sel);
    for (int i = 0; i < 3; i++) s[i] = (sel_t)i;
    duckdb_vector_copy_sel(src, dst, sel, 3, 0, 0);
    int32_t *d = (int32_t *)duckdb_vector_get_data(dst);
    uint64_t *m = duckdb_vector_get_validity(dst);
    printf("rows:");
    for (int i = 0; i < 3; i++) {
        if (m && !duckdb_validity_row_is_valid(m, i)) printf(" NULL"); else printf(" %d", d[i]);
    }
    printf("\n");
    duckdb_destroy_selection_vector(sel);
    duckdb_destroy_vector(&dst);
    duckdb_destroy_logical_type(&ty);
    duckdb_destroy_data_chunk(&out);
    duckdb_destroy_arrow_converted_schema(&conv);
    duckdb_disconnect(&con); duckdb_close(&db);
    free(values);
    return 0;
}
```

Observed, identical on all eight releases apart from the version line
(program output compared with `md5sum`; valgrind reported 1 error from 1
context on each, the same invalid read of size 4 in
`duckdb::VectorOperations::Copy`, 0 bytes after the 8-byte `malloc` block
of the values). The valgrind trace is 1.5.5's, with the process id,
addresses and the program's path normalised:

```text
Built with DuckDB v1.5.5
rows: 6 NULL 5
```

```text
==PID== Invalid read of size 4
==PID==    at 0x…: duckdb::VectorOperations::Copy(duckdb::Vector const&, duckdb::Vector&, duckdb::SelectionVector const&, unsigned long, unsigned long, unsigned long, unsigned long) (in /opt/duckdb/1.5.5/libduckdb.so)
==PID==    by 0x…: duckdb::VectorOperations::Copy(duckdb::Vector const&, duckdb::Vector&, duckdb::SelectionVector const&, unsigned long, unsigned long, unsigned long) (in /opt/duckdb/1.5.5/libduckdb.so)
==PID==    by 0x…: main (in item29)
==PID==  Address 0x… is 0 bytes after a block of size 8 alloc'd
==PID==    at 0x…: malloc (in /usr/libexec/valgrind/vgpreload_memcheck-amd64-linux.so)
==PID==    by 0x…: main (in item29)
```

Expected: the sentinel entry inside memory `DuckDB` owns (copy the values
when the indices can be NULL), or a copy that skips invalid rows.

quack-rs mitigation: `data_chunk_from_arrow` flattens every
dictionary-encoded column with `duckdb_vector_copy_sel`, so it performs this
read itself. Its Safety section requires such a dictionary's values buffer
to be readable for one element past `offset + length`. Found by running
`tests/ffi_roundtrip/arrow_layout.rs` against a `libduckdb` built with
AddressSanitizer (heap-buffer-overflow in `TemplatedCopy<int>`); that test's
dictionaries are now padded as the Safety section requires.

---

## 30. Table-description column accessors abort the process on index `idx_t(-1)`

From 1.5.0, `duckdb_table_description_get_column_name`,
`duckdb_table_description_get_column_type` and `duckdb_column_has_default`
pass their `idx_t index` to `GetTableDescription(TableDescriptionWrapper *,
duckdb::optional_idx)` (`table_description-c.cpp`). The implicit
`optional_idx(idx_t)` constructor throws `InternalException` for
`INVALID_INDEX`, which is `idx_t(-1)` (`optional_idx.hpp`), and none of the
three functions has a `try`, so the exception leaves the C API. Every other
out-of-range index is reported as an error. 1.4.4 and 1.4.5 take a plain
`idx_t` and return NULL.

Environment: prebuilt `libduckdb` v1.4.4, v1.4.5 and v1.5.0 to v1.5.5, x86_64
Linux. Build: `gcc -I<libduckdb dir> item30.c -L<libduckdb dir> -lduckdb -o
item30`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_table_description_get_column_name (and _get_column_type,
// duckdb_column_has_default) convert the index to duckdb::optional_idx, whose
// constructor throws InternalException for idx_t(-1), outside any try.
// Build: gcc -I<libduckdb dir> item30.c -L<libduckdb dir> -lduckdb -o item30
#include <stdio.h>
#include <stdint.h>
#include <duckdb.h>
int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    printf("Built with DuckDB %s\n", duckdb_library_version());
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_query(con, "CREATE TABLE t(a INTEGER)", NULL);
    duckdb_table_description desc;
    if (duckdb_table_description_create(con, "main", "t", &desc) != DuckDBSuccess) { printf("create failed\n"); return 1; }
    char *in_range = duckdb_table_description_get_column_name(desc, 0);
    printf("index 0: %s\n", in_range ? in_range : "NULL");
    duckdb_free(in_range);
    char *past = duckdb_table_description_get_column_name(desc, 5);
    printf("index 5: %s\n", past ? past : "NULL");
    printf("index UINT64_MAX:\n");
    char *max = duckdb_table_description_get_column_name(desc, UINT64_MAX);
    printf("returned %s\n", max ? max : "NULL");
    duckdb_table_description_destroy(&desc);
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed, in two groups (output compared with `md5sum`, the version line
and the stack-trace pointers set aside). v1.4.4 and v1.4.5, exit status 0:

```text
Built with DuckDB v1.4.4
index 0: a
index 5: NULL
index UINT64_MAX:
returned NULL
```

v1.5.0 to v1.5.5, exit status 134 (SIGABRT); this is v1.5.5:

```text
Built with DuckDB v1.5.5
index 0: a
index 5: NULL
index UINT64_MAX:
terminate called after throwing an instance of 'duckdb::InternalException'
  what():  {"exception_type":"INTERNAL","exception_message":"optional_idx cannot be initialized with an invalid index","stack_trace_pointers":"…"}
```

Expected: an error state, as for index 5.

quack-rs mitigation: `TableDescription::column_name`, `column_type` and
`column_has_default` return `None` for `idx_t::MAX` without calling `DuckDB`
(`tests/ffi_roundtrip/table_description.rs` aborted before the fix).

---

## 31. An Arrow dictionary with `null_count = -1` imports its NULL rows as values

The Arrow C Data Interface lets a producer set `null_count` to -1 when it has
not computed it. `GetValidityMask` (`arrow_conversion.cpp`) copies the
validity bitmap whenever `null_count != 0`, but `CanContainNull`, which
decides whether `ColumnArrowToDuckDBDictionary` builds the selection with
the indices' validity, tests `null_count > 0`. With -1 the selection is built
from the raw indices, so a NULL row's index, whatever the producer left in
that slot, is looked up in the dictionary: out of range here, so the value
comes from past the dictionary's buffer.

Environment: prebuilt `libduckdb` v1.4.4, v1.4.5 and v1.5.0 to v1.5.5, x86_64
Linux. Build: `gcc -I<libduckdb dir> item31.c -L<libduckdb dir> -lduckdb -o
item31`; run `item31 dict 1`, `item31 dict -1`, `item31 union 0` and
`item31 union -1` with `LD_LIBRARY_PATH=<libduckdb dir>`. The same program
reproduces item 32.

```c
// duckdb_data_chunk_from_arrow with null_count = -1 ("not computed", which the
// Arrow C Data Interface allows):
//   dict  : a dictionary-encoded INTEGER column, indices [0, 7, 1] with row 1
//           NULL (validity 0b101), dictionary [100, 200]. GetValidityMask
//           copies the bitmap (it tests null_count != 0), but CanContainNull
//           tests null_count > 0, so the selection is built from the raw
//           indices: row 1 reads dictionary entry 7.
//   union : a sparse union <a: INTEGER[], b: INTEGER> with type ids [0, 0].
//           A union has no validity bitmap, but with null_count != 0
//           GetValidityMask reads buffer 0 (the type ids) as one.
// Usage: item31 dict|union <null_count>
// Build: gcc -I<libduckdb dir> item31.c -L<libduckdb dir> -lduckdb -o item31
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }

static void run(duckdb_connection con, struct ArrowSchema *col_schema, struct ArrowArray *col, int64_t rows,
                const char *sql_type) {
    struct ArrowSchema *scols[1] = {col_schema};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
    struct ArrowArray *cols[1] = {col};
    const void *nb[1] = {NULL};
    struct ArrowArray rb = {rows, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};
    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return; }
    duckdb_data_chunk out = NULL;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&rb, conv, &out);
    if (e2) { printf("from_arrow: %s\n", duckdb_error_data_message(e2)); return; }
    char sql[128];
    snprintf(sql, sizeof sql, "CREATE TABLE t(v %s)", sql_type);
    duckdb_query(con, sql, NULL);
    duckdb_appender ap; duckdb_appender_create(con, NULL, "t", &ap);
    if (duckdb_append_data_chunk(ap, out) != DuckDBSuccess) { printf("append: %s\n", duckdb_appender_error(ap)); return; }
    duckdb_appender_close(ap); duckdb_appender_destroy(&ap);
    duckdb_result res; duckdb_query(con, "SELECT coalesce(v::VARCHAR, 'NULL') FROM t ORDER BY rowid", &res);
    printf("rows:");
    for (idx_t i = 0; i < duckdb_row_count(&res); i++) {
        char *s = duckdb_value_varchar(&res, 0, i);
        printf(" %s", s);
        duckdb_free(s);
    }
    printf("\n");
    duckdb_destroy_result(&res);
    duckdb_destroy_data_chunk(&out);
    duckdb_destroy_arrow_converted_schema(&conv);
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc < 3) return 2;
    int64_t null_count = atoll(argv[2]);
    printf("Built with DuckDB %s, %s, null_count %lld\n", duckdb_library_version(), argv[1], (long long)null_count);
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);
    if (strcmp(argv[1], "dict") == 0) {
        int32_t *values = malloc(2 * sizeof(int32_t));
        values[0] = 100; values[1] = 200;
        const void *vbuf[2] = {NULL, values};
        struct ArrowArray dict = {2, 0, 0, 2, 0, vbuf, NULL, NULL, noop_release, NULL};
        uint8_t valid = 0x05;
        int32_t idx[3] = {0, 7, 1};
        const void *ibuf[2] = {&valid, idx};
        struct ArrowArray col = {3, null_count, 0, 2, 0, ibuf, NULL, &dict, noop_release, NULL};
        struct ArrowSchema s_values = {"i", "v", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema s_col = {"i", "v", NULL, 2, 0, NULL, &s_values, noop_srelease, NULL};
        run(con, &s_col, &col, 3, "INTEGER");
        free(values);
    } else {
        // Member a: INTEGER[] with rows [5] and [6]; member b: INTEGER.
        int32_t items[2] = {5, 6};
        const void *ibuf[2] = {NULL, items};
        struct ArrowArray item = {2, 0, 0, 2, 0, ibuf, NULL, NULL, noop_release, NULL};
        struct ArrowArray *item_children[1] = {&item};
        int32_t offsets[3] = {0, 1, 2};
        const void *lbuf[2] = {NULL, offsets};
        struct ArrowArray list = {2, 0, 0, 2, 1, lbuf, item_children, NULL, noop_release, NULL};
        int32_t bvals[2] = {0, 0};
        const void *bbuf[2] = {NULL, bvals};
        struct ArrowArray b = {2, 0, 0, 2, 0, bbuf, NULL, NULL, noop_release, NULL};
        struct ArrowArray *members[2] = {&list, &b};
        int8_t type_ids[2] = {0, 0};
        const void *ubuf[1] = {type_ids};
        struct ArrowArray col = {2, null_count, 0, 1, 2, ubuf, members, NULL, noop_release, NULL};
        struct ArrowSchema s_item = {"i", "item", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema *s_item_children[1] = {&s_item};
        struct ArrowSchema s_a = {"+l", "a", NULL, 2, 1, s_item_children, NULL, noop_srelease, NULL};
        struct ArrowSchema s_b = {"i", "b", NULL, 2, 0, NULL, NULL, noop_srelease, NULL};
        struct ArrowSchema *s_members[2] = {&s_a, &s_b};
        struct ArrowSchema s_col = {"+us:0,1", "v", NULL, 2, 2, s_members, NULL, noop_srelease, NULL};
        run(con, &s_col, &col, 2, "UNION(a INTEGER[], b INTEGER)");
    }
    duckdb_disconnect(&con); duckdb_close(&db);
    return 0;
}
```

Observed. `item31 dict 1`, identical on all eight releases apart from the
version line:

```text
Built with DuckDB v1.5.5, dict, null_count 1
rows: 100 NULL 200
```

`item31 dict -1`: row 1 is never NULL; its value is whatever lies past the
dictionary, and differs by release (`md5sum` puts v1.5.4 and v1.5.5 in one
group, each other release alone):

```text
Built with DuckDB v1.4.4, dict, null_count -1
rows: 100 22086 200
```

```text
Built with DuckDB v1.5.5, dict, null_count -1
rows: 100 0 200
```

Expected: `100 NULL 200`, as with the count set.

quack-rs mitigation: `data_chunk_from_arrow` refuses a dictionary-encoded
array whose `null_count` is negative while it has a validity buffer
(`src/arrow/import_layout.rs`; `tests/ffi_roundtrip/arrow_layout.rs`).

---

## 32. An Arrow sparse union with a nonzero `null_count` has its type ids read as a validity bitmap

A union array has no validity bitmap: a sparse union's buffer 0 holds its
type ids. `GetValidityMask` (`arrow_conversion.cpp`) still copies buffer 0 as
a bitmap whenever `null_count != 0`, including the -1 the C Data Interface
allows for "not computed", and the union's rows take NULLs from its type
ids.

Reproducer: the program in item 31, modes `union 0` and `union -1`.

Observed, identical on all eight releases apart from the version line
(`md5sum`):

```text
Built with DuckDB v1.5.5, union, null_count 0
rows: [5] [6]
```

```text
Built with DuckDB v1.5.5, union, null_count -1
rows: NULL NULL
```

Expected: `[5] [6]` in both.

quack-rs mitigation: `data_chunk_from_arrow` refuses a union node whose
`null_count` is not 0 (`src/arrow/import_layout.rs`;
`tests/ffi_roundtrip/arrow_layout.rs`).

---

## 33. Arrow import of a `geoarrow.wkb` column of more than 2048 rows writes past a vector

`ColumnArrowToDuckDB` (`arrow_conversion.cpp`) converts an Arrow extension
type that has an `arrow_to_duckdb` step by converting the storage first into
`Vector input_data(arrow_type.extension_data->GetInternalType())`, whose
capacity is the standard vector size (2048), passing `size`, the batch's row
count. From 1.5.0 `geoarrow.wkb` is registered in core with such a step
(`arrow_type_extension.cpp`), and its `BLOB` storage conversion writes
`size` strings into that vector. (`arrow.bool8`, the only other core
extension with a conversion step, is imported without a copy.)

Environment: prebuilt `libduckdb` v1.4.4, v1.4.5 and v1.5.0 to v1.5.5, x86_64
Linux. Build: `gcc -I<libduckdb dir> item33.c -L<libduckdb dir> -lduckdb -o
item33`; run `item33 2048` and `item33 4096` with
`LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_data_chunk_from_arrow on a geoarrow.wkb column of more than 2048
// rows. ColumnArrowToDuckDB converts an extension type with an
// arrow_to_duckdb step by first converting its storage into
// `Vector input_data(type)`, whose capacity is STANDARD_VECTOR_SIZE (2048),
// with `size` rows: past 2048 the BLOB conversion writes past the vector.
// Usage: item33 <rows>
// Build: gcc -I<libduckdb dir> item33.c -L<libduckdb dir> -lduckdb -o item33
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
static void noop_release(struct ArrowArray *a) { a->release = NULL; }
static void noop_srelease(struct ArrowSchema *s) { s->release = NULL; }

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    int64_t rows = argc > 1 ? atoll(argv[1]) : 4096;
    printf("Built with DuckDB %s, %lld rows\n", duckdb_library_version(), (long long)rows);
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);
    // WKB POINT(i 0): byte order 1, type 1, x, y.
    uint8_t *data = malloc(21 * rows);
    int32_t *offsets = malloc(4 * (rows + 1));
    offsets[0] = 0;
    for (int64_t i = 0; i < rows; i++) {
        uint8_t *p = data + 21 * i;
        uint32_t type = 1; double x = (double)i, y = 0;
        p[0] = 1; memcpy(p + 1, &type, 4); memcpy(p + 5, &x, 8); memcpy(p + 13, &y, 8);
        offsets[i + 1] = (int32_t)(21 * (i + 1));
    }
    const void *buf[3] = {NULL, offsets, data};
    struct ArrowArray col = {rows, 0, 0, 3, 0, buf, NULL, NULL, noop_release, NULL};
    struct ArrowArray *cols[1] = {&col};
    const void *nb[1] = {NULL};
    struct ArrowArray rb = {rows, 0, 0, 1, 1, nb, cols, NULL, noop_release, NULL};
    // Metadata: one pair, ARROW:extension:name = geoarrow.wkb.
    char meta[64]; int32_t n = 1, kl = 20, vl = 12;
    memcpy(meta, &n, 4); memcpy(meta + 4, &kl, 4); memcpy(meta + 8, "ARROW:extension:name", 20);
    memcpy(meta + 28, &vl, 4); memcpy(meta + 32, "geoarrow.wkb", 12);
    struct ArrowSchema s_col = {"z", "g", meta, 2, 0, NULL, NULL, noop_srelease, NULL};
    struct ArrowSchema *scols[1] = {&s_col};
    struct ArrowSchema ps = {"+s", "", NULL, 0, 1, scols, NULL, noop_srelease, NULL};
    duckdb_arrow_converted_schema conv;
    duckdb_error_data e1 = duckdb_schema_from_arrow(con, (void *)&ps, &conv);
    if (e1) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e1)); return 1; }
    duckdb_data_chunk out = NULL;
    duckdb_error_data e2 = duckdb_data_chunk_from_arrow(con, (void *)&rb, conv, &out);
    printf("import: %s\n", e2 ? duckdb_error_data_message(e2) : "ok");
    if (out) duckdb_destroy_data_chunk(&out);
    duckdb_destroy_arrow_converted_schema(&conv);
    duckdb_disconnect(&con); duckdb_close(&db);
    printf("done\n");
    return 0;
}
```

Observed. With 2048 rows, on all eight releases:

```text
Built with DuckDB v1.5.5, 2048 rows
import: ok
done
exit=0
```

With 4096 rows, v1.4.4 and v1.4.5 (no `geoarrow.wkb` registration; the
column imports as `BLOB`):

```text
Built with DuckDB v1.4.4, 4096 rows
import: ok
done
exit=0
```

v1.5.0:

```text
Built with DuckDB v1.5.0, 4096 rows
malloc(): corrupted top size
exit=134
```

v1.5.1 to v1.5.5 (SIGSEGV):

```text
Built with DuckDB v1.5.5, 4096 rows
exit=139
```

Expected: the rows imported, as up to 2048.

quack-rs mitigation: `data_chunk_from_arrow` refuses a `geoarrow.wkb` node
that `DuckDB` would convert as more than 2048 rows
(`src/arrow/import_layout.rs`; `tests/ffi_roundtrip/arrow_layout.rs`, which
crashed with SIGSEGV before the check). Extension types that other `DuckDB`
extensions register may take the same path and are not known here.

---

## 34. Arrow export declares plain binary for `BIGNUM` and `GEOMETRY` but writes binary views

Under `arrow_output_version = '1.4'` the appender writes non-`VARCHAR`
binary data in the four-buffer binary-view layout, but before 1.5.5
`duckdb_to_arrow_schema` declares `BIGNUM` (and, from 1.5.0, `GEOMETRY`,
registered as `geoarrow.wkb`) as plain binary, format `z`, which has three
buffers (`arrow_type_extension.cpp`, `PopulateSchema`; 1.5.5 declares `vz`).
A consumer reads the view structs as offsets. `DuckDB`'s own import of the
pair reports success; using the imported chunk crashes.

Environment: prebuilt `libduckdb` v1.4.4, v1.4.5 and v1.5.0 to v1.5.5, x86_64
Linux. Build: `gcc -I<libduckdb dir> item34.c -L<libduckdb dir> -lduckdb -o
item34`; run `item34 bignum` and `item34 geometry` with
`LD_LIBRARY_PATH=<libduckdb dir>`.

```c
// duckdb_data_chunk_to_arrow under arrow_output_version = '1.4': does the
// array have the buffers the schema from duckdb_to_arrow_schema declares?
// Plain binary ("z") has three buffers; a binary view ("vz") has four. The
// exported pair is then imported back.
// Usage: item34 bignum|geometry
// Build: gcc -I<libduckdb dir> item34.c -L<libduckdb dir> -lduckdb -o item34
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <duckdb.h>
struct ArrowArray { int64_t length, null_count, offset, n_buffers, n_children; const void **buffers;
  struct ArrowArray **children; struct ArrowArray *dictionary; void (*release)(struct ArrowArray *); void *private_data; };
struct ArrowSchema { const char *format, *name, *metadata; int64_t flags, n_children; struct ArrowSchema **children;
  struct ArrowSchema *dictionary; void (*release)(struct ArrowSchema *); void *private_data; };
int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    const char *q = argc > 1 && strcmp(argv[1], "geometry") == 0 ? "SELECT 'POINT (1 2)'::GEOMETRY"
                                                                   : "SELECT 123456789012345678901234567890::BIGNUM";
    printf("Built with DuckDB %s, %s\n", duckdb_library_version(), q);
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_query(con, "SET arrow_output_version = '1.4'", NULL);
    duckdb_result r;
    if (duckdb_query(con, q, &r) != DuckDBSuccess) { printf("query: %s\n", duckdb_result_error(&r)); return 1; }
    duckdb_data_chunk ch = duckdb_fetch_chunk(r);
    duckdb_arrow_options opt; duckdb_connection_get_arrow_options(con, &opt);
    duckdb_logical_type t = duckdb_column_logical_type(&r, 0);
    const char *nm = "c";
    struct ArrowSchema s = {0}; struct ArrowArray a = {0};
    duckdb_error_data e = duckdb_to_arrow_schema(opt, &t, &nm, 1, (void *)&s);
    if (e) { printf("schema: %s\n", duckdb_error_data_message(e)); return 1; }
    e = duckdb_data_chunk_to_arrow(opt, ch, (void *)&a);
    if (e) { printf("export: %s\n", duckdb_error_data_message(e)); return 1; }
    printf("schema format %s, array n_buffers %lld\n", s.children[0]->format, (long long)a.children[0]->n_buffers);
    duckdb_arrow_converted_schema conv;
    e = duckdb_schema_from_arrow(con, (void *)&s, &conv);
    if (e) { printf("schema_from_arrow: %s\n", duckdb_error_data_message(e)); return 1; }
    printf("importing\n");
    duckdb_data_chunk back = NULL;
    e = duckdb_data_chunk_from_arrow(con, (void *)&a, conv, &back);
    printf("import: %s\n", e ? duckdb_error_data_message(e) : "ok");
    if (e) return 0;
    duckdb_query(con, "CREATE TABLE rt AS SELECT * FROM (SELECT NULL::BIGNUM c) WHERE false", NULL);
    duckdb_appender ap; duckdb_appender_create(con, NULL, "rt", &ap);
    if (duckdb_append_data_chunk(ap, back) != DuckDBSuccess) { printf("append: %s\n", duckdb_appender_error(ap)); return 0; }
    duckdb_appender_close(ap); duckdb_appender_destroy(&ap);
    duckdb_result r2; duckdb_query(con, "SELECT c::VARCHAR FROM rt", &r2);
    char *x = duckdb_value_varchar(&r2, 0, 0);
    printf("round trip: %s\n", x ? x : "NULL");
    return 0;
}
```

Observed, `item34 bignum`. v1.4.4 to v1.5.4, identical apart from the
version line, exit status 139 (SIGSEGV while appending):

```text
Built with DuckDB v1.5.4, SELECT 123456789012345678901234567890::BIGNUM
schema format z, array n_buffers 4
importing
import: ok
exit=139
```

v1.5.5:

```text
Built with DuckDB v1.5.5, SELECT 123456789012345678901234567890::BIGNUM
schema format vz, array n_buffers 4
importing
import: ok
round trip: 123456789012345678901234567890
exit=0
```

`item34 geometry`, run before the round-trip lines were added (so it stops
at the import): v1.5.0 to v1.5.4 declare `z` with four buffers and the import
fails with `Invalid Input: Unsupported geometry type in WKB`; v1.5.5 declares
`vz` and imports; 1.4.x has no `GEOMETRY` type.

Expected: the declared format matching the buffers written.

quack-rs mitigation: after the export, `data_chunk_to_arrow` compares every
plain binary or UTF-8 node's buffer count with the schema `DuckDB` declares
for the chunk's types and refuses the array on a mismatch
(`src/arrow/export_check.rs`; `tests/ffi_roundtrip/arrow_export.rs`, which
failed on a 1.5.4 engine before the check).

---

## 35. Window frames with `EXCLUDE` never destroy one aggregate state per row

A window frame with an `EXCLUDE` clause is evaluated by the segment tree in
two parts. `WindowSegmentTreeLocalState::Evaluate` (`window_segment_tree.cpp`)
evaluates the right-hand part per chunk: `right_part->Evaluate` runs
`Initialize(count)`, which calls `state_init` on `count` states, and the
part is then combined into the left one. Only `Finalize` destroys states, and
it runs for the left part alone; `~WindowSegmentTreePart` is empty. So each
chunk initialises one right-hand state per row that nothing destroys. A C API
aggregate with `EXCLUDE CURRENT ROW`, `GROUP` or `TIES` takes this path
(`WindowConstantAggregator::CanAggregate` refuses exclusions, and the C API
has no custom window callback).

Environment: prebuilt `libduckdb` v1.4.4, v1.4.5 and v1.5.0 to v1.5.5, x86_64
Linux. Build: `gcc -I<libduckdb dir> item35.c -L<libduckdb dir> -lduckdb -o
item35`, then run with `LD_LIBRARY_PATH=<libduckdb dir>`. The counters are
atomic because `DuckDB` calls the callbacks from several threads.

```c
// A C API aggregate in a sliding window, with and without EXCLUDE: count the
// states state_init created and the destructor destroyed.
// Build: gcc -I<libduckdb dir> item35.c -L<libduckdb dir> -lduckdb -o item35
#include <stdio.h>
#include <stdint.h>
#include <duckdb.h>
static long long inits, destroys;
static idx_t state_size(duckdb_function_info info) { (void)info; return 8; }
static void state_init(duckdb_function_info info, duckdb_aggregate_state s) { (void)info; *(int64_t *)s = 0; __atomic_fetch_add(&inits, 1, __ATOMIC_SEQ_CST); }
static void update(duckdb_function_info info, duckdb_data_chunk input, duckdb_aggregate_state *states) {
    (void)info;
    idx_t n = duckdb_data_chunk_get_size(input);
    int64_t *v = (int64_t *)duckdb_vector_get_data(duckdb_data_chunk_get_vector(input, 0));
    for (idx_t i = 0; i < n; i++) *(int64_t *)states[i] += v[i];
}
static void combine(duckdb_function_info info, duckdb_aggregate_state *src, duckdb_aggregate_state *dst, idx_t n) {
    (void)info;
    for (idx_t i = 0; i < n; i++) *(int64_t *)dst[i] += *(int64_t *)src[i];
}
static void finalize(duckdb_function_info info, duckdb_aggregate_state *src, duckdb_vector out, idx_t n, idx_t off) {
    (void)info;
    int64_t *d = (int64_t *)duckdb_vector_get_data(out);
    for (idx_t i = 0; i < n; i++) d[off + i] = *(int64_t *)src[i];
}
static void destroy(duckdb_aggregate_state *states, idx_t n) { (void)states; __atomic_fetch_add(&destroys, (long long)n, __ATOMIC_SEQ_CST); }

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    printf("Built with DuckDB %s\n", duckdb_library_version());
    duckdb_database db; duckdb_connection con; duckdb_open(NULL, &db); duckdb_connect(db, &con);
    duckdb_aggregate_function f = duckdb_create_aggregate_function();
    duckdb_aggregate_function_set_name(f, "counted_sum");
    duckdb_logical_type t = duckdb_create_logical_type(DUCKDB_TYPE_BIGINT);
    duckdb_aggregate_function_add_parameter(f, t);
    duckdb_aggregate_function_set_return_type(f, t);
    duckdb_aggregate_function_set_functions(f, state_size, state_init, update, combine, finalize);
    duckdb_aggregate_function_set_destructor(f, destroy);
    if (duckdb_register_aggregate_function(con, f) != DuckDBSuccess) { printf("register failed\n"); return 1; }
    const char *frames[] = {"", "EXCLUDE CURRENT ROW", "EXCLUDE GROUP", "EXCLUDE TIES"};
    for (int k = 0; k < 4; k++) {
        char sql[512];
        snprintf(sql, sizeof sql,
                 "SELECT sum(x) FROM (SELECT counted_sum(i) OVER (ORDER BY i ROWS BETWEEN 2 PRECEDING AND "
                 "CURRENT ROW %s) AS x FROM range(5000) t(i))", frames[k]);
        inits = destroys = 0;
        duckdb_result r;
        if (duckdb_query(con, sql, &r) != DuckDBSuccess) { printf("%s: %s\n", frames[k], duckdb_result_error(&r)); continue; }
        duckdb_destroy_result(&r);
        duckdb_query(con, "SELECT 1", NULL);
        printf("frame %-20s: %lld states initialised, %lld destroyed\n", frames[k][0] ? frames[k] : "(none)", inits, destroys);
    }
    duckdb_disconnect(&con); duckdb_close(&db);
    printf("after close: %lld initialised, %lld destroyed\n", inits, destroys);
    return 0;
}
```

Observed, identical on all eight releases apart from the version line
(`md5sum`):

```text
Built with DuckDB v1.5.5
frame (none)              : 5336 states initialised, 5336 destroyed
frame EXCLUDE CURRENT ROW : 10336 states initialised, 5336 destroyed
frame EXCLUDE GROUP       : 10336 states initialised, 5336 destroyed
frame EXCLUDE TIES        : 10336 states initialised, 5336 destroyed
after close: 10336 initialised, 5336 destroyed
```

Expected: as many states destroyed as initialised, as without `EXCLUDE`.

quack-rs mitigation: none possible in the callbacks. A slot `state_init`
receives again cannot be told from reused memory that holds a stale copy of
a state `DuckDB` moved, so it cannot be dropped there safely. `FfiState<T>`
stores a small `T` inline, so only what `T` owns on the heap leaks; the
limitation is documented on `DestroyFn` and `FfiState` and in the book's
known limitations, and `tests/ffi_roundtrip/agg_states.rs` counts the `T`s
never dropped (5000 of a 5000-row window with `EXCLUDE CURRENT ROW` on
1.5.5, none without).
