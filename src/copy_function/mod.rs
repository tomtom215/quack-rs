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
use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_connection, duckdb_copy_function_bind_info, duckdb_copy_function_finalize_info,
    duckdb_copy_function_global_init_info, duckdb_copy_function_set_bind,
    duckdb_copy_function_set_copy_from_function, duckdb_copy_function_set_extra_info,
    duckdb_copy_function_set_finalize, duckdb_copy_function_set_global_init,
    duckdb_copy_function_set_name, duckdb_copy_function_set_sink, duckdb_copy_function_sink_info,
    duckdb_create_copy_function, duckdb_data_chunk, duckdb_delete_callback_t,
    duckdb_destroy_copy_function, duckdb_register_copy_function, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::table::TableFunctionHandle;
use crate::types::TypeId;

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
    copy_from: Option<TableFunctionHandle>,
    extra_info: Option<crate::extra_info::ExtraInfo>,
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
            copy_from: None,
            extra_info: None,
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

    /// Attaches a table function that implements `COPY … FROM` for this format.
    ///
    /// Without it a copy function is export-only: `COPY t TO 'f' (FORMAT mine)`
    /// works and `COPY t FROM 'f' (FORMAT mine)` reports that the format does
    /// not support reading.
    ///
    /// # How `DuckDB` drives it
    ///
    /// The table function is not called the way a `FROM my_func(...)` one is.
    /// `CCopyFromBind` synthesises the call:
    ///
    /// - the file path arrives as **positional parameter 0**, a `VARCHAR`;
    /// - every option in `COPY … FROM 'f' (FORMAT mine, X 1, Y 'z')` arrives as
    ///   a **named parameter**. An option the table function did not declare is
    ///   a bind error, so declare each one with
    ///   [`named_param`][crate::table::TableFunctionBuilder::named_param];
    /// - the **target table's schema is already fixed**, because `COPY … FROM`
    ///   loads into an existing table. `duckdb.h` is explicit that the bind
    ///   callback "should not define its own result columns using
    ///   `duckdb_bind_add_result_column`" and should read the expected schema
    ///   from [`BindInfo::result_column_count`][crate::table::BindInfo::result_column_count]
    ///   and its siblings instead.
    ///
    /// `DuckDB` copies the table function here, so the handle may be dropped
    /// immediately afterwards.
    ///
    /// # Name the reader after the format
    ///
    /// A bad `COPY … FROM` option is reported as
    /// `'X' is not a supported option for copy function 'NAME'`, where `NAME`
    /// is the **table function's** name, not the copy function's:
    /// `CCopyFromBind` reads `info.tf.name`, and
    /// `duckdb_copy_function_set_copy_from_function` only substitutes the copy
    /// function's name when the table function has none — which
    /// [`TableFunctionBuilder`][crate::table::TableFunctionBuilder] never
    /// produces. So a reader called `my_format_reader` attached to a format
    /// called `csv2` tells the user about `csv2`'s options under the name
    /// `my_format_reader`. Name the two alike.
    ///
    /// # Errors
    ///
    /// `duckdb.h` states the precondition plainly — "the table function must
    /// take a single VARCHAR parameter (the file path)" — and nothing enforces
    /// it. `duckdb_copy_function_set_copy_from_function` never looks at
    /// `tf.arguments` beyond rejecting `INVALID` types, and `CCopyFromBind`
    /// builds the argument list itself: it pushes exactly one `Value` holding
    /// the path, whatever the function declared. So a reader that declares two
    /// parameters, or one that is not `VARCHAR`, does not fail here — it fails
    /// later, inside its own bind callback, reading an argument that is not what
    /// it asked for. That is what this check turns into an error you get at
    /// registration time.
    ///
    /// The other preconditions need no check here: `DuckDB` silently does
    /// nothing if the table function is missing its bind, init or scan callback
    /// — which
    /// [`build_handle`][crate::table::TableFunctionBuilder::build_handle]
    /// already refuses to produce — or if an argument or named-parameter type
    /// contains `INVALID` anywhere, including nested inside a `LIST` or
    /// `STRUCT`, which the builders reject at every reachable entry point (see
    /// [`TypeId::is_composite`][crate::types::TypeId::is_composite]). Only a
    /// type built from a raw handle could still smuggle one through.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::copy_function::CopyFunctionBuilder;
    /// use quack_rs::table::TableFunctionBuilder;
    /// use quack_rs::types::TypeId;
    ///
    /// # fn demo(
    /// #     bind: quack_rs::table::BindFn,
    /// #     init: quack_rs::table::InitFn,
    /// #     scan: quack_rs::table::ScanFn,
    /// # ) -> Result<(), quack_rs::error::ExtensionError> {
    /// // SAFETY: the callbacks match their declared signatures.
    /// let reader = unsafe {
    ///     TableFunctionBuilder::new("my_format_reader")
    ///         .param(TypeId::Varchar) // the file path
    ///         .named_param("skip_rows", TypeId::BigInt)
    ///         .bind(bind)
    ///         .init(init)
    ///         .scan(scan)
    ///         .build_handle()
    /// }?;
    ///
    /// let _copy = CopyFunctionBuilder::try_new("my_format")?.copy_from(reader)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn copy_from(
        mut self,
        table_function: TableFunctionHandle,
    ) -> Result<Self, ExtensionError> {
        let params = table_function.param_types();
        if params.len() != 1 || params[0] != Some(TypeId::Varchar) {
            let described = if params.is_empty() {
                String::from("none")
            } else {
                params
                    .iter()
                    .map(|p| p.map_or("<unknown>", TypeId::sql_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(ExtensionError::new(format!(
                "copy_from: the table function '{}' declares {} positional parameter(s) ({described}), \
                 but DuckDB calls a COPY ... FROM reader with exactly one VARCHAR — the file path. \
                 Declare `.param(TypeId::Varchar)` and nothing else; COPY options arrive as named \
                 parameters. DuckDB does not check this itself, so it is checked here.",
                table_function.name(),
                params.len(),
            )));
        }
        self.copy_from = Some(table_function);
        Ok(self)
    }

    /// Attaches arbitrary data to this copy function.
    ///
    /// Available in every callback via `get_extra_info`. The allocation belongs
    /// to the builder until [`register`][Self::register] hands it to `DuckDB`;
    /// a builder that is dropped instead runs `destroy` itself rather than
    /// leaking it.
    ///
    /// # Safety
    ///
    /// - `data` must be a pointer `destroy` can free exactly once.
    /// - `destroy` must not panic: it is an `extern "C" fn`, so an unwind out of
    ///   it aborts the process at its own boundary. Use
    ///   [`catch_ffi_panic`][crate::callback::catch_ffi_panic] inside it.
    pub unsafe fn extra_info(
        mut self,
        data: *mut c_void,
        destroy: duckdb_delete_callback_t,
    ) -> Self {
        // SAFETY: forwarded from this method's own contract.
        self.extra_info = Some(unsafe { crate::extra_info::ExtraInfo::new(data, destroy) });
        self
    }

    /// Registers the copy function on the given connection.
    ///
    /// A copy function may implement either direction, or both:
    ///
    /// | Direction | Requires |
    /// |---|---|
    /// | `COPY … TO`   | [`bind`][Self::bind] + [`sink`][Self::sink] + [`finalize`][Self::finalize] (and optionally [`global_init`][Self::global_init]) |
    /// | `COPY … FROM` | [`copy_from`][Self::copy_from] |
    ///
    /// A read-only format sets only `copy_from` and leaves the writing
    /// callbacks unset — `duckdb_register_copy_function` accepts that, because
    /// it decides what a copy function supports from
    /// `info.sink != nullptr` and `copy_from_bind != nullptr` independently.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if neither direction is implemented, if
    /// `COPY … TO` is only partly implemented, or if registration fails.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // A copy function may implement `COPY ... TO`, `COPY ... FROM`, or both.
        // `duckdb_register_copy_function` decides which by looking at the sink
        // (`is_copy_to = info.sink != nullptr`) and the reader
        // (`is_copy_from = copy_from_bind != nullptr`), and refuses — with no
        // message — a function that implements neither. Hence the check here.
        let writer =
            match (self.bind, self.sink, self.finalize) {
                (Some(bind), Some(sink), Some(finalize)) => Some((bind, sink, finalize)),
                (None, None, None) if self.copy_from.is_some() => None,
                (_, _, _) if self.copy_from.is_some() => return Err(ExtensionError::new(
                    "copy function implements COPY ... TO only partially: bind, sink and finalize \
                     must all be set. Leave all three unset for a read-only format that supports \
                     COPY ... FROM alone.",
                )),
                _ => {
                    return Err(ExtensionError::new(
                        "copy function implements nothing: set bind, sink and finalize for \
                     COPY ... TO, or copy_from for COPY ... FROM, or both",
                    ))
                }
            };

        // SAFETY: duckdb_create_copy_function allocates a new handle.
        let func = unsafe { duckdb_create_copy_function() };

        // SAFETY: func is a valid newly created handle.
        unsafe {
            duckdb_copy_function_set_name(func, self.name.as_ptr());
        }

        if let Some((bind, sink, finalize)) = writer {
            // SAFETY: func is a valid newly created handle.
            unsafe {
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
        }

        // Attach the COPY ... FROM reader, if any. DuckDB copies the table
        // function by value here, so `reader` can be dropped afterwards.
        if let Some(ref reader) = self.copy_from {
            // SAFETY: both handles are live and the preconditions were checked
            // in `copy_from`.
            unsafe {
                duckdb_copy_function_set_copy_from_function(func, reader.as_raw());
            }
        }

        if let Some(ref info) = self.extra_info {
            // SAFETY: func is valid; data and destroy came from the caller.
            unsafe {
                duckdb_copy_function_set_extra_info(func, info.data(), info.destroy());
                // DuckDB owns the allocation from here.
                info.mark_transferred();
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
            .field("copy_from", &self.copy_from)
            .field("extra_info", &self.extra_info)
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
