# Error Handling

quack-rs uses a single error type throughout: `ExtensionError`.

---

## `ExtensionError`

```rust
use quack_rs::error::{ExtensionError, ExtResult};
# let (name, code) = ("my_fn", 1);
# let some_std_error = std::fmt::Error;

// From a string literal
let e = ExtensionError::from("something went wrong");

// From a format string
let e = ExtensionError::new(format!("failed to register '{}': code {}", name, code));

// Wrapping another error
let e = ExtensionError::from_error(some_std_error);
```

`ExtensionError` implements:
- `std::error::Error`
- `Display`, `Debug`, `Clone`, `PartialEq`, `Eq`
- `From<&str>`, `From<String>`, `From<Box<dyn Error>>`
- `From<std::io::Error>`, `From<std::ffi::NulError>`, `From<std::fmt::Error>`

The `From<std::io::Error>` impl is especially useful for extensions that
allocate runtime resources (e.g., tokio) during initialization — the `?`
operator works directly without `.map_err()`:

```rust
# use quack_rs::connection::Connection;
# use quack_rs::error::ExtensionError;
# // A stand-in with tokio's signature: `Runtime::new() -> std::io::Result<Runtime>`.
# mod tokio { pub mod runtime { pub struct Runtime;
#     impl Runtime { pub fn new() -> std::io::Result<Self> { Ok(Runtime) } } } }
fn register_all(con: &Connection) -> Result<(), ExtensionError> {
    let _rt = tokio::runtime::Runtime::new()?; // ← io::Error → ExtensionError
    // ... register functions ...
    Ok(())
}
```

---

## `ExtResult<T>`

A type alias for `Result<T, ExtensionError>`, used throughout the SDK:

```rust
# use quack_rs::error::ExtensionError;
pub type ExtResult<T> = Result<T, ExtensionError>;
```

---

## Propagating errors with `?`

In your registration function:

```rust
# use libduckdb_sys::{duckdb_connection, duckdb_data_chunk, duckdb_function_info, duckdb_vector};
# use quack_rs::prelude::*;
# unsafe extern "C" fn my_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        ScalarFunctionBuilder::new("my_fn")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(my_fn)
            .register(con)?;   // ← ? propagates registration errors

        SqlMacro::scalar("my_macro", &["x"], "x + 1")?
            .register(con)?;

        Ok(())
    }
}
```

If any registration call fails, `?` returns the error from `register`, which
`init_extension` then reports to DuckDB via `access.set_error`.

---

## Error reporting to DuckDB

`init_extension` converts `ExtensionError` to a `CString` for the DuckDB error callback
with `to_c_string`. A C string cannot hold a NUL byte, so each one is replaced with
`?` and the rest of the message is kept:

```rust
use quack_rs::error::ExtensionError;

let err = ExtensionError::new("bad\0input");
assert_eq!(err.to_c_string().to_str(), Ok("bad?input"));
```

DuckDB surfaces this string to the user as the extension load error.

---

## No panics, ever

The cardinal rule of DuckDB extension development:

> **Never `unwrap()`, `expect()`, or `panic!()` in any code path that DuckDB may call.**

A panic cannot unwind out of an `extern "C"` function: since Rust 1.81 the runtime aborts
the process instead, taking the user's DuckDB session with it. quack-rs's callback macros
and typed builders catch panics and turn them into SQL errors (which requires
`panic = "unwind"` in the release profile), but that is a safety net for bugs, not an
error-handling strategy.

### Safe patterns

```rust
# use libduckdb_sys::duckdb_aggregate_state;
# use quack_rs::aggregate::{AggregateState, FfiState};
# use quack_rs::error::ExtensionError;
# #[derive(Default)] struct MyState { count: u64 }
# impl AggregateState for MyState {}
# fn some_fallible_call() -> Result<u64, ExtensionError> { Ok(1) }
# unsafe fn demo(state_ptr: duckdb_aggregate_state, maybe_count: Option<u64>)
#     -> Result<(), ExtensionError> {
// ✅ Use Option methods
if let Some(s) = FfiState::<MyState>::with_state_mut(state_ptr) {
    s.count += 1;
}

// ✅ Use Result and ?
let value = some_fallible_call()?;

// ✅ Use unwrap_or / unwrap_or_else / map
let count = maybe_count.unwrap_or(0);

// ❌ Never in FFI callbacks
let s = FfiState::<MyState>::with_state_mut(state_ptr).unwrap(); // panics if None
# Ok(())
# }
```

### In `init_extension`

`init_extension` wraps everything in `match` and reports errors via `set_error` — it can
never panic regardless of what your registration closure returns.
