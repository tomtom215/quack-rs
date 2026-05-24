# Structured Errors

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

`ErrorData` is an RAII wrapper around DuckDB's `duckdb_error_data` handle — the
structured error type returned by several DuckDB 1.5 C API surfaces. Unlike a
bare error string, an `ErrorData` carries both a human-readable **message** and a
machine-readable **category** ([`DuckDbErrorType`]), so your extension can branch
on the *kind* of failure (for example, distinguishing `Io` from `OutOfMemory`).

It is the common currency of the other 1.5 modules: [`Expression::fold`](expression.md),
the [virtual file system](file-system.md), and the [appender](appender.md) all
report failures as an `ErrorData`.

## Inspecting an error

```rust,no_run
use quack_rs::error_data::{DuckDbErrorType, ErrorData};

# fn handle(err: ErrorData) {
if err.has_error() {
    match err.error_type() {
        DuckDbErrorType::Io => eprintln!("I/O failure"),
        DuckDbErrorType::OutOfMemory => eprintln!("out of memory"),
        other => eprintln!("{other:?}: {}", err.message().unwrap_or_default()),
    }
}
# }
```

## Constructing an error

Build a structured error to hand back to DuckDB (for example from a callback):

```rust,no_run
use quack_rs::error_data::{DuckDbErrorType, ErrorData};

let err = ErrorData::new(DuckDbErrorType::InvalidInput, "row index out of range");
assert!(err.has_error());
assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
```

## Propagating with `?`

`into_extension_error` converts an `ErrorData` into the SDK's [`ExtensionError`],
so a structured DuckDB error can flow through the `?` operator in your
registration or callback logic:

```rust,no_run
use quack_rs::error::ExtensionError;
use quack_rs::file_system::{FileOpenOptions, FileSystem};
use quack_rs::client_context::ClientContext;

# fn read_header(ctx: &ClientContext) -> Result<(), ExtensionError> {
let fs = FileSystem::from_client_context(ctx)
    .ok_or_else(|| ExtensionError::new("no file system"))?;
let handle = fs
    .open(c"data.bin", &FileOpenOptions::read_only())
    .map_err(ErrorData::into_extension_error)?;
# let _ = handle;
# Ok(())
# }
```

## API

| Item | Description |
|------|-------------|
| `ErrorData::new(error_type, message)` | Construct a structured error |
| `ErrorData::from_raw(raw)` (unsafe) | Take ownership of a raw `duckdb_error_data` |
| `has_error()` | `true` if the handle represents an actual error |
| `error_type()` | The [`DuckDbErrorType`] category |
| `message()` | `Option<String>` — the error text |
| `into_extension_error()` | Consume into an [`ExtensionError`] |
| `is_null()` / `as_raw()` / `into_raw()` | Handle inspection / escape hatches |

`DuckDbErrorType` is a `#[non_exhaustive]` enum mirroring `duckdb_error_type`
(`Io`, `OutOfMemory`, `Conversion`, `Catalog`, `Constraint`, `Permission`, …).
Unknown or future categories map to `DuckDbErrorType::Invalid`.

## UTF-8 validation

The free function `check_valid_utf8` exposes DuckDB's own UTF-8 validator, which
is stricter than Rust's in some cases. Use it to validate externally-sourced
bytes before handing them to DuckDB string APIs:

```rust,no_run
use quack_rs::error_data::check_valid_utf8;

# fn demo(bytes: &[u8]) {
match check_valid_utf8(bytes) {
    Ok(()) => { /* safe to pass to DuckDB */ }
    Err(err) => eprintln!("invalid UTF-8: {}", err.message().unwrap_or_default()),
}
# }
```

## Ownership

`ErrorData` calls `duckdb_destroy_error_data` on drop. When you receive one from
a fallible 1.5 API, it owns the handle — just let it drop, or call
`into_extension_error()` / `into_raw()` to move the data out.

## Related modules

- [Bound Expressions](expression.md) — `Expression::fold` returns `ErrorData`
- [Virtual File System](file-system.md) — file operations return `ErrorData`
- [Bulk Appender](appender.md) — `Appender::error_data` returns `ErrorData`
- [Error Handling](../concepts/errors.md) — the SDK's primary [`ExtensionError`] type

[`DuckDbErrorType`]: https://docs.rs/quack-rs/latest/quack_rs/error_data/enum.DuckDbErrorType.html
[`ExtensionError`]: https://docs.rs/quack-rs/latest/quack_rs/error/struct.ExtensionError.html
