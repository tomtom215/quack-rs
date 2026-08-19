// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Copy function registration (`DuckDB` 1.5.0+).
//!
//! Extensions can register custom `COPY TO` handlers that define how data is
//! exported to a specific file format. The lifecycle consists of four phases:
//!
//! 1. **Bind** — inspect the output columns and configure the export.
//! 2. **Global init** — set up global state (open file, allocate buffers).
//! 3. **Sink** — receive data chunks to write.
//! 4. **Finalize** — flush and close.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::copy_function::CopyFunctionBuilder;
//!
//! let builder = CopyFunctionBuilder::try_new("my_format")?;
//! // .bind(my_bind_fn)
//! // .global_init(my_init_fn)
//! // .sink(my_sink_fn)
//! // .finalize(my_finalize_fn)
//! # Ok::<(), quack_rs::error::ExtensionError>(())
//! ```

pub mod info;

pub use info::{CopyBindInfo, CopyFinalizeInfo, CopyGlobalInitInfo, CopySinkInfo};

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_connection, duckdb_copy_function_bind_info, duckdb_copy_function_finalize_info,
    duckdb_copy_function_global_init_info, duckdb_copy_function_set_bind,
    duckdb_copy_function_set_finalize, duckdb_copy_function_set_global_init,
    duckdb_copy_function_set_name, duckdb_copy_function_set_sink, duckdb_copy_function_sink_info,
    duckdb_create_copy_function, duckdb_data_chunk, duckdb_destroy_copy_function,
    duckdb_register_copy_function, DuckDBSuccess,
};

use crate::error::ExtensionError;

/// Callback type aliases for copy function phases.
///
/// Bind callback — called once to configure the export.
pub type CopyBindFn = unsafe extern "C" fn(info: duckdb_copy_function_bind_info);

/// Global init callback — called once to set up global state.
pub type CopyGlobalInitFn = unsafe extern "C" fn(info: duckdb_copy_function_global_init_info);

/// Sink callback — called once per data chunk to write data.
pub type CopySinkFn =
    unsafe extern "C" fn(info: duckdb_copy_function_sink_info, chunk: duckdb_data_chunk);

/// Finalize callback — called once to flush and close.
pub type CopyFinalizeFn = unsafe extern "C" fn(info: duckdb_copy_function_finalize_info);

/// Builder for registering a custom `COPY TO` function.
///
/// All four lifecycle callbacks (bind, `global_init`, sink, finalize) should be
/// set before calling [`register`][Self::register].
#[must_use]
pub struct CopyFunctionBuilder {
    name: CString,
    bind: Option<CopyBindFn>,
    global_init: Option<CopyGlobalInitFn>,
    sink: Option<CopySinkFn>,
    finalize: Option<CopyFinalizeFn>,
}

impl CopyFunctionBuilder {
    /// Creates a new copy function builder with the given format name.
    ///
    /// The name corresponds to the format identifier used in
    /// `COPY table TO 'file' (FORMAT name)`.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the name contains a null byte.
    pub fn try_new(name: &str) -> Result<Self, ExtensionError> {
        let c_name = CString::new(name)
            .map_err(|_| ExtensionError::new("copy function name contains null byte"))?;
        Ok(Self {
            name: c_name,
            bind: None,
            global_init: None,
            sink: None,
            finalize: None,
        })
    }

    /// Returns the name of this copy function.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or("")
    }

    /// Sets the bind callback.
    pub fn bind(mut self, f: CopyBindFn) -> Self {
        self.bind = Some(f);
        self
    }

    /// Sets the global init callback.
    pub fn global_init(mut self, f: CopyGlobalInitFn) -> Self {
        self.global_init = Some(f);
        self
    }

    /// Sets the sink callback.
    pub fn sink(mut self, f: CopySinkFn) -> Self {
        self.sink = Some(f);
        self
    }

    /// Sets the finalize callback.
    pub fn finalize(mut self, f: CopyFinalizeFn) -> Self {
        self.finalize = Some(f);
        self
    }

    /// Registers the copy function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if required callbacks are missing or registration
    /// fails.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let bind = self
            .bind
            .ok_or_else(|| ExtensionError::new("copy function bind callback not set"))?;
        let sink = self
            .sink
            .ok_or_else(|| ExtensionError::new("copy function sink callback not set"))?;
        let finalize = self
            .finalize
            .ok_or_else(|| ExtensionError::new("copy function finalize callback not set"))?;

        // SAFETY: duckdb_create_copy_function allocates a new handle.
        let func = unsafe { duckdb_create_copy_function() };

        // SAFETY: func is a valid newly created handle.
        unsafe {
            duckdb_copy_function_set_name(func, self.name.as_ptr());
            duckdb_copy_function_set_bind(func, Some(bind));
            duckdb_copy_function_set_sink(func, Some(sink));
            duckdb_copy_function_set_finalize(func, Some(finalize));
        }

        if let Some(global_init) = self.global_init {
            // SAFETY: func is valid.
            unsafe {
                duckdb_copy_function_set_global_init(func, Some(global_init));
            }
        }

        // SAFETY: con is valid, func is fully configured.
        let result = unsafe { duckdb_register_copy_function(con, func) };

        // SAFETY: func must be destroyed after registration.
        let mut func_mut = func;
        unsafe {
            duckdb_destroy_copy_function(&raw mut func_mut);
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_copy_function failed for '{}'",
                self.name.to_string_lossy()
            )))
        }
    }
}

