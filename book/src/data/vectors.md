# Reading & Writing Vectors

DuckDB passes data to and from your extension as **vectors**: columnar arrays of typed
values, each with a separate validity (NULL) bitmap. `VectorReader` and `VectorWriter`
give typed access to these vectors from scalar, aggregate and table function callbacks;
`DataChunk`, `StructReader`, `StructWriter` and `ChunkWriter` build on them.

---

## `VectorReader`

### Construction

```rust
# use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(input: duckdb_data_chunk, column_index: usize) {
// In a scalar function callback:
let reader = unsafe { VectorReader::new(input, column_index) };

// In an aggregate update callback:
let reader = unsafe { VectorReader::new(input, 0) };   // first column
# }
```

`VectorReader::new` takes the `duckdb_data_chunk` and a zero-based column index. The
reader holds raw pointers into the chunk, so it must not outlive the callback.

### Row count

```rust
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(reader: &VectorReader) {
let n = reader.row_count();   // number of rows in this chunk
# }
```

Chunk sizes vary. Always loop over `0..reader.row_count()`; never assume a fixed size.

### NULL check

```rust
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(reader: &VectorReader, writer: &mut VectorWriter) {
# for row in 0..reader.row_count() {
if unsafe { !reader.is_valid(row) } {
    // row is NULL — skip or propagate NULL to output
    unsafe { writer.set_null(row) };
    continue;
}
# }
# }
```

**Always check `is_valid` before reading.** Reading a fixed-width value from a
NULL row returns garbage data; reading a `VARCHAR` or `BLOB` from one can
follow a stale pointer into freed memory (see
[NULL Handling & Strings](nulls-and-strings.md)).

### Reading values

```rust
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(reader: &VectorReader, row: usize) {
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
# }
```

---

## `VectorWriter`

### Construction

```rust
# use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(output: duckdb_vector, result: duckdb_vector) {
// In a scalar function callback:
let mut writer = unsafe { VectorWriter::new(output) };

// In an aggregate finalize callback:
let mut writer = unsafe { VectorWriter::new(result) };
# }
```

### Writing values

```rust
# use quack_rs::vector::{VectorReader, VectorWriter};
# use quack_rs::interval::DuckInterval;
# fn demo(writer: &mut VectorWriter, row: usize, s: &str, interval: DuckInterval,
#     days_since_epoch: i32, micros_since_epoch: i64, micros_since_midnight: i64,
#     bytes: Vec<u8>, uuid_bits: u128) {
unsafe { writer.write_i8(row, -8) };
unsafe { writer.write_i16(row, -16) };
unsafe { writer.write_i32(row, -32) };
unsafe { writer.write_i64(row, -64) };
unsafe { writer.write_u8(row, 8) };
unsafe { writer.write_u16(row, 16) };
unsafe { writer.write_u32(row, 32) };
unsafe { writer.write_u64(row, 64) };
unsafe { writer.write_f32(row, 3.5) };
unsafe { writer.write_f64(row, 2.5) };
unsafe { writer.write_bool(row, true) };
unsafe { writer.write_varchar(row, s) };   // &str
unsafe { writer.write_str(row, s) };       // alias for write_varchar
unsafe { writer.write_interval(row, interval) };  // DuckInterval

// Temporal and binary types (v0.10.0+):
unsafe { writer.write_date(row, days_since_epoch) };
unsafe { writer.write_timestamp(row, micros_since_epoch) };
unsafe { writer.write_time(row, micros_since_midnight) };
unsafe { writer.write_blob(row, &bytes) };
unsafe { writer.write_uuid(row, uuid_bits) };        // UUID's textual 128 bits
# }
```

`write_varchar` and `write_blob` panic for a value longer than
`vector::string::MAX_STRING_LEN` (`u32::MAX` bytes, DuckDB's string length
limit) rather than store a truncated one. Inside `scalar_callback!` and the
typed scalar constructors the panic becomes a SQL error. To handle the error
yourself, use `try_write_varchar` / `try_write_blob`, which return
`Result<(), ExtensionError>` and write nothing on error.

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

```rust
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(writer: &mut VectorWriter, row: usize) {
unsafe { writer.set_null(row) };
# }
```

