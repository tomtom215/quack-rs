# Replacement Scans

A DuckDB replacement scan lets users query a file by its path alone:

```sql
SELECT * FROM 'myfile.myformat'
```

When DuckDB finds no table with that name, it calls each registered replacement
scan in registration order, and one of them can redirect the query to a table
function (in the example below, `read_myformat('myfile.myformat')`). DuckDB's own
CSV, Parquet and JSON readers use the same mechanism. This page shows how to
register one from a Rust extension with quack-rs.

`quack-rs` provides `ReplacementScanBuilder` (a static registration helper) and
`ReplacementScanInfo` (an ergonomic wrapper for callbacks).

## Registration API

Unlike the other builders in quack-rs, `ReplacementScanBuilder` uses a single
static call because the DuckDB C API takes all arguments at once:

```rust
# use libduckdb_sys::{duckdb_database, duckdb_replacement_scan_info};
# use std::os::raw::{c_char, c_void};
# unsafe extern "C" fn my_scan_callback(_: duckdb_replacement_scan_info, _: *const c_char, _: *mut c_void) {}
# fn demo(db: duckdb_database, my_state: String) {
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
# }
```

> **Note:** Replacement scans are registered on a **database** handle
> (`duckdb_database`), not a connection, and apply to every connection to that
> database. In an entry point, `Connection::register_replacement_scan` and
> `register_replacement_scan_with_data` do the same through the `Connection` you
> are given.

A raw `delete_callback` must accept a null argument: unlike DuckDB's other
destructor slots, it is called with `extra_data` even when that is null.

## Callback signature

The raw callback receives `duckdb_replacement_scan_info`, but you can wrap it
with `ReplacementScanInfo` for ergonomic, safe access:

```rust
# use libduckdb_sys::duckdb_replacement_scan_info;
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
        return; // not ours: DuckDB tries the next replacement scan
    }

    // Use ReplacementScanInfo for ergonomic access
    unsafe {
        ReplacementScanInfo::new(info)
            .set_function("read_myformat")
            .add_varchar_parameter(path);
    }
}
```

`table_name` is the last part of the table reference: `FROM myschema."f.myformat"`
reaches the callback as `f.myformat`, so a callback cannot see or honour a schema.

### `ReplacementScanInfo` methods

| Method | Description |
|--------|-------------|
| `set_function(name)` | Redirect to the named table function |
| `add_varchar_parameter(value)` | Add a VARCHAR parameter to the redirected call |
| `add_i64_parameter(value)` | Add a BIGINT (`i64`) parameter |
| `add_bool_parameter(value)` | Add a BOOLEAN parameter |
| `add_parameter_raw(duckdb_value)` | Add a parameter of any type; DuckDB copies it, so the caller still destroys the value |
| `set_error(message)` | Report an error, which fails the query. DuckDB ignores an empty message, so an empty one is replaced with a placeholder |

Data passed to `ReplacementScanBuilder::register_with_data` must be `Send + Sync`:
it lives in the database-wide configuration, is read by the callback from any
connection's thread (concurrently), and is dropped by whichever thread closes the
database. The raw `register` has the same requirement, stated in its `# Safety`.

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
