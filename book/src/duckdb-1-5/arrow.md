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
let options = result.arrow_options()?;

let columns: Vec<(String, quack_rs::types::LogicalType)> = (0..result.column_count())
    .filter_map(|i| Some((result.column_name(i)?, result.column_logical_type(i)?)))
    .collect();
let pairs: Vec<(&str, &quack_rs::types::LogicalType)> =
    columns.iter().map(|(n, t)| (n.as_str(), t)).collect();

let schema = to_arrow_schema(&options, &pairs)?;
assert_eq!(schema.format(), Some("+s")); // a record batch is a struct

while let Some(chunk) = result.next_chunk() {
    let array = data_chunk_to_arrow(&options, &chunk)?;
    // hand `array` (plus `schema`) to any Arrow consumer
    let _ = array;
}
# Ok(())
# }
```

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
in the converted schema with no bounds check, and dereferences an array without
checking whether it was already released. Both are segfaults, not errors.
`data_chunk_from_arrow` checks them first — which is why `ArrowConvertedSchema`
remembers the column count of the schema it was built from.

One thing it still cannot check: whether each child array's *buffers* match the
type its schema declares. DuckDB reads `array.buffers[1]` for a primitive column
without testing `n_buffers` or the pointer, so a malformed producer is a null
dereference inside DuckDB. Arrays that came from `data_chunk_to_arrow`, from
arrow-rs, or from any other conforming Arrow implementation are fine.

## Bridging to arrow-rs

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
