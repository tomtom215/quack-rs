# Complex Types: STRUCT, LIST, MAP

DuckDB's complex types — `STRUCT`, `LIST`, and `MAP` — are stored as nested vectors.
`quack-rs` provides three helper types in [`vector::complex`] to access the child
vectors without manual offset arithmetic.

## Overview

| DuckDB type | Storage | quack-rs helper |
|-------------|---------|-----------------|
| `STRUCT{a T, b U, …}` | Parent vector + N child vectors (one per field) | `StructVector` |
| `LIST<T>` | Parent vector holds `{offset, length}` per row; flat child vector holds elements | `ListVector` |
| `MAP<K, V>` | Stored as `LIST<STRUCT{key K, value V}>` | `MapVector` |

## Reading complex types (input vectors)

### STRUCT

```rust
use quack_rs::vector::{VectorReader, complex::StructVector};

// Inside a scan or finalize callback:
// parent_vec comes from duckdb_data_chunk_get_vector(chunk, col_idx)
let x_reader = unsafe { StructVector::field_reader(parent_vec, 0, row_count) };
let y_reader = unsafe { StructVector::field_reader(parent_vec, 1, row_count) };

for row in 0..row_count {
    if unsafe { x_reader.is_valid(row) } {
        let x: f64 = unsafe { x_reader.read_f64(row) };
        let y: f64 = unsafe { y_reader.read_f64(row) };
        // process (x, y) …
    }
}
```

### LIST

```rust
use quack_rs::vector::{VectorReader, complex::ListVector};

let total_elements = unsafe { ListVector::get_size(list_vec) };
let elem_reader = unsafe { ListVector::child_reader(list_vec, total_elements) };

for row in 0..row_count {
    let entry = unsafe { ListVector::get_entry(list_vec, row) };
    for i in 0..entry.length as usize {
        let elem_idx = entry.offset as usize + i;
        if unsafe { elem_reader.is_valid(elem_idx) } {
            let val: i64 = unsafe { elem_reader.read_i64(elem_idx) };
            // process val …
        }
    }
}
```

### MAP

`MAP` is `LIST<STRUCT{key, value}>`. Access keys and values via the inner struct:

```rust
use quack_rs::vector::{VectorReader, complex::MapVector};

let total = unsafe { MapVector::total_entry_count(map_vec) };
let key_reader   = unsafe { VectorReader::from_vector(MapVector::keys(map_vec), total) };
let value_reader = unsafe { VectorReader::from_vector(MapVector::values(map_vec), total) };

for row in 0..row_count {
    let entry = unsafe { MapVector::get_entry(map_vec, row) };
    for i in 0..entry.length as usize {
        let idx = entry.offset as usize + i;
        let k = unsafe { key_reader.read_str(idx) };
        let v: i64 = unsafe { value_reader.read_i64(idx) };
        // process (k, v) …
    }
}
```

## Writing complex types (output vectors)

### STRUCT

```rust
use quack_rs::vector::{VectorWriter, complex::StructVector};

let mut x_writer = unsafe { StructVector::field_writer(out_vec, 0) };
let mut y_writer = unsafe { StructVector::field_writer(out_vec, 1) };

for row in 0..batch_size {
    unsafe { x_writer.write_f64(row, x_values[row]) };
    unsafe { y_writer.write_f64(row, y_values[row]) };
}
```

### LIST

```rust
use quack_rs::vector::{VectorWriter, complex::ListVector};

let total_elements: usize = rows.iter().map(|r| r.len()).sum();
unsafe { ListVector::reserve(list_vec, total_elements) };

let mut child_writer = unsafe { ListVector::child_writer(list_vec) };
let mut offset = 0usize;
for (row, elements) in rows.iter().enumerate() {
    for (i, &val) in elements.iter().enumerate() {
        unsafe { child_writer.write_i64(offset + i, val) };
    }
    unsafe { ListVector::set_entry(list_vec, row, offset as u64, elements.len() as u64) };
    offset += elements.len();
}
unsafe { ListVector::set_size(list_vec, total_elements) };
```

### MAP

The MAP write workflow is identical to LIST, but keys and values are written into
the two struct child vectors:

```rust
use quack_rs::vector::{VectorWriter, complex::MapVector};

unsafe { MapVector::reserve(map_vec, total_pairs) };

let mut key_writer   = unsafe { VectorWriter::from_vector(MapVector::keys(map_vec)) };
let mut val_writer   = unsafe { VectorWriter::from_vector(MapVector::values(map_vec)) };
let mut offset = 0usize;
for (row, pairs) in all_pairs.iter().enumerate() {
    for (i, (k, v)) in pairs.iter().enumerate() {
        unsafe { key_writer.write_varchar(offset + i, k) };
        unsafe { val_writer.write_i64(offset + i, *v) };
    }
    unsafe { MapVector::set_entry(map_vec, row, offset as u64, pairs.len() as u64) };
    offset += pairs.len();
}
unsafe { MapVector::set_size(map_vec, total_pairs) };
```

## API reference

All helpers are in `quack_rs::vector::complex` (re-exported from `quack_rs::prelude`).

### `StructVector`

| Method | Description |
|--------|-------------|
| `get_child(vec, field_idx)` | Returns the raw child vector for field `field_idx` |
| `field_reader(vec, field_idx, row_count)` | Creates a `VectorReader` for a STRUCT field |
| `field_writer(vec, field_idx)` | Creates a `VectorWriter` for a STRUCT field |

### `ListVector`

| Method | Description |
|--------|-------------|
| `get_child(vec)` | Returns the flat element child vector |
| `get_size(vec)` | Total number of elements across all rows |
| `set_size(vec, n)` | Sets the number of elements after writing |
| `reserve(vec, capacity)` | Reserves capacity in the child vector |
| `get_entry(vec, row)` | Returns `{offset, length}` for a row (reading) |
| `set_entry(vec, row, offset, length)` | Sets `{offset, length}` for a row (writing) |
| `child_reader(vec, count)` | Creates a `VectorReader` for the element vector |
| `child_writer(vec)` | Creates a `VectorWriter` for the element vector |

### `MapVector`

| Method | Description |
|--------|-------------|
| `struct_child(vec)` | Returns the inner STRUCT vector |
| `keys(vec)` | Returns the key vector (STRUCT field 0) |
| `values(vec)` | Returns the value vector (STRUCT field 1) |
| `total_entry_count(vec)` | Total key-value pairs |
| `reserve(vec, n)` | Reserves capacity |
| `set_size(vec, n)` | Sets total entry count after writing |
| `get_entry(vec, row)` | Returns `{offset, length}` for a row (reading) |
| `set_entry(vec, row, offset, length)` | Sets `{offset, length}` for a row (writing) |

[`vector::complex`]: ../../src/vector/complex.rs
