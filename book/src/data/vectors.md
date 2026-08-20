# Reading & Writing Vectors

DuckDB passes data to and from your extension as **vectors** — columnar arrays of typed
values, with a separate NULL bitmap. `VectorReader` and `VectorWriter` provide safe,
typed access to these vectors.

---

## `VectorReader`

### Construction

```rust,ignore
// In a scalar function callback:
let reader = unsafe { VectorReader::new(input, column_index) };

// In an aggregate update callback:
let reader = unsafe { VectorReader::new(input, 0) };   // first column
```

`VectorReader::new` takes the `duckdb_data_chunk` and a zero-based column index. The
reader borrows the chunk — it must not outlive the callback.

### Row count

```rust,ignore
let n = reader.row_count();   // number of rows in this chunk
```

Chunk sizes vary. Always loop from `0..reader.row_count()`, never assume a fixed size.

### NULL check

```rust,ignore
if unsafe { !reader.is_valid(row) } {
    // row is NULL — skip or propagate NULL to output
    unsafe { writer.set_null(row) };
    continue;
}
```

**Always check `is_valid` before reading.** Reading from a NULL row returns garbage data.

### Reading values

```rust,ignore
let i: i8  = unsafe { reader.read_i8(row) };
let i: i16 = unsafe { reader.read_i16(row) };
let i: i32 = unsafe { reader.read_i32(row) };
let i: i64 = unsafe { reader.read_i64(row) };
let u: u8  = unsafe { reader.read_u8(row) };
let u: u16 = unsafe { reader.read_u16(row) };
let u: u32 = unsafe { reader.read_u32(row) };
let u: u64 = unsafe { reader.read_u64(row) };
let f: f32 = unsafe { reader.read_f32(row) };
let f: f64 = unsafe { reader.read_f64(row) };
let b: bool = unsafe { reader.read_bool(row) };   // safe: uses u8 != 0
let s: &str = unsafe { reader.read_str(row) };    // handles inline + pointer format
let iv = unsafe { reader.read_interval(row) };    // returns DuckInterval

// Temporal and binary types (v0.10.0+):
let d: i32 = unsafe { reader.read_date(row) };      // days since epoch
let ts: i64 = unsafe { reader.read_timestamp(row) }; // microseconds since epoch
let t: i64 = unsafe { reader.read_time(row) };       // microseconds since midnight
let blob: &[u8] = unsafe { reader.read_blob(row) };  // binary data
let uuid: u128 = unsafe { reader.read_uuid(row) };   // UUID's textual 128 bits
```

---

## `VectorWriter`

### Construction

```rust,ignore
// In a scalar function callback:
let mut writer = unsafe { VectorWriter::new(output) };

// In an aggregate finalize callback:
let mut writer = unsafe { VectorWriter::new(result) };
```

### Writing values

```rust,ignore
unsafe { writer.write_i8(row, value) };
unsafe { writer.write_i16(row, value) };
unsafe { writer.write_i32(row, value) };
unsafe { writer.write_i64(row, value) };
unsafe { writer.write_u8(row, value) };
unsafe { writer.write_u16(row, value) };
unsafe { writer.write_u32(row, value) };
unsafe { writer.write_u64(row, value) };
unsafe { writer.write_f32(row, value) };
unsafe { writer.write_f64(row, value) };
unsafe { writer.write_bool(row, value) };
unsafe { writer.write_varchar(row, s) };   // &str (also available as write_str)
unsafe { writer.write_str(row, s) };       // alias for write_varchar
unsafe { writer.write_interval(row, interval) };  // DuckInterval

// Temporal and binary types (v0.10.0+):
unsafe { writer.write_date(row, days_since_epoch) };
unsafe { writer.write_timestamp(row, micros_since_epoch) };
unsafe { writer.write_time(row, micros_since_midnight) };
unsafe { writer.write_blob(row, &bytes) };
unsafe { writer.write_uuid(row, uuid_bits) };        // UUID's textual 128 bits
```

### `UUID` is not stored as you'd expect

A `UUID` column is physically a `HUGEINT`, but the 128 bits in the vector are
**not** the bits you see in the text form: `DuckDB` flips the top bit so that
comparing the signed integers orders UUIDs the same way comparing their strings
does.

```text
SELECT '11111111-2222-3333-4444-555555555555'::UUID
  read_i128 (raw storage) : 0x91111111222233334444555555555555
  read_uuid (textual bits): 0x11111111222233334444555555555555
```

`read_uuid` / `write_uuid` apply the flip for you and speak in **textual bits**
(`u128`) — the same convention as `Value::uuid` / `Value::as_uuid` and every
Rust `Uuid` type. Reach for `read_i128` / `write_i128` only when you want the raw
storage, and use `quack_rs::vector::{uuid_from_storage, uuid_to_storage}` to
convert.

### Writing NULL

```rust,ignore
unsafe { writer.set_null(row) };
```

