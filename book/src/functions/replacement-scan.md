# Replacement Scans

A replacement scan lets users write:

```sql
SELECT * FROM 'myfile.myformat'
```

and have DuckDB automatically invoke your extension's table-valued scan instead of
trying to open the path as a built-in file type. This is how DuckDB's built-in CSV,
Parquet, and JSON readers work.

`quack-rs` provides `ReplacementScanBuilder` (a static registration helper) and
`ReplacementScanInfo` (an ergonomic wrapper for callbacks).

## Registration API

Unlike the other builders in quack-rs, `ReplacementScanBuilder` uses a single
static call because the DuckDB C API takes all arguments at once:

```rust
use quack_rs::replacement_scan::ReplacementScanBuilder;

// Low-level: pass raw extra_data and an optional delete callback.
unsafe {
    ReplacementScanBuilder::register(
        db,                            // duckdb_database
        my_scan_callback,              // ReplacementScanFn
        std::ptr::null_mut(),          // extra_data (or a raw pointer)
        None,                          // delete_callback
    );
}

// Ergonomic: pass owned Rust data; boxing and destructor are handled for you.
unsafe {
    ReplacementScanBuilder::register_with_data(db, my_scan_callback, my_state);
}
```

> **Note:** Replacement scans are registered on a **database** handle
> (`duckdb_database`), not a connection. Register them before opening connections.

## Callback signature

The raw callback receives `duckdb_replacement_scan_info`, but you can wrap it
with `ReplacementScanInfo` for ergonomic, safe access:

```rust
use quack_rs::replacement_scan::ReplacementScanInfo;

unsafe extern "C" fn my_scan_callback(
    info: duckdb_replacement_scan_info,
    table_name: *const ::std::os::raw::c_char,
    _data: *mut ::std::os::raw::c_void,
) {
    let path = unsafe { std::ffi::CStr::from_ptr(table_name) }
        .to_str()
        .unwrap_or("");

    if !path.ends_with(".myformat") {
        return; // pass — DuckDB will try other handlers
    }

    // Use ReplacementScanInfo for ergonomic access
    unsafe {
        ReplacementScanInfo::new(info)
            .set_function("read_myformat")
            .add_varchar_parameter(path);
    }
}
```

### `ReplacementScanInfo` methods

| Method | Description |
|--------|-------------|
| `set_function(name)` | Redirect to the named table function |
| `add_varchar_parameter(value)` | Add a VARCHAR parameter to the redirected call |
| `add_i64_parameter(value)` | Add a BIGINT (i64) parameter (v0.11.0+) |
| `add_bool_parameter(value)` | Add a BOOLEAN parameter (v0.11.0+) |
| `add_parameter_raw(duckdb_value)` | Add any typed `duckdb_value` parameter (v0.11.0+) |
| `set_error(message)` | Report an error (aborts this replacement scan) |

## When to use replacement scans vs table functions

| Scenario | Use |
|----------|-----|
| `SELECT * FROM my_function('file.ext')` | Table function |
| `SELECT * FROM 'file.ext'` (bare path) | Replacement scan → delegates to a table function |
| File type auto-detection | Replacement scan |

Most extensions implement **both**: a table function that does the actual work,
and a replacement scan that detects the file extension and transparently routes
bare-path queries to the table function.

## See also

- [`replacement_scan`](https://docs.rs/quack-rs/latest/quack_rs/replacement_scan/index.html) module documentation
- [Table Functions](table-functions.md)
