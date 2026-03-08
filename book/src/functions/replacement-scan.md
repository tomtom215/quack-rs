# Replacement Scans

A replacement scan lets users write:

```sql
SELECT * FROM 'myfile.myformat'
```

and have DuckDB automatically invoke your extension's table-valued scan instead of
trying to open the path as a built-in file type. This is how DuckDB's built-in CSV,
Parquet, and JSON readers work.

`quack-rs` provides `ReplacementScanBuilder` to register a replacement scan with
a 4-method chain.

## Builder API

```rust
use quack_rs::replacement_scan::ReplacementScanBuilder;

ReplacementScanBuilder::new()
    .callback(my_scan_callback)
    .delete_callback(my_delete_callback)      // optional but recommended
    .extra_data(Box::into_raw(Box::new(my_state)) as *mut _)
    .register(db)?;
```

> **Note:** Replacement scans are registered on a **database** handle
> (`duckdb_database`), not a connection. Register them before opening connections.

## Callback signature

```rust
unsafe extern "C" fn my_scan_callback(
    info: duckdb_replacement_scan_info,
    table_name: *const ::std::os::raw::c_char,
    data: *mut ::std::os::raw::c_void,
) {
    // table_name is the path string from FROM '...'
    let path = unsafe { std::ffi::CStr::from_ptr(table_name) }
        .to_str()
        .unwrap_or("");

    // Only handle files that match your format
    if !path.ends_with(".myformat") {
        return; // pass — DuckDB will try other handlers
    }

    // Tell DuckDB which function to call for this path
    unsafe { duckdb_replacement_scan_set_function_name(info, c"read_myformat".as_ptr()) };
    // Add the path as a parameter
    let val = unsafe { duckdb_create_varchar_length(path.as_ptr().cast(), path.len() as _) };
    unsafe { duckdb_replacement_scan_add_parameter(info, val) };
    unsafe { duckdb_destroy_value(&mut { val }) };
}
```

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

- [`replacement_scan`](../../src/replacement_scan/) module documentation
- [Table Functions](table-functions.md)