impl core::fmt::Debug for CopyFunctionBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use crate::debug_repr::Callback;
        f.debug_struct("CopyFunctionBuilder")
            .field("name", &self.name)
            .field("bind", &Callback::of(&self.bind))
            .field("global_init", &Callback::of(&self.global_init))
            .field("sink", &Callback::of(&self.sink))
            .field("finalize", &Callback::of(&self.finalize))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_new_valid_name() {
        let builder = CopyFunctionBuilder::try_new("parquet").unwrap();
        assert_eq!(builder.name(), "parquet");
    }

    #[test]
    fn try_new_null_byte_rejected() {
        let result = CopyFunctionBuilder::try_new("bad\0name");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            err.to_string().contains("null byte"),
            "error should mention null byte"
        );
    }

    #[test]
    fn builder_stores_bind_callback() {
        unsafe extern "C" fn dummy_bind(_info: duckdb_copy_function_bind_info) {}
        let builder = CopyFunctionBuilder::try_new("fmt")
            .unwrap()
            .bind(dummy_bind);
        assert_eq!(builder.name(), "fmt");
    }

    #[test]
    fn builder_stores_global_init_callback() {
        unsafe extern "C" fn dummy_init(_info: duckdb_copy_function_global_init_info) {}
        let builder = CopyFunctionBuilder::try_new("fmt")
            .unwrap()
            .global_init(dummy_init);
        assert_eq!(builder.name(), "fmt");
    }

    #[test]
    fn builder_stores_sink_callback() {
        unsafe extern "C" fn dummy_sink(
            _info: duckdb_copy_function_sink_info,
            _chunk: duckdb_data_chunk,
        ) {
        }
        let builder = CopyFunctionBuilder::try_new("fmt")
            .unwrap()
            .sink(dummy_sink);
        assert_eq!(builder.name(), "fmt");
    }

    #[test]
    fn builder_stores_finalize_callback() {
        unsafe extern "C" fn dummy_finalize(_info: duckdb_copy_function_finalize_info) {}
        let builder = CopyFunctionBuilder::try_new("fmt")
            .unwrap()
            .finalize(dummy_finalize);
        assert_eq!(builder.name(), "fmt");
    }

    #[test]
    fn full_builder_chain_compiles() {
        unsafe extern "C" fn bind(_: duckdb_copy_function_bind_info) {}
        unsafe extern "C" fn init(_: duckdb_copy_function_global_init_info) {}
        unsafe extern "C" fn sink(_: duckdb_copy_function_sink_info, _: duckdb_data_chunk) {}
        unsafe extern "C" fn finalize(_: duckdb_copy_function_finalize_info) {}

        let builder = CopyFunctionBuilder::try_new("my_format")
            .unwrap()
            .bind(bind)
            .global_init(init)
            .sink(sink)
            .finalize(finalize);
        assert_eq!(builder.name(), "my_format");
    }

    #[test]
    fn builder_stores_all_callbacks() {
        unsafe extern "C" fn my_bind(_: duckdb_copy_function_bind_info) {}
        unsafe extern "C" fn my_init(_: duckdb_copy_function_global_init_info) {}
        unsafe extern "C" fn my_sink(_: duckdb_copy_function_sink_info, _: duckdb_data_chunk) {}
        unsafe extern "C" fn my_finalize(_: duckdb_copy_function_finalize_info) {}

        let b = CopyFunctionBuilder::try_new("f")
            .unwrap()
            .bind(my_bind)
            .global_init(my_init)
            .sink(my_sink)
            .finalize(my_finalize);
        assert!(b.bind.is_some());
        assert!(b.global_init.is_some());
        assert!(b.sink.is_some());
        assert!(b.finalize.is_some());
    }

    #[test]
    fn try_new_stores_name() {
        let b = CopyFunctionBuilder::try_new("my_copy").unwrap();
        assert_eq!(b.name(), "my_copy");
    }

    #[test]
    fn callbacks_default_to_none() {
        let b = CopyFunctionBuilder::try_new("fmt").unwrap();
        assert!(b.bind.is_none());
        assert!(b.global_init.is_none());
        assert!(b.sink.is_none());
        assert!(b.finalize.is_none());
    }
}