> **Pitfall L4**: `set_null` calls `duckdb_vector_ensure_validity_writable` automatically
> before `duckdb_vector_get_validity`. A vector with no NULLs yet usually has no validity
> mask, so without that call `get_validity` returns NULL and `duckdb_validity_set_row_invalid`
> silently does nothing — the row you meant to be NULL reads back as a valid value.
> `VectorWriter::set_null` handles this correctly. See [Pitfall L4](../reference/pitfalls.md#l4-ensure_validity_writable-is-required-before-null-output).

#### NULL rows of STRUCT and ARRAY outputs

For a `STRUCT` output, `set_null(row)` (and `set_null_range`, and
`DataChunk::propagate_nulls`, which uses it) also nulls that row in **every
field**, recursively; for an `ARRAY` of size `n` it nulls child rows
`row * n .. row * n + n`. This mirrors DuckDB's internal `FlatVector::SetNull`,
and it matters: `struct_extract` / `s.a` reads the field vector without looking
at the parent, so a NULL struct row whose fields were left valid returns the
stale field value. `StructWriter::set_row_null(row)` does the same from a
`StructWriter`. `LIST` / `MAP` elements are not touched (as in DuckDB).

To reuse such a row, call `set_valid(row)` first: on a row that is NULL it also
marks valid everything `set_null` nulled below it. Then write the fields, and
any field NULLs after that. On a row that is already valid, `set_valid` leaves
the fields alone, so marking a row valid after writing its fields keeps their
NULLs.

### Clearing NULL (v0.11.0+)

To undo a previous `set_null` call and mark a row as valid again:

```rust
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(writer: &mut VectorWriter, row: usize) {
unsafe { writer.set_valid(row) };
# }
```

Like `set_null`, `set_valid` calls `ensure_validity_writable` first.

---

## `DataChunk`

`DataChunk` wraps a `duckdb_data_chunk` handle and gives access to its vectors
and row count without raw FFI calls:

```rust
# use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
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
- `any_null(row)` — whether any column is NULL at `row`
- `propagate_nulls(&mut writer)` — mark each output row NULL where any input column is NULL
- `into_chunk_writer()` — convert to `ChunkWriter`, which calls `set_size` on drop

---

## `StructWriter` / `StructReader`

For STRUCT columns, creating a `VectorWriter` or `VectorReader` for each field by
hand is verbose. `StructWriter` and `StructReader` create one per field at
construction:

```rust
# use quack_rs::data_chunk::DataChunk;
# struct Output { success: bool, data: String, count: i64, day: i32, payload: Vec<u8> }
# fn demo(chunk: &DataChunk, row: usize, result: &Output) {
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
# }
```

---

## `ChunkWriter`

`ChunkWriter` wraps an output `duckdb_data_chunk` and counts the rows handed out
by `next_row`. It calls `set_size` with that count on drop, so the row count
cannot be forgotten or set wrongly. `next_row` returns `None` once the chunk
holds `duckdb_vector_size()` rows:

```rust
# use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# use quack_rs::data_chunk::DataChunk;
# struct Item { name: String, value: i64 }
# fn demo(output: duckdb_data_chunk, data: &[Item]) {
let mut cw = unsafe { DataChunk::from_raw(output).into_chunk_writer() };
for item in data {
    let Some(row) = cw.next_row() else { break };   // chunk is full
    unsafe { cw.writer(0).write_varchar(row, &item.name) };
    unsafe { cw.writer(1).write_i64(row, item.value) };
}
// set_size called automatically when `cw` is dropped
# }
```

---

## `ValidityBitmap`

For advanced NULL handling beyond `VectorWriter::set_null`, use `ValidityBitmap`
directly:

```rust
# use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# fn demo(some_vector: duckdb_vector, row: usize) {
use quack_rs::vector::ValidityBitmap;

// Writing NULLs:
let mut bitmap = unsafe { ValidityBitmap::ensure_writable(some_vector) };
unsafe { bitmap.set_row_invalid(row as u64) };   // mark as NULL
unsafe { bitmap.set_row_valid(row as u64) };     // mark as non-NULL

// Reading NULLs:
let bitmap = unsafe { ValidityBitmap::get_read_only(some_vector) };
let is_valid = unsafe { bitmap.row_is_valid(row as u64) };
# }
```

`ValidityBitmap` is available in the prelude: `use quack_rs::prelude::*`.

---

## Utility functions

The `quack_rs::vector` module provides two utility functions:

```rust
# use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# fn demo(some_vector: duckdb_vector) {
use quack_rs::vector::{vector_size, vector_get_column_type};

// Rows per data chunk: 2048 unless DuckDB was built with another STANDARD_VECTOR_SIZE.
let size: u64 = vector_size();

// Returns the LogicalType of a vector (unsafe — requires a valid duckdb_vector).
let lt = unsafe { vector_get_column_type(some_vector) };
# }
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
This is why `duckdb_vector_ensure_validity_writable` must be called before
`duckdb_vector_get_validity` when writing NULLs; `VectorWriter` and
`ValidityBitmap::ensure_writable` do so.

---

## Complete scalar function pattern

```rust
# use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn transform(v: i64) -> i64 { v }
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