> **Pitfall L4**: `set_null` calls `duckdb_vector_ensure_validity_writable` automatically
> before accessing the validity bitmap. Calling `duckdb_vector_get_validity` without this
> prerequisite returns an uninitialized pointer → SEGFAULT. `VectorWriter::set_null` handles
> this correctly. See [Pitfall L4](../reference/pitfalls.md#l4-ensure_validity_writable-is-required-before-null-output).

### Clearing NULL (v0.11.0+)

To undo a previous `set_null` call and mark a row as valid again:

```rust,ignore
unsafe { writer.set_valid(row) };
```

`set_valid` also calls `ensure_validity_writable` automatically.

---

## `DataChunk`

`DataChunk` wraps a `duckdb_data_chunk` handle, providing ergonomic access to
vectors and metadata without raw FFI calls:

```rust
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
use quack_rs::data_chunk::DataChunk;

unsafe extern "C" fn my_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let mut writer = unsafe { chunk.writer(0) };    // VectorWriter for column 0
    unsafe { writer.write_i64(0, 42) };
    unsafe { chunk.set_size(1) };                   // set output row count
}
```

Methods:
- `size()` — current row count
- `set_size(n)` — set row count (0 = end of stream)
- `column_count()` — number of columns
- `vector(col)` — raw `duckdb_vector` handle
- `writer(col)` — `VectorWriter` for a column
- `reader(col)` — `VectorReader` for a column
- `struct_writer(col, field_count)` — `StructWriter` for a STRUCT output column
- `struct_reader(col, field_count)` — `StructReader` for a STRUCT input column
- `struct_field_reader(col, field)` — `VectorReader` for a specific STRUCT field
- `into_chunk_writer()` — convert to `ChunkWriter` with auto `set_size` on drop

---

## `StructWriter` / `StructReader`

For STRUCT columns with many fields, creating individual `VectorWriter`/`VectorReader`
instances for each field is verbose. `StructWriter` and `StructReader` pre-create all
field writers/readers at construction:

```rust,ignore
// Writing a 5-field STRUCT output:
let mut sw = unsafe { chunk.struct_writer(0, 5) };
unsafe {
    sw.write_bool(row, 0, result.success);
    sw.write_varchar(row, 1, &result.data);
    sw.write_i64(row, 2, result.count);
    sw.write_date(row, 3, result.day);
    sw.write_blob(row, 4, &result.payload);
}

// Reading a 3-field STRUCT input:
let sr = unsafe { chunk.struct_reader(0, 3) };
for row in 0..chunk.size() {
    let name = unsafe { sr.read_str(row, 0) };
    let age = unsafe { sr.read_i32(row, 1) };
    let active = unsafe { sr.read_bool(row, 2) };
}
```

---

## `ChunkWriter`

`ChunkWriter` wraps an output `duckdb_data_chunk` and tracks rows. It automatically
calls `set_size` on drop, preventing the common off-by-one bug:

```rust,ignore
let mut cw = unsafe { DataChunk::from_raw(output).into_chunk_writer() };
while let Some(row) = cw.next_row() {
    unsafe { cw.writer(0).write_varchar(row, &data[row].name) };
    unsafe { cw.writer(1).write_i64(row, data[row].value) };
    if cw.is_full() { break; }
}
// set_size called automatically when `cw` is dropped
```

---

## `ValidityBitmap`

For advanced NULL handling beyond `VectorWriter::set_null`, use `ValidityBitmap`
directly:

```rust,ignore
use quack_rs::vector::ValidityBitmap;

// Writing NULLs:
let mut bitmap = unsafe { ValidityBitmap::ensure_writable(some_vector) };
unsafe { bitmap.set_row_invalid(row as u64) };   // mark as NULL
unsafe { bitmap.set_row_valid(row as u64) };     // mark as non-NULL

// Reading NULLs:
let bitmap = unsafe { ValidityBitmap::get_read_only(some_vector) };
let is_valid = unsafe { bitmap.row_is_valid(row as u64) };
```

`ValidityBitmap` is available in the prelude: `use quack_rs::prelude::*`.

---

## Utility functions

The `quack_rs::vector` module provides two utility functions:

```rust,ignore
use quack_rs::vector::{vector_size, vector_get_column_type};

// Returns the default vector size used by DuckDB (typically 2048).
let size: u64 = vector_size();

// Returns the LogicalType of a vector (unsafe — requires a valid duckdb_vector).
let lt = unsafe { vector_get_column_type(some_vector) };
```

---

## Memory layout details

DuckDB stores vector data as flat arrays. `VectorReader` and `VectorWriter` compute
element addresses as `base_ptr + row * stride`:

```text
[value0][value1][value2]...[valueN]   ← typed array
[validity bitmap]                      ← separate bit array, 1 bit per row
```

The validity bitmap is lazily allocated — it may be null if no NULLs have been written.
This is why `ensure_validity_writable` must be called before any `get_validity` call
that follows a write path.

---

## Complete scalar function pattern

```rust,ignore
unsafe extern "C" fn my_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let reader = unsafe { VectorReader::new(input, 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        if unsafe { !reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let value = unsafe { reader.read_i64(row) };
        unsafe { writer.write_i64(row, transform(value)) };
    }
}
```
