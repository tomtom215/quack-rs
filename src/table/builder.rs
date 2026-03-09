// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` table functions.
//!
//! Table functions are the backbone of "real" `DuckDB` extensions: they are
//! SELECT-able, support projection pushdown, named parameters, and can
//! produce arbitrary output schemas determined at query-parse time.
//!
//! # Table function lifecycle
//!
//! ```text
//! 1. bind       — parse args, declare output columns, optionally set cardinality hint
//! 2. init       — allocate global scan state (shared across threads)
//! 3. local_init — allocate per-thread scan state (optional)
//! 4. scan       — fill one output chunk; set chunk size to 0 when exhausted
//! ```
//!
//! # Example: A constant table function
//!
//! ```rust,no_run
//! use quack_rs::table::{TableFunctionBuilder, FfiBindData, FfiInitData};
//! use quack_rs::types::TypeId;
//! use libduckdb_sys::{
//!     duckdb_bind_info, duckdb_init_info, duckdb_function_info,
//!     duckdb_data_chunk, duckdb_data_chunk_set_size,
//! };
//!
//! struct Config { limit: u64 }
//! struct State  { emitted: u64 }
//!
//! unsafe extern "C" fn bind(info: duckdb_bind_info) {
//!     unsafe {
//!         // Declare the output schema.
//!         quack_rs::table::BindInfo::new(info)
//!             .add_result_column("n", TypeId::BigInt);
//!         // Store bind-time configuration.
//!         FfiBindData::<Config>::set(info, Config { limit: 100 });
//!     }
//! }
//!
//! unsafe extern "C" fn init(info: duckdb_init_info) {
//!     unsafe { FfiInitData::<State>::set(info, State { emitted: 0 }); }
//! }
//!
//! unsafe extern "C" fn scan(info: duckdb_function_info, output: duckdb_data_chunk) {
//!     // scan logic
//! }
//!
//! // fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
//! //     unsafe {
//! //         TableFunctionBuilder::new("my_table_fn")
//! //             .bind(bind)
//! //             .init(init)
//! //             .scan(scan)
//! //             .register(con)
//! //     }
//! // }
//! ```

use std::ffi::CString;
use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_bind_info, duckdb_connection, duckdb_create_table_function, duckdb_data_chunk,
    duckdb_destroy_table_function, duckdb_function_info, duckdb_init_info,
    duckdb_register_table_function, duckdb_table_function_add_named_parameter,
    duckdb_table_function_add_parameter, duckdb_table_function_set_bind,
    duckdb_table_function_set_extra_info, duckdb_table_function_set_function,
    duckdb_table_function_set_init, duckdb_table_function_set_local_init,
    duckdb_table_function_set_name, duckdb_table_function_supports_projection_pushdown,
    DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};
use crate::validate::validate_function_name;

/// The bind callback: declare output columns, read parameters, store bind data.
pub type BindFn = unsafe extern "C" fn(info: duckdb_bind_info);

/// The init callback: allocate global scan state.
pub type InitFn = unsafe extern "C" fn(info: duckdb_init_info);

/// The scan callback: fill one output chunk; set chunk size to 0 when done.
pub type ScanFn = unsafe extern "C" fn(info: duckdb_function_info, output: duckdb_data_chunk);

/// The extra-info destructor callback: called by `DuckDB` to free function-level extra data.
pub type ExtraDestroyFn = unsafe extern "C" fn(data: *mut c_void);

/// A named parameter specification: (name, type).
struct NamedParam {
    name: CString,
    type_id: TypeId,
}

/// Builder for registering a `DuckDB` table function.
///
/// Table functions are the most powerful extension type — they can return
/// arbitrary result schemas, support named parameters, projection pushdown,
/// and parallel execution.
///
/// # Required fields
///
/// - [`bind`][TableFunctionBuilder::bind]: must be set.
/// - [`init`][TableFunctionBuilder::init]: must be set.
/// - [`scan`][TableFunctionBuilder::scan]: must be set.
///
/// # Optional features
///
/// - [`param`][TableFunctionBuilder::param]: positional parameters.
/// - [`named_param`][TableFunctionBuilder::named_param]: named parameters (`name := value`).
/// - [`local_init`][TableFunctionBuilder::local_init]: per-thread init (enables parallel scan).
/// - [`projection_pushdown`][TableFunctionBuilder::projection_pushdown]: hint projection info to `DuckDB`.
/// - [`extra_info`][TableFunctionBuilder::extra_info]: function-level data available in all callbacks.
#[must_use]
pub struct TableFunctionBuilder {
    name: CString,
    params: Vec<TypeId>,
    named_params: Vec<NamedParam>,
    bind: Option<BindFn>,
    init: Option<InitFn>,
    local_init: Option<InitFn>,
    scan: Option<ScanFn>,
    projection_pushdown: bool,
    extra_info: Option<(*mut c_void, ExtraDestroyFn)>,
}

