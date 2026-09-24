<!-- SPDX-License-Identifier: MIT -->

# Upstream DuckDB reports (pending)

Working notes for reports to the DuckDB project about C API behaviour found
while auditing quack-rs (see `AUDIT.md`, section 8.5). None has been filed.
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

## Before filing

- Search the DuckDB issue tracker for existing reports of each item; this was
  not done when the notes were written.
- Re-run each reproducer against the current DuckDB release. The notes record
  DuckDB v1.5.5 behaviour, and a source reading of DuckDB `main` at
  `30c64e17` (2026-09-23), where the C API files moved to
  `src/main/capi/v1/`; `main` was not built.
- Item 6 has no plain-C reproducer yet.

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
