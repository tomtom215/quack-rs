# Vector Operations

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

The `quack_rs::vector::ops` module works on whole vectors at once. With it you
can:

- allocate a vector of your own (`OwnedVector`);
- copy the rows a [selection vector](selection-vector.md) names into another
  vector (`copy_selected`);
- filter or reorder a vector in place without copying its data (`slice`);
- fill a vector with one constant value (`reference_value`);
- make one vector share another's data (`reference_vector`).

Start with `copy_selected`. It is the only one of these operations whose result
quack-rs's `VectorReader` can read directly.

## Allocating a vector: `OwnedVector`

DuckDB normally hands your callbacks the vectors they read and write.
`OwnedVector` is for when you need one of your own: a destination for
`copy_selected`, a staging buffer, or a vector to pass to
`Appender::append_chunk`. It is flat, and DuckDB destroys it when the value is
dropped.

```rust,no_run
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::OwnedVector;

# fn demo() -> Result<(), quack_rs::error::ExtensionError> {
let v = OwnedVector::new(&LogicalType::new(TypeId::BigInt), 2048)?;
let raw = v.as_raw(); // borrow the duckdb_vector; `v` still owns it
# let _ = raw;
# Ok(())
# }
```

`OwnedVector::new` returns an error if DuckDB cannot back a vector with the
type (`ANY`, `INVALID`), or if the capacity is above `ops::MAX_CAPACITY`. An
`ARRAY(T, n)` column counts as `capacity * n` elements against that limit.
`into_raw()` gives up ownership; you must then call `duckdb_destroy_vector`
yourself.

## Copying selected rows: `copy_selected`

`copy_selected(src, dst, &sel, src_count, src_offset, dst_offset)` copies the
rows `sel` names from `src` into `dst`, in the selection's order. `dst` stays
flat, so you can read it with an ordinary `VectorReader`. To copy the whole
selection, pass `src_count = sel.len()` and `src_offset = 0`.

```rust
use quack_rs::selection_vector::SelectionVector;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::ops::copy_selected;
use quack_rs::vector::{OwnedVector, VectorReader, VectorWriter};

# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# fn demo() -> Result<(), quack_rs::error::ExtensionError> {
let ty = LogicalType::new(TypeId::BigInt);
let src = OwnedVector::new(&ty, 8)?;
let dst = OwnedVector::new(&ty, 8)?;

// Fill src with 0, 10, 20, ..., 70.
let mut writer = unsafe { VectorWriter::from_vector(src.as_raw()) };
for row in 0..8 {
    unsafe { writer.write_i64(row, row as i64 * 10) };
}

// Keep rows 6, 3 and 0, in that order.
let mut sel = SelectionVector::new(3)?;
sel.as_mut_slice().copy_from_slice(&[6, 3, 0]);

// SAFETY: same type, every index is a row of `src`, `dst` has room for 3 rows.
unsafe { copy_selected(src.as_raw(), dst.as_raw(), &sel, sel.len(), 0, 0) };

let reader = unsafe { VectorReader::from_vector(dst.as_raw(), 3) };
let kept: Vec<i64> = (0..3).map(|row| unsafe { reader.read_i64(row) }).collect();
assert_eq!(kept, [60, 30, 0]);
# Ok(())
# }
# demo().unwrap();
```

The caller must guarantee that `src` and `dst` have the same logical type, that
every selected index is a row of `src`, and that `dst` has room for
`dst_offset + (src_count - src_offset)` rows.

## Operations that make a vector non-flat

`slice`, `reference_value` and `reference_vector` copy no data. The price is
that the vector stops being **flat**, and quack-rs's `VectorReader` and
`VectorWriter` assume flat vectors. Reading a non-flat vector with them returns
the wrong rows, and nothing reports it.

Use these operations when the vector goes back to DuckDB, which understands
every layout. When you need to read the result yourself, use `copy_selected`.

| Function | What it does | Afterwards the vector is |
|----------|--------------|--------------------------|
| `slice(vector, &sel, len)` | Row `i` becomes original row `sel[i]` | A dictionary vector |
| `reference_value(vector, &value)` | Every row becomes `value` | A constant vector |
| `reference_vector(to, from)` | `to` shares `from`'s data | Whatever layout `from` has |

All three are `unsafe`. In particular:

- **After any of them**, do not use a `VectorReader` or `VectorWriter` created
  over the vector before the call. Both cache the old data pointer.
- **`slice`** keeps reading through `sel`'s buffer. Do not change `sel` through
  `as_mut_slice()` while the vector is in use; dropping `sel` is fine.
- **`reference_value`** needs a `Value` of exactly the vector's logical type.
  A mismatched value can overflow the vector's buffers, and a value whose cast
  fails aborts the process. To fill a vector with `NULL`, use a typed `NULL` of
  the vector's own type.
- **`reference_vector`** requires both vectors to have the same type, and
  `from` must outlive every read of `to`.

```rust,no_run
use quack_rs::value::Value;
use quack_rs::vector::ops::reference_value;

# unsafe fn demo(output: libduckdb_sys::duckdb_vector) {
// Every row of this BIGINT output vector is 42.
let answer = Value::bigint(42);
unsafe { reference_value(output, &answer) };
# }
```

## Where flatness is already guaranteed

DuckDB flattens the vectors it hands an extension: scalar function inputs,
aggregate `update` inputs, cast inputs and copy-function sink chunks. Query
results from `duckdb_fetch_chunk` are flat too. So `VectorReader` is correct in
those places. Inside your own code, a vector becomes non-flat only through
`slice`, `reference_value`, or `reference_vector` from a non-flat source.

## See also

- [Selection Vectors](selection-vector.md): building the `SelectionVector` these functions take
- [Reading & Writing Vectors](../data/vectors.md): `VectorReader` and `VectorWriter`
- [`vector::ops`](https://docs.rs/quack-rs/latest/quack_rs/vector/ops/index.html) module documentation
