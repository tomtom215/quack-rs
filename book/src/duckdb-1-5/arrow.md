# Arrow Interop

> **Requires the `duckdb-1-5-4` feature flag.**

DuckDB 1.5.0 added a conversion family that moves data straight between a
`duckdb_data_chunk` and the
[Arrow C Data Interface](https://arrow.apache.org/docs/format/CDataInterface.html),
without a query result in between. `quack_rs::arrow` wraps all of it.

## No `arrow` crate dependency

The Arrow C Data Interface is an **ABI**, not a library: `ArrowSchema` and
`ArrowArray` are plain `#[repr(C)]` records with a `release` callback.
`libduckdb-sys` defines them directly — and asserts in its own test suite that
they match arrow-rs's `FFI_ArrowSchema` / `FFI_ArrowArray` field for field — so
`quack-rs` speaks Arrow without pulling in the `arrow` crate, and an extension
that *does* use arrow-rs bridges across with a pointer cast.

## Why the feature is `duckdb-1-5-4` and not `duckdb-1-5`

All eight C functions are in `duckdb_ext_api_v1` from DuckDB **1.5.0** — that
was checked against the v1.5.0 `duckdb_extension.h`, not assumed. The floor
comes from the bindings: `libduckdb-sys` declared both records as *opaque
zero-sized* bindgen placeholders (`_unused: [u8; 0]`) until **1.10504.0**, and
you cannot allocate the caller-owned structs these APIs need out of a
zero-sized type. `src/arrow.rs` carries a `const` assertion that says exactly
that if you build against an older binding.

## The types

| Type | Wraps | Freed by |
|---|---|---|
| `ArrowOptions` | `duckdb_arrow_options` | `duckdb_destroy_arrow_options` |
| `ArrowSchema` | the `ArrowSchema` ABI record | `release(schema)` |
| `ArrowArray` | the `ArrowArray` ABI record | `release(array)` |
| `ArrowConvertedSchema` | `duckdb_arrow_converted_schema` | `duckdb_destroy_arrow_converted_schema` |

`RawArrowSchema` and `RawArrowArray` are the ABI records themselves, re-exported
for code that already speaks the raw interface.

## Exporting a chunk

`ArrowOptions` carries the settings DuckDB renders Arrow with — the timezone for
`TIMESTAMPTZ`, the string offset width, registered extension types. Take them
from the *result* whose chunks you are exporting, so schema and data agree:

```rust,no_run
use quack_rs::arrow::{data_chunk_to_arrow, to_arrow_schema};
use quack_rs::query::QueryResult;

# fn demo(result: &mut QueryResult) -> Result<(), Box<dyn std::error::Error>> {
// SAFETY: the connection that ran this query stays open while `options` is
// in use — the options point at that connection's client context.
let options = unsafe { result.arrow_options() }?;

let columns: Vec<(String, quack_rs::types::LogicalType)> = (0..result.column_count())
    .filter_map(|i| Some((result.column_name(i)?, result.column_logical_type(i)?)))
    .collect();
let pairs: Vec<(&str, &quack_rs::types::LogicalType)> =
    columns.iter().map(|(n, t)| (n.as_str(), t)).collect();

let schema = to_arrow_schema(&options, &pairs)?;
assert_eq!(schema.format(), Some("+s")); // a record batch is a struct

while let Some(chunk) = result.next_chunk()? {
    let array = data_chunk_to_arrow(&options, &chunk)?;
    // hand `array` (plus `schema`) to any Arrow consumer
    let _ = array;
}
# Ok(())
# }
```

### `ArrowOptions` must not outlive its connection

The options hold a raw pointer to the connection's client context, and the
conversion functions dereference it. Used after the connection is closed they
read freed memory. `ArrowOptions<'conn>` carries that lifetime:

- `ArrowOptions::from_connection(&con)` is safe. It borrows the
  `OwnedConnection`, so the compiler rejects a `drop(con)` while the options
  are still in use.
- `ArrowOptions::from_raw_connection(raw)` (for a raw `duckdb_connection`) and
  `QueryResult::arrow_options()` / `ArrowOptions::from_result` are `unsafe`. A
  `QueryResult` does not borrow the connection that ran it, so nothing checks
  that the connection is still open. The caller must keep it open for as long
  as the options are used.

## Importing an array

Going the other way needs the Arrow schema translated into DuckDB's own type
descriptors first. That translation is reusable — do it once, not per batch:

```rust,no_run
use quack_rs::arrow::{data_chunk_from_arrow, schema_from_arrow, ArrowArray, ArrowSchema};

# fn demo(
#     con: libduckdb_sys::duckdb_connection,
#     schema: &mut ArrowSchema,
#     array: ArrowArray,
# ) -> Result<(), Box<dyn std::error::Error>> {
// SAFETY: `con` is a live DuckDB connection.
let converted = unsafe { schema_from_arrow(con, schema) }?;
// SAFETY: same connection; `array` was built against `schema`.
let chunk = unsafe { data_chunk_from_arrow(con, array, &converted) }?;
let _ = chunk.size();
# Ok(())
# }
```

Note the asymmetry, which mirrors what DuckDB actually does:

- `schema_from_arrow` **borrows** the schema. You still own it and it is released
  when its `ArrowSchema` drops.
- `data_chunk_from_arrow` **takes** the array by value. DuckDB sets
  `arrow_array->release = nullptr` before the conversion loop body — so it claims
  the array even when the conversion then fails. The by-value binding is still
  dropped on the way out, which releases the array in the one case where DuckDB
  does *not* claim it (a zero-column schema, where the loop never runs).

The resulting chunk keeps the Arrow buffers alive, so the data is shared rather
than copied.

## What the wrapper refuses that DuckDB would not

`duckdb_data_chunk_from_arrow` indexes `arrow_array->children[i]` once per column
in the converted schema with no bounds check, dereferences each child without a
null check, reads `offset + length` rows from each child without comparing its
length, and dereferences an array without checking whether it was already
released. Those are segfaults or out-of-bounds reads, not errors.
`data_chunk_from_arrow` checks them first — which is why `ArrowConvertedSchema`
remembers the column count of the schema it was built from — and returns an
`InvalidInput` error instead.

What it cannot check, and what `data_chunk_from_arrow`'s `# Safety` section
therefore makes the caller's job:

- **The array must conform to the converted schema.** Nothing in an Arrow
  array records its type, so DuckDB reads each child's buffers as the format
  the *schema* declares. An `int32` child imported under a `utf8` schema has
  its values read as string offsets into a buffer that does not exist. Arrays
  exported with `data_chunk_to_arrow` under the schema you converted conform.
- **The buffers must be as long as the lengths say**, and `length` must be
  the true row count. DuckDB allocates the chunk for `length` rows before its
  error handling starts, so an absurd length is an allocation failure that
  aborts the process.

It also refuses valid Arrow layouts that DuckDB imports wrongly: it walks the
array alongside its schema and returns `InvalidInput`, naming the node, for

- an offset below the top level that DuckDB applies to the wrong rows: a
  struct inside an offset struct or a list, a union's members, a run-end-encoded
  array's value validity (`docs/upstream-duckdb-reports.md`, item 24);
- a dictionary with NULLs under a list that starts past element 0, or with more
  than 2048 rows and NULLs of its own or an enclosing struct's, which DuckDB
  copies past a 2048-row heap mask (items 9 and 25);
- a dictionary whose values are themselves dictionary-encoded (item 26);
- list views that overlap or leave gaps (item 27);
- a sparse union whose `+us:` type codes are not `0, 1, …` (item 28);
- a run-end-encoded array where DuckDB reads a plain one: a fixed-size list's
  child, or another run-end array's values (item 24).

Arrays that `data_chunk_to_arrow` produced, paired with the schema they were
produced with, never take these shapes. Arrays from other producers can: one
that slices a nested array without copying it may leave offsets below the top
level. Copying the slice before export avoids them. Every error DuckDB
reports from the conversion arrives as `InvalidInput`.

## Round trips are not always exact

Two types come back different from an Arrow round trip through DuckDB's own
converters:

- `TIMETZ` comes back as `TIME` with the offset dropped:
  `01:02:03+05:30` returns as `01:02:03`.
- `BIT` comes back as `BLOB`.

Check the converted types (`ArrowConvertedSchema`) when a round trip must be
lossless.

`DuckDB` would export three kinds of value wrongly, with no error (checked on
1.4.4, 1.5.0 and 1.5.5), so `data_chunk_to_arrow` checks the chunk first and
refuses one that holds such a value, at any nesting depth:

- An `INTERVAL` whose microseconds exceed about ±106,751 days (2,562,047
  hours) would wrap, because Arrow counts nanoseconds in an `i64` and DuckDB
  multiplies by 1000 unchecked: `INTERVAL 2562048 HOUR` would export as a
  negative interval.
- A `UHUGEINT` of 2^127 or more would export as a negative
  `decimal128(38, 0)` (`2^128 - 1` becomes `-1`).
- A 39-digit `HUGEINT` would export as a `decimal128(38, 0)` it does not fit,
  unless `arrow_lossless_conversion` is set (then it exports as a 16-byte
  fixed-size binary and is not refused).

## Bridging to arrow-rs

This sketch uses the `arrow` crate's `FFI_ArrowArray`, which quack-rs does not
depend on, so it is not compiled with the book:

```rust,ignore
// quack-rs -> arrow-rs
let ffi: FFI_ArrowArray = unsafe { std::mem::transmute(array.into_raw()) };

// arrow-rs -> quack-rs, neutralising the source so only one side releases
let array = unsafe { ArrowArray::take_from(std::ptr::from_mut(&mut ffi).cast()) };
```

`take_from` moves the record out and writes a released placeholder back, so the
foreign wrapper's own `Drop` becomes a no-op instead of a double free.

## Thread safety

None of these types are `Send` or `Sync`. The Arrow C Data Interface says nothing
about which thread may call `release`, and `duckdb_arrow_options` wraps a
`ClientProperties` tied to the connection's client context.