impl TableFunctionBuilder {
    /// Creates a new builder for a table function with the given name.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    pub fn new(name: &str) -> Self {
        Self {
            name: CString::new(name).expect("function name must not contain null bytes"),
            params: Vec::new(),
            named_params: Vec::new(),
            bind: None,
            init: None,
            local_init: None,
            scan: None,
            projection_pushdown: false,
            extra_info: None,
        }
    }

    /// Creates a new builder with function name validation.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the name is invalid.
    pub fn try_new(name: &str) -> Result<Self, ExtensionError> {
        validate_function_name(name)?;
        let c_name = CString::new(name)
            .map_err(|_| ExtensionError::new("function name contains interior null byte"))?;
        Ok(Self {
            name: c_name,
            params: Vec::new(),
            named_params: Vec::new(),
            bind: None,
            init: None,
            local_init: None,
            scan: None,
            projection_pushdown: false,
            extra_info: None,
        })
    }

    /// Adds a positional parameter with the given type.
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.params.push(type_id);
        self
    }

    /// Adds a named parameter (e.g., `my_fn(path := 'data.csv')`).
    ///
    /// Named parameters are accessed in the bind callback via
    /// `duckdb_bind_get_named_parameter`.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    pub fn named_param(mut self, name: &str, type_id: TypeId) -> Self {
        self.named_params.push(NamedParam {
            name: CString::new(name).expect("parameter name must not contain null bytes"),
            type_id,
        });
        self
    }

    /// Sets the bind callback.
    ///
    /// The bind callback is called once at query-parse time. It must:
    /// - Declare all output columns via [`BindInfo::add_result_column`].
    /// - Optionally read parameters and store bind data via [`crate::table::FfiBindData::set`].
    pub fn bind(mut self, f: BindFn) -> Self {
        self.bind = Some(f);
        self
    }

    /// Sets the global init callback.
    ///
    /// Called once per query. Use [`crate::table::FfiInitData::set`] to store global scan state.
    pub fn init(mut self, f: InitFn) -> Self {
        self.init = Some(f);
        self
    }

    /// Sets the per-thread local init callback (optional).
    ///
    /// When set, `DuckDB` calls this once per worker thread. Use [`crate::table::FfiLocalInitData::set`]
    /// to store thread-local scan state. Setting a local init enables parallel scanning.
    pub fn local_init(mut self, f: InitFn) -> Self {
        self.local_init = Some(f);
        self
    }

    /// Sets the scan callback.
    ///
    /// Called repeatedly until all rows are produced. Set the output chunk's size
    /// to `0` (via `duckdb_data_chunk_set_size(output, 0)`) to signal end of stream.
    pub fn scan(mut self, f: ScanFn) -> Self {
        self.scan = Some(f);
        self
    }

    /// Enables or disables projection pushdown support (default: disabled).
    ///
    /// When enabled, `DuckDB` informs the `init` callback which columns were
    /// requested. Use `duckdb_init_get_column_count` and `duckdb_init_get_column_index`
    /// in your init callback to skip producing unrequested columns.
    pub const fn projection_pushdown(mut self, enable: bool) -> Self {
        self.projection_pushdown = enable;
        self
    }

    /// Sets function-level extra info shared across all callbacks.
    ///
    /// This data is available via `duckdb_function_get_extra_info` and
    /// `duckdb_bind_get_extra_info` in all callbacks. The `destroy` callback
    /// is called by `DuckDB` when the function is dropped.
    ///
    /// # Safety
    ///
    /// `data` must remain valid until `DuckDB` calls `destroy`. The typical pattern
    /// is to box your data: `Box::into_raw(Box::new(my_data)).cast()`.
    pub unsafe fn extra_info(mut self, data: *mut c_void, destroy: ExtraDestroyFn) -> Self {
        self.extra_info = Some((data, destroy));
        self
    }

    /// Registers the table function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - The bind, init, or scan callback was not set.
    /// - `DuckDB` reports a registration failure.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let bind = self
            .bind
            .ok_or_else(|| ExtensionError::new("bind callback not set"))?;
        let init = self
            .init
            .ok_or_else(|| ExtensionError::new("init callback not set"))?;
        let scan = self
            .scan
            .ok_or_else(|| ExtensionError::new("scan callback not set"))?;

        // SAFETY: creates a new table function handle.
        let func = unsafe { duckdb_create_table_function() };

        // SAFETY: func is a valid newly created handle.
        unsafe {
            duckdb_table_function_set_name(func, self.name.as_ptr());
        }

        // Add positional parameters.
        for type_id in &self.params {
            let lt = LogicalType::new(*type_id);
            // SAFETY: func and lt.as_raw() are valid.
            unsafe {
                duckdb_table_function_add_parameter(func, lt.as_raw());
            }
        }

        // Add named parameters.
        for np in &self.named_params {
            let lt = LogicalType::new(np.type_id);
            // SAFETY: func, name ptr, and lt.as_raw() are valid.
            unsafe {
                duckdb_table_function_add_named_parameter(func, np.name.as_ptr(), lt.as_raw());
            }
        }

        // Set callbacks.
        // SAFETY: func is valid; callbacks are valid extern "C" fn pointers.
        unsafe {
            duckdb_table_function_set_bind(func, Some(bind));
            duckdb_table_function_set_init(func, Some(init));
            duckdb_table_function_set_function(func, Some(scan));
        }

        // Set optional local init.
        if let Some(local_init) = self.local_init {
            // SAFETY: func is valid; local_init is a valid extern "C" fn pointer.
            unsafe {
                duckdb_table_function_set_local_init(func, Some(local_init));
            }
        }

        // Configure projection pushdown.
        // SAFETY: func is valid.
        unsafe {
            duckdb_table_function_supports_projection_pushdown(func, self.projection_pushdown);
        }

        // Set extra info if provided.
        if let Some((data, destroy)) = self.extra_info {
            // SAFETY: func is valid; data and destroy are provided by caller.
            unsafe {
                duckdb_table_function_set_extra_info(func, data, Some(destroy));
            }
        }

        // Register.
        // SAFETY: con and func are valid.
        let result = unsafe { duckdb_register_table_function(con, func) };

        // Always destroy the function handle; ownership transferred to DuckDB on success.
        // SAFETY: func was created above.
        unsafe {
            duckdb_destroy_table_function(&mut { func });
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_table_function failed for '{}'",
                self.name.to_string_lossy()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_stores_name() {
        let b = TableFunctionBuilder::new("my_table_fn");
        assert_eq!(b.name.to_str().unwrap(), "my_table_fn");
    }

    #[test]
    fn builder_stores_params() {
        let b = TableFunctionBuilder::new("f")
            .param(TypeId::Varchar)
            .param(TypeId::BigInt);
        assert_eq!(b.params.len(), 2);
        assert_eq!(b.params[0], TypeId::Varchar);
        assert_eq!(b.params[1], TypeId::BigInt);
    }

    #[test]
    fn builder_stores_named_params() {
        let b = TableFunctionBuilder::new("f")
            .named_param("path", TypeId::Varchar)
            .named_param("limit", TypeId::BigInt);
        assert_eq!(b.named_params.len(), 2);
        assert_eq!(b.named_params[0].name.to_str().unwrap(), "path");
        assert_eq!(b.named_params[1].name.to_str().unwrap(), "limit");
    }

    #[test]
    fn builder_stores_callbacks() {
        unsafe extern "C" fn my_bind(_: duckdb_bind_info) {}
        unsafe extern "C" fn my_init(_: duckdb_init_info) {}
        unsafe extern "C" fn my_scan(_: duckdb_function_info, _: duckdb_data_chunk) {}

        let b = TableFunctionBuilder::new("f")
            .bind(my_bind)
            .init(my_init)
            .scan(my_scan);
        assert!(b.bind.is_some());
        assert!(b.init.is_some());
        assert!(b.scan.is_some());
    }

    #[test]
    fn builder_projection_pushdown() {
        let b = TableFunctionBuilder::new("f").projection_pushdown(true);
        assert!(b.projection_pushdown);
    }

    #[test]
    fn try_new_valid_name() {
        assert!(TableFunctionBuilder::try_new("read_csv_ext").is_ok());
    }

    #[test]
    fn try_new_invalid_name() {
        assert!(TableFunctionBuilder::try_new("").is_err());
        assert!(TableFunctionBuilder::try_new("MyFunc").is_err());
    }

    #[test]
    fn try_new_null_byte_rejected() {
        assert!(TableFunctionBuilder::try_new("func\0name").is_err());
    }
}
