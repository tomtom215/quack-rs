# NULL Handling & Strings

This page covers two topics that are handled together in practice: checking for
NULL before reading, and reading VARCHAR values from DuckDB vectors.

---

## NULL checks

Every row in a DuckDB vector may be NULL. Always check validity before reading:

```rust
for row in 0..reader.row_count() {
    if unsafe { !reader.is_valid(row) } {
        // Propagate NULL to output
        unsafe { writer.set_null(row) };
        continue;
    }
    // Safe to read
    let value = unsafe { reader.read_str(row) };
}
```

**Reading from a NULL row returns garbage data** — the vector's data buffer is
not zeroed at NULL positions. There is no bounds check or error; you get random
bytes from the data buffer.

### Writing NULL

```rust
unsafe { writer.set_null(row) };
```

> **Pitfall L4**: `VectorWriter::set_null` calls `duckdb_vector_ensure_validity_writable`
> before accessing the validity bitmap. Calling `duckdb_vector_get_validity` without this
> prerequisite returns an uninitialized pointer → SEGFAULT. Never write NULL manually;
> always use `set_null`. See [Pitfall L4](../reference/pitfalls.md#l4-ensure_validity_writable-is-required-before-null-output).

---

## VARCHAR reading

Read VARCHAR columns with `VectorReader::read_str`:

```rust
let s: &str = unsafe { reader.read_str(row) };
```

The returned `&str` borrows from the DuckDB vector — it must not outlive the
callback. Do not store it in a struct; clone it to a `String` if you need to
keep it.

### The `duckdb_string_t` format

> **Pitfall P7** — The `duckdb_string_t` format is not documented in the Rust
> bindings. This is the internalized knowledge encoded in `quack-rs`.

DuckDB stores VARCHAR values in a 16-byte `duckdb_string_t` struct with two
representations, selected at runtime based on string length:

| Format | Condition | Layout |
|--------|-----------|--------|
| **Inline** | length ≤ 12 | `[len: u32][data: [u8; 12]]` |
| **Pointer** | length > 12 | `[len: u32][prefix: [u8; 4]][ptr: *const u8][unused: u32]` |

`VectorReader::read_str` and the underlying `read_duck_string` function handle
both formats transparently. You never need to inspect the raw struct.

### Empty strings vs NULL

An empty string (`""`) and NULL are distinct values:

```rust
// NULL: is_valid returns false
// Empty string: is_valid returns true, read_str returns ""
if unsafe { !reader.is_valid(row) } {
    // This is NULL
} else {
    let s = unsafe { reader.read_str(row) };
    if s.is_empty() {
        // This is an empty string, not NULL
    }
}
```

### Writing VARCHAR

```rust
unsafe { writer.write_varchar(row, my_str) };  // &str
```

`write_varchar` copies the string bytes into DuckDB's managed storage. The
`&str` reference is no longer needed after the call returns.

---

## Complete NULL-safe VARCHAR pattern

```rust
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
        let s = unsafe { reader.read_str(row) };
        let upper = s.to_uppercase();
        unsafe { writer.write_varchar(row, &upper) };
    }
}
```

---

## DuckStringView

For advanced use cases where you need access to the raw string bytes or the
inline/pointer distinction, `quack_rs::vector::string::DuckStringView` is
available:

```rust
use quack_rs::vector::string::{DuckStringView, DUCK_STRING_SIZE};

// From raw 16-byte data (inside a vector callback)
let raw: &[u8; 16] = unsafe { &*data.add(idx * DUCK_STRING_SIZE).cast() };
let view = DuckStringView::from_bytes(raw);

println!("length: {}", view.len());
println!("is_empty: {}", view.is_empty());
if let Some(s) = view.as_str() {
    println!("content: {s}");
}
```

In practice, prefer `reader.read_str(row)` — `DuckStringView` is only needed
when you have a raw pointer and want to avoid creating a full `VectorReader`.

---

## Constants

| Constant | Value | Meaning |
|----------|-------|---------|
| `DUCK_STRING_SIZE` | `16` | Size of one `duckdb_string_t` in bytes |
| `DUCK_STRING_INLINE_MAX_LEN` | `12` | Max length stored inline (no heap ptr) |
