# Virtual File System

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

This module exposes DuckDB's **virtual file system** (VFS) to your extension, so
a custom table function, replacement scan, or copy function can read and write
files through the *same* abstraction DuckDB uses internally. That means
transparently honouring `httpfs` (`s3://`, `http://`), in-memory files, and any
other registered file system — instead of reaching for `std::fs` and only ever
seeing local disk.

## Obtaining a `FileSystem`

A `FileSystem` comes from a [`ClientContext`], which most function callbacks can
hand you (for example via `BindInfo::get_client_context()` or
`ScalarBindInfo::get_client_context()`).

## Reading a file

```rust,no_run
use quack_rs::client_context::ClientContext;
use quack_rs::file_system::{FileOpenOptions, FileSystem};

# fn read_all(ctx: &ClientContext) -> Option<Vec<u8>> {
let fs = FileSystem::from_client_context(ctx)?;
let handle = fs.open(c"s3://bucket/data.csv", &FileOpenOptions::read_only()).ok()?;

let mut buf = Vec::new();
handle.read_to_end(&mut buf).ok()?;
Some(buf)
# }
```

## Writing a file

```rust,no_run
use quack_rs::client_context::ClientContext;
use quack_rs::file_system::{FileOpenOptions, FileSystem};

# fn write_report(ctx: &ClientContext, bytes: &[u8]) -> Result<(), quack_rs::error_data::ErrorData> {
let fs = FileSystem::from_client_context(ctx).expect("file system");
let handle = fs.open(c"report.bin", &FileOpenOptions::write_create())?;
handle.write_all(bytes)?;
handle.sync()?;
handle.close()?;
# Ok(())
# }
```

## Open options and flags

`FileOpenOptions` describes how a file is opened. Two convenience constructors
cover the common cases; use `set_flag` for anything else.

| Constructor / method | Effect |
|----------------------|--------|
| `FileOpenOptions::read_only()` | Open for reading |
| `FileOpenOptions::write_create()` | Open for writing, creating if absent |
| `FileOpenOptions::new()` | Empty; configure with `set_flag` |
| `set_flag(flag, value)` | Set an individual [`FileFlag`]; returns `true` on success |

`FileFlag` variants: `Read`, `Write`, `Create`, `CreateNew`, `Append`.

- `Create`, `CreateNew` and `Append` need `Write` as well.
- `CreateNew` ("create, failing if the file exists") also sets `Create`. DuckDB maps
  it to `FILE_FLAGS_EXCLUSIVE_CREATE`, which only has that meaning together with
  `FILE_FLAGS_FILE_CREATE`; on its own it neither created a missing file nor refused
  an existing one.
- `set_flag(flag, false)` does **not** clear a flag: the C API ORs flags in and
  ignores `value`. Build a fresh `FileOpenOptions` instead.

## `FileHandle` operations

| Method | Returns | Description |
|--------|---------|-------------|
| `read(&mut buf)` | `Result<usize, ErrorData>` | Read **up to** `buf.len()` bytes (0 = EOF) |
| `read_exact(&mut buf)` | `Result<(), ErrorData>` | Read exactly `buf.len()` bytes, or fail |
| `read_to_end(&mut vec)` | `Result<usize, ErrorData>` | Append the rest of the file |
| `write(&buf)` | `Result<usize, ErrorData>` | Write **up to** `buf.len()` bytes |
| `write_all(&buf)` | `Result<(), ErrorData>` | Write all of `buf`, or fail |
| `seek(position)` | `Result<(), ErrorData>` | Seek to an absolute byte offset |
| `tell()` | `Result<u64, ErrorData>` | Current byte offset |
| `size()` | `Result<u64, ErrorData>` | Total file size in bytes |
| `sync()` | `Result<(), ErrorData>` | Flush buffered writes to durable storage |
| `close()` | `Result<(), ErrorData>` | Close the file |
| `error_data()` | `ErrorData` | Structured error from the last failed operation |

### Short reads and short writes are real

`duckdb_file_handle_read` and `duckdb_file_handle_write` return "the number of
bytes **actually** read/written" — not a promise that the whole buffer moved. On
a local file the two almost always agree; over `httpfs` they routinely do not.
Prefer `read_exact` / `read_to_end` / `write_all`, which loop for you, and reach
for the raw `read` / `write` only when a partial transfer is what you want.

`tell()` and `size()` are `Result` for the same reason: the C API reports failure
with a *negative* return value, which is far too easy to clamp to zero and then
silently treat as an empty file.

`FileSystem` exposes `open(path, options)` and `error_data()`. Both `FileSystem`
and `FileHandle` are RAII: they are destroyed (and the handle closed) on drop.

## Lifetimes

A `FileSystem<'ctx>` borrows the `ClientContext` it came from, and a
`FileHandle<'fs>` borrows the `FileSystem` that opened it. A handle refers to the
database's file system, so using it after the database is closed is a
use-after-free (valgrind: invalid read in `duckdb::FileHandle::Read`); the borrow
makes that a compile error instead:

```rust,compile_fail
use quack_rs::client_context::ClientContext;
use quack_rs::file_system::{FileOpenOptions, FileSystem};

fn demo(ctx: &ClientContext) {
    let fs = FileSystem::from_client_context(ctx).unwrap();
    let handle = fs.open(c"data.csv", &FileOpenOptions::read_only()).unwrap();
    drop(fs); // error[E0505]: cannot move out of `fs` because it is borrowed
    let _ = handle.size();
}
```

## Related modules

- [Replacement Scans](../functions/replacement-scan.md) — `SELECT * FROM 'file.xyz'`
  handlers that read through this file system
- [Copy Functions](../functions/copy-functions.md) — `COPY TO` handlers that write through it
- [Structured Errors](error-data.md) — the [`ErrorData`] returned on failure

[`ClientContext`]: https://docs.rs/quack-rs/latest/quack_rs/client_context/struct.ClientContext.html
[`FileFlag`]: https://docs.rs/quack-rs/latest/quack_rs/file_system/enum.FileFlag.html
[`ErrorData`]: https://docs.rs/quack-rs/latest/quack_rs/error_data/struct.ErrorData.html
