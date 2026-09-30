# Complex Types: STRUCT, LIST, MAP, ARRAY

DuckDB stores its nested types — `STRUCT`, `LIST`, `MAP` and `ARRAY` — as a parent
vector with one or more child vectors. This page shows how a quack-rs extension reads
and writes them: the four helper types in [`vector::complex`] reach the child vectors,
and `ListBuilder` writes `LIST` and `MAP` output without manual offset arithmetic.

## Overview

| DuckDB type | Storage | quack-rs helper |
|-------------|---------|-----------------|
| `STRUCT{a T, b U, …}` | Parent vector + N child vectors (one per field) | `StructVector` |
| `LIST<T>` | Parent vector holds `{offset, length}` per row; flat child vector holds elements | `ListVector` |
| `MAP<K, V>` | Stored as `LIST<STRUCT{key K, value V}>` | `MapVector` |
| `ARRAY<T>[N]` | Fixed-size array; single child vector | `ArrayVector` |

## Reading complex types (input vectors)

### STRUCT

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(parent_vec: duckdb_vector, row_count: usize) {
use quack_rs::vector::{VectorReader, complex::StructVector};

// Inside a scalar function or aggregate update callback:
// parent_vec comes from duckdb_data_chunk_get_vector(chunk, col_idx)
let x_reader = unsafe { StructVector::field_reader(parent_vec, 0, row_count) };
let y_reader = unsafe { StructVector::field_reader(parent_vec, 1, row_count) };

for row in 0..row_count {
    // Each field has its own validity bitmap.
    if unsafe { x_reader.is_valid(row) && y_reader.is_valid(row) } {
        let x: f64 = unsafe { x_reader.read_f64(row) };
        let y: f64 = unsafe { y_reader.read_f64(row) };
        // process (x, y) …
    }
}
# }
```

### LIST

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(list_vec: duckdb_vector, row_count: usize) {
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
# }
```

### MAP

`MAP` is `LIST<STRUCT{key, value}>`. `MapVector::key_reader` and `value_reader`
read the two fields of the inner struct:

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(map_vec: duckdb_vector, row_count: usize) {
use quack_rs::vector::complex::MapVector;

let total = unsafe { MapVector::total_entry_count(map_vec) };
let key_reader   = unsafe { MapVector::key_reader(map_vec, total) };
let value_reader = unsafe { MapVector::value_reader(map_vec, total) };

for row in 0..row_count {
    let entry = unsafe { MapVector::get_entry(map_vec, row) };
    for i in 0..entry.length as usize {
        let idx = entry.offset as usize + i;
        let k = unsafe { key_reader.read_str(idx) };   // MAP keys are never NULL
        if unsafe { value_reader.is_valid(idx) } {
            let v: i64 = unsafe { value_reader.read_i64(idx) };
            // process (k, v) …
        }
    }
}
# }
```

## Writing complex types (output vectors)

### STRUCT

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(out_vec: duckdb_vector, batch_size: usize, x_values: &[f64], y_values: &[f64]) {
use quack_rs::vector::{VectorWriter, complex::StructVector};

let mut x_writer = unsafe { StructVector::field_writer(out_vec, 0) };
let mut y_writer = unsafe { StructVector::field_writer(out_vec, 1) };

for row in 0..batch_size {
    unsafe { x_writer.write_f64(row, x_values[row]) };
    unsafe { y_writer.write_f64(row, y_values[row]) };
}
# }
```

### Nested complex types inside STRUCT (v0.11.0+)

When a STRUCT field is itself a LIST, MAP or ARRAY, `child_vector(field_idx)` on
`StructWriter` or `StructReader` returns the field's raw vector handle, which the
`ListVector`, `MapVector` and `ArrayVector` helpers and `ListBuilder` accept:

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(struct_vec: duckdb_vector, row: usize) {
use quack_rs::vector::{ListBuilder, StructWriter};

// STRUCT(name VARCHAR, services VARCHAR[], message VARCHAR)
let mut sw = unsafe { StructWriter::new(struct_vec, 3) };

// Write scalar fields normally
unsafe { sw.write_varchar(row, 0, "hello") };
unsafe { sw.write_varchar(row, 2, "ok") };

// The LIST field at index 1: ListBuilder appends after any elements
// earlier rows already wrote to the child vector.
let services = ["a", "b", "c"];
let mut builder = unsafe { ListBuilder::new(sw.child_vector(1)) };
unsafe {
    builder.push_row(row, services.len(), |writer, base| {
        for (i, s) in services.iter().enumerate() {
            writer.write_varchar(base + i, s);
        }
    });
    builder.finish();
}
# }
```

### LIST — recommended: `ListBuilder`

`ListBuilder` tracks the running offset, writes each parent row's
`{offset, length}` entry, and — importantly — re-fetches the child writer after
every reserve:

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(list_vec: duckdb_vector, rows: &[Vec<i64>]) {
use quack_rs::vector::ListBuilder;

let mut builder = unsafe { ListBuilder::new(list_vec) };
for (row, elements) in rows.iter().enumerate() {
    unsafe {
        builder.push_row(row, elements.len(), |writer, base| {
            for (i, &val) in elements.iter().enumerate() {
                writer.write_i64(base + i, val);
            }
        });
    }
}
unsafe { builder.finish() };
# }
```

> **Why the re-fetch matters.** `duckdb_list_vector_reserve` takes a *total*
> capacity, and when it grows it reallocates the child vector's data buffer. A
> `VectorWriter` obtained before that call is left holding a dangling pointer.
> The manual pattern below is safe only because it reserves exactly once, before
> any writer exists — which requires knowing the total element count up front.
> `ListBuilder` has no such requirement. The same applies to writers on anything
> below the child, such as the fields of a `LIST` of `STRUCT`s: fetch them again
> after every reserve that grows the list (see
> [Pitfall L18](../reference/pitfalls.md#l18-a-list-reserve-moves-every-buffer-below-its-child)).

`push_map_row` does the same for `MAP`, handing the closure a writer for the key
child and one for the value child.

DuckDB limits a child vector to 2^37 bytes per buffer
(`MAX_LIST_CHILD_CAPACITY`), and a reservation above that — or one the
allocator cannot satisfy — throws a C++ exception through the C API, which
aborts the process. `vector::max_child_capacity(vec)` turns the byte limit into
an element count for the child's type: 2^34 `BIGINT`s, 2^33 `VARCHAR`s.
`ListBuilder` applies it by itself. When row lengths come from untrusted input,
also set a limit that fits in memory with `with_element_limit(n)`: a row that
would exceed the limit is written as NULL instead, and `overflowed()` reports
that it happened.

### LIST — manual

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(list_vec: duckdb_vector, rows: &[Vec<i64>]) {
use quack_rs::vector::{VectorWriter, complex::ListVector};

let total_elements: usize = rows.iter().map(|r| r.len()).sum();
// Must not exceed quack_rs::vector::max_child_capacity(list_vec); see above.
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
# }
```

### MAP — manual

Writing a MAP follows the LIST pattern, but keys and values go into the two
fields of the inner STRUCT vector. Prefer `ListBuilder::push_map_row` unless you
know the total pair count before writing:

```rust
# use libduckdb_sys::duckdb_vector;
# fn demo(map_vec: duckdb_vector, total_pairs: usize, all_pairs: &[Vec<(String, i64)>]) {
use quack_rs::vector::complex::MapVector;

unsafe { MapVector::reserve(map_vec, total_pairs) };

let mut key_writer = unsafe { MapVector::key_writer(map_vec) };
let mut val_writer = unsafe { MapVector::value_writer(map_vec) };
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
# }
```

## Constructing complex logical types

Use `LogicalType` constructors to define complex column types. Each constructor
has a variant that accepts `TypeId` values (for simple element types) and a
`_from_logical` variant (for nested complex types):

| Constructor | `_from_logical` variant | Creates |
|-------------|------------------------|---------|
| `LogicalType::list(TypeId)` | `list_from_logical(&LogicalType)` | `LIST<T>` |
| `LogicalType::map(TypeId, TypeId)` | `map_from_logical(&LogicalType, &LogicalType)` | `MAP<K, V>` |
| `LogicalType::struct_type(&[(&str, TypeId)])` | `struct_type_from_logical(&[(&str, LogicalType)])` | `STRUCT{...}` |
| `LogicalType::union_type(&[(&str, TypeId)])` | `union_type_from_logical(&[(&str, LogicalType)])` | `UNION(...)` |
| `LogicalType::array(TypeId, u64)` | `array_from_logical(&LogicalType, u64)` | `ARRAY<T>[N]` |
| `LogicalType::enum_type(&[&str])` | — | `ENUM(...)` |
| `LogicalType::decimal(u8, u8)` | — | `DECIMAL(w, s)` |

Each constructor also has a `try_` form (`try_list`, `try_struct_type_from_logical`, …)
that returns `Result<LogicalType, LogicalTypeError>`; the plain forms panic where
the `try_` form returns an error. Errors include a composite `TypeId` passed where
a `_from_logical` variant is needed, `STRUCT` field or `UNION` member names that
are equal ignoring ASCII case, and a `UNION` with more than `MAX_UNION_MEMBERS`
(255) members.

## API reference

All helpers are in `quack_rs::vector::complex` (re-exported from `quack_rs::prelude`).

### `StructVector`

| Method | Description |
|--------|-------------|
| `get_child(vec, field_idx)` | Returns the raw child vector for field `field_idx` |
| `field_reader(vec, field_idx, row_count)` | Creates a `VectorReader` for a STRUCT field |
| `field_writer(vec, field_idx)` | Creates a `VectorWriter` for a STRUCT field |

### `StructWriter` / `StructReader` complex field access (v0.11.0+)

| Method | Description |
|--------|-------------|
| `StructWriter::child_vector(field_idx)` | Returns the raw `duckdb_vector` of a nested field (LIST, MAP, ARRAY) |
| `StructWriter::child_list_vector(field_idx)` | Alias of `child_vector` for a LIST field |
| `StructReader::child_vector(field_idx)` | Same as `StructWriter::child_vector`, for reading (`unsafe`) |

### `ListVector`

| Method | Description |
|--------|-------------|
| `get_child(vec)` | Returns the flat element child vector |
| `get_size(vec)` | Total number of elements across all rows |
| `set_size(vec, n)` | Sets the number of elements after writing |
| `reserve(vec, capacity)` | Reserves capacity in the child vector (at most `max_child_capacity(vec)`) |
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
| `reserve(vec, n)` | Reserves capacity for `n` pairs (at most `max_child_capacity(vec)`) |
| `set_size(vec, n)` | Sets total entry count after writing |
| `get_entry(vec, row)` | Returns `{offset, length}` for a row (reading) |
| `set_entry(vec, row, offset, length)` | Sets `{offset, length}` for a row (writing) |
| `key_reader(vec, count)` / `value_reader(vec, count)` | Creates a `VectorReader` for the keys / values |
| `key_writer(vec)` / `value_writer(vec)` | Creates a `VectorWriter` for the keys / values |

### `ArrayVector`

| Method | Description |
|--------|-------------|
| `get_child(vec)` | Returns the child vector of a fixed-size ARRAY vector |

[`vector::complex`]: https://docs.rs/quack-rs/latest/quack_rs/vector/complex/index.html
