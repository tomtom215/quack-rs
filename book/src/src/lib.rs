//! Does something useful
//!
//! A DuckDB extension built with [quack-rs](https://github.com/tomtom215/quack-rs).

use quack_rs::prelude::*;

// ---------------------------------------------------------------------------
// Example: a simple SQL macro. Replace with your own functions.
// ---------------------------------------------------------------------------

/// Registers all extension functions on the given connection.
fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
    // Example: register a scalar SQL macro (no unsafe callbacks needed).
    // Replace this with your own aggregate, scalar, or table functions.
    //
    // SAFETY: `con` is the connection quack-rs opened for this entry point and
    // is valid for the duration of this function.
    unsafe {
        SqlMacro::scalar(
            "my_extension_hello",
            &["name"],
            "concat('Hello from my_extension! ', name)",
        )?
        .register(con)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point — the C Extension API handles everything, no C++ glue needed.
// ---------------------------------------------------------------------------

quack_rs::entry_point!(my_extension_init_c_api, register);
