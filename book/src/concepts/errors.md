# Error Handling

quack-rs uses a single error type throughout: `ExtensionError`.

---

## `ExtensionError`

```rust,ignore
use quack_rs::error::{ExtensionError, ExtResult};

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

```rust,ignore
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
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
pub type ExtResult<T> = Result<T, ExtensionError>;
```

---

## Propagating errors with `?`

In your registration function:

```rust,ignore
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

`init_extension` converts `ExtensionError` to a `CString` for the DuckDB error callback:

```rust,ignore
pub fn to_c_string(&self) -> CString {
    // Truncates at the first null byte if message contains one
    CString::new(self.message.as_bytes()).unwrap_or_else(...)
}
```

DuckDB surfaces this string to the user as the extension load error.

---

## No panics, ever

The cardinal rule of DuckDB extension development:

> **Never `unwrap()`, `expect()`, or `panic!()` in any code path that DuckDB may call.**

quack-rs catches panics at every `extern "C"` boundary and converts them to DuckDB errors,
which requires `panic = "unwind"` in the release profile — under `panic = "abort"` the
process dies with `SIGABRT` instead. That guard is a backstop, not a licence: a panicking
code path still aborts the user's query, so keep them out of DuckDB-called code.

### Safe patterns

```rust,ignore
// ✅ Use Option methods
if let Some(s) = FfiState::<MyState>::with_state_mut(state_ptr) {
    s.count += 1;
}

// ✅ Use Result and ?
let value = some_fallible_call()?;

// ✅ Use unwrap_or / unwrap_or_else / map
let count = maybe_count.unwrap_or(0);

// ❌ Never in FFI callbacks
let s = FfiState::<MyState>::with_state_mut(state_ptr).unwrap(); // undefined behavior
```

### In `init_extension`

`init_extension` wraps everything in `match` and reports errors via `set_error` — it can
never panic regardless of what your registration closure returns.
