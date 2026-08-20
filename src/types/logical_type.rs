// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! RAII wrapper for `duckdb_logical_type`.
//!
//! # Pitfall L7: `LogicalType` memory leak
//!
//! Every `duckdb_create_logical_type` call allocates memory that must be freed
//! with `duckdb_destroy_logical_type`. Forgetting to call the destructor leaks
//! memory. [`LogicalType`] implements `Drop` to prevent this.

use crate::types::TypeId;
use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_array_type_child_type, duckdb_create_array_type,
    duckdb_create_decimal_type, duckdb_create_enum_type, duckdb_create_list_type,
    duckdb_create_logical_type, duckdb_create_map_type, duckdb_create_struct_type,
    duckdb_create_union_type, duckdb_decimal_internal_type, duckdb_decimal_scale,
    duckdb_decimal_width, duckdb_destroy_logical_type, duckdb_enum_dictionary_size,
    duckdb_enum_dictionary_value, duckdb_enum_internal_type, duckdb_free, duckdb_get_type_id,
    duckdb_list_type_child_type, duckdb_logical_type, duckdb_logical_type_get_alias,
    duckdb_logical_type_set_alias, duckdb_map_type_key_type, duckdb_map_type_value_type,
    duckdb_struct_type_child_count, duckdb_struct_type_child_name, duckdb_struct_type_child_type,
    duckdb_union_type_member_count, duckdb_union_type_member_name, duckdb_union_type_member_type,
};
use std::fmt;

/// Error returned by the fallible [`LogicalType`] constructors.
///
/// Two things go wrong when building a logical type, and this reports both:
///
/// - the underlying `DuckDB` C API returned a null pointer (an out-of-range
///   `DECIMAL` width, for instance), or
/// - the requested type could not be built from a bare [`TypeId`] at all — see
///   [`TypeId::is_composite`][crate::types::TypeId::is_composite].
#[derive(Debug, Clone)]
pub struct LogicalTypeError {
    api_func: &'static str,
    /// When set, the whole message; otherwise `"<api_func> returned null"`.
    detail: Option<String>,
}

impl LogicalTypeError {
    /// The `DuckDB` C API function (or internal step) that failed.
    #[must_use]
    pub const fn api_func(&self) -> &'static str {
        self.api_func
    }

    /// Builds the "returned null" form.
    const fn null(api_func: &'static str) -> Self {
        Self {
            api_func,
            detail: None,
        }
    }
}

impl fmt::Display for LogicalTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => f.write_str(detail),
            None => write!(f, "{} returned null", self.api_func),
        }
    }
}

impl std::error::Error for LogicalTypeError {}

/// The diagnostic for a composite [`TypeId`] used where a primitive was needed.
///
/// Shared by [`LogicalType::new`]'s panic, [`LogicalType::try_new`]'s error, and
/// every builder that turns a `TypeId` into a parameter or return type, so the
/// advice is identical wherever the mistake is caught.
fn composite_message(type_id: TypeId) -> String {
    let hint = type_id
        .composite_constructor_hint()
        .unwrap_or("a dedicated LogicalType constructor");
    format!(
        "{} carries parameters that a bare TypeId cannot express, so \
         duckdb_create_logical_type would return an invalid (non-null) type. \
         Build it with {hint} and pass the result through the `*_logical` \
         variant of this method (e.g. `param_logical` / `returns_logical`).",
        type_id.sql_name()
    )
}

/// An RAII wrapper around a `duckdb_logical_type` handle.
///
/// Created from a [`TypeId`], this type ensures `duckdb_destroy_logical_type`
/// is called when it is dropped. This prevents the memory leak described in
/// [Pitfall L7](https://github.com/tomtom215/quack-rs/blob/main/LESSONS.md).
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::types::{LogicalType, TypeId};
///
/// // Requires DuckDB runtime to be initialized (i.e., loaded as an extension).
/// let lt = LogicalType::new(TypeId::BigInt);
/// // `lt` is automatically destroyed when it goes out of scope
/// ```
pub struct LogicalType {
    inner: duckdb_logical_type,
}

impl LogicalType {
    /// Creates a `LogicalType` from an existing raw `duckdb_logical_type` handle.
    ///
    /// The returned `LogicalType` takes ownership of the handle and will call
    /// `duckdb_destroy_logical_type` when dropped.
    ///
    /// # Safety
    ///
    /// - `ptr` must be a valid, non-null `duckdb_logical_type` handle returned by
    ///   a `duckdb_create_*` function (e.g. `duckdb_create_logical_type`,
    ///   `duckdb_create_list_type`, `duckdb_create_struct_type`, etc.).
    /// - The caller must not call `duckdb_destroy_logical_type` on the handle
    ///   after passing it to this function.
    /// - The handle must not be used after this call except through the returned
    ///   `LogicalType`.
    ///
    /// # Panics
    ///
    /// Panics if `ptr` is null.
    #[must_use]
    pub unsafe fn from_raw(ptr: duckdb_logical_type) -> Self {
        assert!(
            !ptr.is_null(),
            "LogicalType::from_raw called with null pointer"
        );
        Self { inner: ptr }
    }

    /// Creates a new `LogicalType` for the given **primitive** `TypeId`.
    ///
    /// Calls `duckdb_create_logical_type` internally.
    ///
    /// # Composite types are rejected
    ///
    /// `duckdb_create_logical_type` documents that it "returns an invalid
    /// logical type" for `DECIMAL`, `ENUM`, `LIST`, `STRUCT`, `MAP`, `ARRAY` and
    /// `UNION` — and "invalid" there means a **non-null handle** wrapping
    /// `LogicalTypeId::INVALID`, so a null check does not catch it. Left alone,
    /// that surfaces much later as an opaque `duckdb_register_*_function failed`
    /// or as a panic from [`get_type_id`][Self::get_type_id]. Each of those
    /// types carries parameters a bare id cannot express, so each has its own
    /// constructor; see [`TypeId::is_composite`].
    ///
    /// # Panics
    ///
    /// - Panics if `type_id` is composite, naming the constructor to use
    ///   instead. Use [`try_new`][Self::try_new] to get a `Result`.
    /// - Panics if `duckdb_create_logical_type` returns a null pointer.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // Requires DuckDB runtime (called from within a loaded extension).
    /// let lt = LogicalType::new(TypeId::Timestamp);
    /// assert!(!lt.as_raw().is_null());
    /// ```
    #[must_use]
    pub fn new(type_id: TypeId) -> Self {
        assert!(
            !type_id.is_composite(),
            "LogicalType::new({type_id:?}) would build an invalid type: {}",
            composite_message(type_id)
        );
        // SAFETY: `duckdb_create_logical_type` is safe to call with any valid DUCKDB_TYPE.
        // It returns a heap-allocated handle that must be freed with duckdb_destroy_logical_type.
        let inner = unsafe { duckdb_create_logical_type(type_id.to_duckdb_type()) };
        assert!(!inner.is_null(), "duckdb_create_logical_type returned null");
        Self { inner }
    }

    /// Builds a `LogicalType` for a named builder slot, turning a composite
    /// [`TypeId`] into an [`ExtensionError`][crate::error::ExtensionError] that
    /// says which slot was wrong and what to use instead.
    ///
    /// Without this, a composite id reaches `duckdb_create_logical_type`, comes
    /// back as a non-null but invalid type, and only fails at
    /// `duckdb_register_*_function` with a message that names neither the
    /// offending parameter nor the fix.
    pub(crate) fn for_slot(
        type_id: TypeId,
        slot: &str,
    ) -> Result<Self, crate::error::ExtensionError> {
        Self::try_new(type_id)
            .map_err(|e| crate::error::ExtensionError::new(format!("{slot}: {e}")))
    }

    /// Validates that `type_id` can be turned into a logical type, without
    /// building one.
    ///
    /// Builders call this **before** allocating any `DuckDB` handle, so a bad
    /// type id is reported without leaking a half-built function. Once it has
    /// passed, the [`new`][Self::new] calls further down cannot hit their
    /// composite-type assertion.
    pub(crate) fn check_slot(
        type_id: TypeId,
        slot: &str,
    ) -> Result<(), crate::error::ExtensionError> {
        Self::for_slot(type_id, slot).map(drop)
    }

    /// Creates a `LIST<element_type>` logical type.
    ///
    /// Lists are variable-length sequences of the given element type.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // Requires DuckDB runtime.
    /// let list_of_int = LogicalType::list(TypeId::Integer);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_list_type` returns null (should never happen).
    #[must_use]
    pub fn list(element_type: TypeId) -> Self {
        let element_lt = Self::new(element_type);
        // SAFETY: element_lt.as_raw() is a valid logical type.
        let inner = unsafe { duckdb_create_list_type(element_lt.as_raw()) };
        assert!(!inner.is_null(), "duckdb_create_list_type returned null");
        Self { inner }
    }

    /// Creates a `MAP<key_type, value_type>` logical type.
    ///
    /// `DuckDB` maps are stored as `LIST<STRUCT{key: K, value: V}>`.
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_map_type` returns null.
    #[must_use]
    pub fn map(key_type: TypeId, value_type: TypeId) -> Self {
        let key_lt = Self::new(key_type);
        let val_lt = Self::new(value_type);
        // SAFETY: both logical types are valid.
        let inner = unsafe { duckdb_create_map_type(key_lt.as_raw(), val_lt.as_raw()) };
        assert!(!inner.is_null(), "duckdb_create_map_type returned null");
        Self { inner }
    }

    /// Creates a `STRUCT` logical type from a slice of `(name, type)` field definitions.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // Requires DuckDB runtime.
    /// let point = LogicalType::struct_type(&[
    ///     ("x", TypeId::Double),
    ///     ("y", TypeId::Double),
    /// ]);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if any field name contains an interior null byte, or if
    /// `duckdb_create_struct_type` returns null.
    #[must_use]
    pub fn struct_type(fields: &[(&str, TypeId)]) -> Self {
        use std::ffi::CString;

        // Build arrays of logical type handles and C name pointers.
        // The logical types must outlive the duckdb_create_struct_type call.
        let field_types: Vec<Self> = fields.iter().map(|&(_, t)| Self::new(t)).collect();
        let c_names: Vec<CString> = fields
            .iter()
            .map(|&(n, _)| CString::new(n).expect("field name must not contain null bytes"))
            .collect();

        let mut type_ptrs: Vec<duckdb_logical_type> =
            field_types.iter().map(Self::as_raw).collect();
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: type_ptrs and name_ptrs are valid for the duration of this call.
        let inner = unsafe {
            duckdb_create_struct_type(
                type_ptrs.as_mut_ptr(),
                name_ptrs.as_mut_ptr(),
                fields.len() as libduckdb_sys::idx_t,
            )
        };
        assert!(!inner.is_null(), "duckdb_create_struct_type returned null");
        Self { inner }
    }

    /// Creates a `DECIMAL(width, scale)` logical type.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::LogicalType;
    ///
    /// // DECIMAL(18, 3) — 18 total digits, 3 after the decimal point
    /// let price = LogicalType::decimal(18, 3);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_decimal_type` returns null.
    #[must_use]
    pub fn decimal(width: u8, scale: u8) -> Self {
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe { duckdb_create_decimal_type(width, scale) };
        assert!(!inner.is_null(), "duckdb_create_decimal_type returned null");
        Self { inner }
    }

    /// Creates an `ARRAY<element_type>[size]` logical type (fixed-size array).
    ///
    /// Unlike `LIST`, arrays have a fixed number of elements known at type
    /// definition time.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // FLOAT[3] — a 3-element array of floats (e.g., for a 3D vector)
    /// let vec3 = LogicalType::array(TypeId::Float, 3);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_array_type` returns null.
    #[must_use]
    pub fn array(element_type: TypeId, size: u64) -> Self {
        let element_lt = Self::new(element_type);
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner =
            unsafe { duckdb_create_array_type(element_lt.as_raw(), size as libduckdb_sys::idx_t) };
        assert!(!inner.is_null(), "duckdb_create_array_type returned null");
        Self { inner }
    }

    /// Creates an `ARRAY<element>[size]` logical type from an existing [`LogicalType`].
    ///
    /// Use this when the element type is itself a complex type.
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_array_type` returns null.
    #[must_use]
    pub fn array_from_logical(element: &Self, size: u64) -> Self {
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner =
            unsafe { duckdb_create_array_type(element.as_raw(), size as libduckdb_sys::idx_t) };
        assert!(!inner.is_null(), "duckdb_create_array_type returned null");
        Self { inner }
    }

    /// Creates a `UNION` logical type from a slice of `(name, type)` member definitions.
    ///
    /// A `UNION` can hold one value of any of its member types at a time,
    /// similar to a tagged union or sum type.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// let result = LogicalType::union_type(&[
    ///     ("str", TypeId::Varchar),
    ///     ("num", TypeId::BigInt),
    /// ]);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if any member name contains an interior null byte, or if
    /// `duckdb_create_union_type` returns null.
    #[must_use]
    pub fn union_type(members: &[(&str, TypeId)]) -> Self {
        use std::ffi::CString;

        let member_types: Vec<Self> = members.iter().map(|&(_, t)| Self::new(t)).collect();
        let c_names: Vec<CString> = members
            .iter()
            .map(|&(n, _)| CString::new(n).expect("member name must not contain null bytes"))
            .collect();

        let mut type_ptrs: Vec<duckdb_logical_type> =
            member_types.iter().map(Self::as_raw).collect();
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_union_type(
                type_ptrs.as_mut_ptr(),
                name_ptrs.as_mut_ptr(),
                members.len() as libduckdb_sys::idx_t,
            )
        };
        assert!(!inner.is_null(), "duckdb_create_union_type returned null");
        Self { inner }
    }

    /// Creates a `UNION` logical type from a slice of `(name, LogicalType)` members.
    ///
    /// Use this when members have complex types.
    ///
    /// # Panics
    ///
    /// Panics if any member name contains an interior null byte, or if
    /// `duckdb_create_union_type` returns null.
    #[must_use]
    pub fn union_type_from_logical(members: &[(&str, Self)]) -> Self {
        use std::ffi::CString;

        let c_names: Vec<CString> = members
            .iter()
            .map(|&(n, _)| CString::new(n).expect("member name must not contain null bytes"))
            .collect();

        let mut type_ptrs: Vec<duckdb_logical_type> =
            members.iter().map(|(_, lt)| lt.as_raw()).collect();
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_union_type(
                type_ptrs.as_mut_ptr(),
                name_ptrs.as_mut_ptr(),
                members.len() as libduckdb_sys::idx_t,
            )
        };
        assert!(!inner.is_null(), "duckdb_create_union_type returned null");
        Self { inner }
    }

    /// Creates an `ENUM` logical type from a list of member names.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::LogicalType;
    ///
    /// let color = LogicalType::enum_type(&["red", "green", "blue"]);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if any name contains an interior null byte, or if
    /// `duckdb_create_enum_type` returns null.
    #[must_use]
    pub fn enum_type(members: &[&str]) -> Self {
        use std::ffi::CString;

        let c_names: Vec<CString> = members
            .iter()
            .map(|n| CString::new(*n).expect("enum member name must not contain null bytes"))
            .collect();

        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_enum_type(
                name_ptrs.as_mut_ptr(),
                members.len() as libduckdb_sys::idx_t,
            )
        };
        assert!(!inner.is_null(), "duckdb_create_enum_type returned null");
        Self { inner }
    }

    /// Creates a `LIST<element>` logical type from an existing [`LogicalType`].
    ///
    /// Use this when the element type is itself a complex type (e.g.
    /// `LIST(STRUCT(...))`) that cannot be expressed as a simple [`TypeId`].
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_list_type` returns null.
    #[must_use]
    pub fn list_from_logical(element: &Self) -> Self {
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe { duckdb_create_list_type(element.as_raw()) };
        assert!(!inner.is_null(), "duckdb_create_list_type returned null");
        Self { inner }
    }

    /// Creates a `MAP<key, value>` logical type from existing [`LogicalType`]s.
    ///
    /// Use this when the key or value types are complex types that cannot be
    /// expressed as simple [`TypeId`] values.
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_map_type` returns null.
    #[must_use]
    pub fn map_from_logical(key: &Self, value: &Self) -> Self {
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe { duckdb_create_map_type(key.as_raw(), value.as_raw()) };
        assert!(!inner.is_null(), "duckdb_create_map_type returned null");
        Self { inner }
    }

    /// Creates a `STRUCT` logical type from a slice of `(name, LogicalType)` fields.
    ///
    /// Use this when struct members have complex types (e.g.
    /// `STRUCT(headers MAP(VARCHAR, VARCHAR), body VARCHAR)`) that cannot be
    /// expressed as simple [`TypeId`] values.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // STRUCT(status INTEGER, headers MAP(VARCHAR, VARCHAR), body VARCHAR)
    /// let response = LogicalType::struct_type_from_logical(&[
    ///     ("status", LogicalType::new(TypeId::Integer)),
    ///     ("headers", LogicalType::map(TypeId::Varchar, TypeId::Varchar)),
    ///     ("body", LogicalType::new(TypeId::Varchar)),
    /// ]);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if any field name contains an interior null byte, or if
    /// `duckdb_create_struct_type` returns null.
    #[must_use]
    pub fn struct_type_from_logical(fields: &[(&str, Self)]) -> Self {
        use std::ffi::CString;

        let c_names: Vec<CString> = fields
            .iter()
            .map(|&(n, _)| CString::new(n).expect("field name must not contain null bytes"))
            .collect();

        let mut type_ptrs: Vec<duckdb_logical_type> =
            fields.iter().map(|(_, lt)| lt.as_raw()).collect();
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_struct_type(
                type_ptrs.as_mut_ptr(),
                name_ptrs.as_mut_ptr(),
                fields.len() as libduckdb_sys::idx_t,
            )
        };
        assert!(!inner.is_null(), "duckdb_create_struct_type returned null");
        Self { inner }
    }

    /// Fallible version of [`LogicalType::new`].
    ///
    /// Returns an error instead of panicking when `type_id` is composite (see
    /// [`LogicalType::new`]) or when the `DuckDB` C API returns a null pointer.
    pub fn try_new(type_id: TypeId) -> Result<Self, LogicalTypeError> {
        if type_id.is_composite() {
            return Err(LogicalTypeError {
                api_func: "duckdb_create_logical_type",
                detail: Some(composite_message(type_id)),
            });
        }
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe { duckdb_create_logical_type(type_id.to_duckdb_type()) };
        if inner.is_null() {
            return Err(LogicalTypeError::null("duckdb_create_logical_type"));
        }
        Ok(Self { inner })
    }

    /// Fallible version of [`LogicalType::list`]. Returns an error instead of
    /// panicking if the `DuckDB` C API returns a null pointer.
    pub fn try_list(element_type: TypeId) -> Result<Self, LogicalTypeError> {
        let element_lt = Self::try_new(element_type)?;
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe { duckdb_create_list_type(element_lt.as_raw()) };
        if inner.is_null() {
            return Err(LogicalTypeError::null("duckdb_create_list_type"));
        }
        Ok(Self { inner })
    }

    /// Fallible version of [`LogicalType::map`]. Returns an error instead of
    /// panicking if the `DuckDB` C API returns a null pointer.
    pub fn try_map(key_type: TypeId, value_type: TypeId) -> Result<Self, LogicalTypeError> {
        let key_lt = Self::try_new(key_type)?;
        let val_lt = Self::try_new(value_type)?;
        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe { duckdb_create_map_type(key_lt.as_raw(), val_lt.as_raw()) };
        if inner.is_null() {
            return Err(LogicalTypeError::null("duckdb_create_map_type"));
        }
        Ok(Self { inner })
    }

    /// Fallible version of [`LogicalType::struct_type`]. Returns an error
    /// instead of panicking if a field name contains an interior null byte or
    /// if the `DuckDB` C API returns a null pointer.
    pub fn try_struct_type(fields: &[(&str, TypeId)]) -> Result<Self, LogicalTypeError> {
        use std::ffi::CString;

        let field_types: Vec<Self> = fields
            .iter()
            .map(|&(_, t)| Self::try_new(t))
            .collect::<Result<_, _>>()?;
        let c_names: Vec<CString> = fields
            .iter()
            .map(|&(n, _)| {
                CString::new(n).map_err(|_| {
                    LogicalTypeError::null("CString::new (field name contains null byte)")
                })
            })
            .collect::<Result<_, _>>()?;

        let mut type_ptrs: Vec<duckdb_logical_type> =
            field_types.iter().map(Self::as_raw).collect();
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_struct_type(
                type_ptrs.as_mut_ptr(),
                name_ptrs.as_mut_ptr(),
                fields.len() as libduckdb_sys::idx_t,
            )
        };
        if inner.is_null() {
            return Err(LogicalTypeError::null("duckdb_create_struct_type"));
        }
        Ok(Self { inner })
    }

    /// Fallible version of [`LogicalType::struct_type_from_logical`]. Returns an
    /// error instead of panicking if a field name contains an interior null byte
    /// or if the `DuckDB` C API returns a null pointer.
    pub fn try_struct_type_from_logical(fields: &[(&str, Self)]) -> Result<Self, LogicalTypeError> {
        use std::ffi::CString;

        let c_names: Vec<CString> = fields
            .iter()
            .map(|&(n, _)| {
                CString::new(n).map_err(|_| {
                    LogicalTypeError::null("CString::new (field name contains null byte)")
                })
            })
            .collect::<Result<_, _>>()?;

        let mut type_ptrs: Vec<duckdb_logical_type> =
            fields.iter().map(|(_, t)| t.as_raw()).collect();
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_struct_type(
                type_ptrs.as_mut_ptr(),
                name_ptrs.as_mut_ptr(),
                fields.len() as libduckdb_sys::idx_t,
            )
        };
        if inner.is_null() {
            return Err(LogicalTypeError::null("duckdb_create_struct_type"));
        }
        Ok(Self { inner })
    }

    /// Fallible version of [`LogicalType::union_type`]. Returns an error instead
    /// of panicking if a member name contains an interior null byte or if the
    /// `DuckDB` C API returns a null pointer.
    pub fn try_union_type(members: &[(&str, TypeId)]) -> Result<Self, LogicalTypeError> {
        let resolved: Vec<(&str, Self)> = members
            .iter()
            .map(|&(n, t)| Self::try_new(t).map(|lt| (n, lt)))
            .collect::<Result<_, _>>()?;
        Self::try_union_type_from_logical(&resolved)
    }

    /// Fallible version of [`LogicalType::union_type_from_logical`]. Returns an
    /// error instead of panicking if a member name contains an interior null
    /// byte or if the `DuckDB` C API returns a null pointer.
    pub fn try_union_type_from_logical(members: &[(&str, Self)]) -> Result<Self, LogicalTypeError> {
        use std::ffi::CString;

        let c_names: Vec<CString> = members
            .iter()
            .map(|&(n, _)| {
                CString::new(n).map_err(|_| {
                    LogicalTypeError::null("CString::new (union member name contains null byte)")
                })
            })
            .collect::<Result<_, _>>()?;

        let mut type_ptrs: Vec<duckdb_logical_type> =
            members.iter().map(|(_, t)| t.as_raw()).collect();
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_union_type(
                type_ptrs.as_mut_ptr(),
                name_ptrs.as_mut_ptr(),
                members.len() as libduckdb_sys::idx_t,
            )
        };
        if inner.is_null() {
            return Err(LogicalTypeError::null("duckdb_create_union_type"));
        }
        Ok(Self { inner })
    }

    /// Fallible version of [`LogicalType::enum_type`]. Returns an error instead
    /// of panicking if a member name contains an interior null byte or if the
    /// `DuckDB` C API returns a null pointer.
    pub fn try_enum_type(members: &[&str]) -> Result<Self, LogicalTypeError> {
        use std::ffi::CString;

        let c_names: Vec<CString> = members
            .iter()
            .map(|n| {
                CString::new(*n).map_err(|_| {
                    LogicalTypeError::null("CString::new (enum member name contains null byte)")
                })
            })
            .collect::<Result<_, _>>()?;
        let mut name_ptrs: Vec<*const std::os::raw::c_char> =
            c_names.iter().map(|s| s.as_ptr()).collect();

        // SAFETY: every argument is a plain value or a logical-type handle borrowed
        // for the duration of the call. DuckDB returns a newly allocated handle,
        // which the caller checks for null.
        let inner = unsafe {
            duckdb_create_enum_type(
                name_ptrs.as_mut_ptr(),
                members.len() as libduckdb_sys::idx_t,
            )
        };
        if inner.is_null() {
            return Err(LogicalTypeError::null("duckdb_create_enum_type"));
        }
        Ok(Self { inner })
    }

    /// Fallible version of [`LogicalType::set_alias`]. Returns an error instead
    /// of panicking if `alias` contains an interior null byte.
    ///
    /// # Safety
    ///
    /// `self` must wrap a valid `duckdb_logical_type`.
    pub unsafe fn try_set_alias(&self, alias: &str) -> Result<(), LogicalTypeError> {
        let c_alias = std::ffi::CString::new(alias)
            .map_err(|_| LogicalTypeError::null("CString::new (alias contains null byte)"))?;
        // SAFETY: self.inner is valid per the caller's contract; c_alias outlives the call.
        unsafe { duckdb_logical_type_set_alias(self.inner, c_alias.as_ptr()) };
        Ok(())
    }

    // ------------------------------------------------------------------
    // Introspection methods
    // ------------------------------------------------------------------

    /// Returns the [`TypeId`] of this logical type.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn get_type_id(&self) -> TypeId {
        TypeId::from_duckdb_type(unsafe { duckdb_get_type_id(self.inner) })
    }

    /// Returns the alias of this logical type, or `None` if no alias is set.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn get_alias(&self) -> Option<String> {
        let ptr = unsafe { duckdb_logical_type_get_alias(self.inner) };
        if ptr.is_null() {
            return None;
        }
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { duckdb_free(ptr.cast::<core::ffi::c_void>()) };
        Some(s)
    }

    /// Sets an alias on this logical type.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    ///
    /// # Panics
    ///
    /// Panics if `alias` contains an interior null byte.
    pub unsafe fn set_alias(&self, alias: &str) {
        let c_alias = std::ffi::CString::new(alias).expect("alias must not contain null bytes");
        unsafe { duckdb_logical_type_set_alias(self.inner, c_alias.as_ptr()) };
    }

    /// Returns the width (total digits) of a `DECIMAL` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `DECIMAL` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn decimal_width(&self) -> u8 {
        unsafe { duckdb_decimal_width(self.inner) }
    }

    /// Returns the scale (digits after decimal point) of a `DECIMAL` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `DECIMAL` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn decimal_scale(&self) -> u8 {
        unsafe { duckdb_decimal_scale(self.inner) }
    }

    /// Returns the internal storage type of a `DECIMAL` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `DECIMAL` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn decimal_internal_type(&self) -> TypeId {
        TypeId::from_duckdb_type(unsafe { duckdb_decimal_internal_type(self.inner) })
    }

    /// Returns the internal storage type of an `ENUM` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ENUM` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn enum_internal_type(&self) -> TypeId {
        TypeId::from_duckdb_type(unsafe { duckdb_enum_internal_type(self.inner) })
    }

    /// Returns the number of members in an `ENUM` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ENUM` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn enum_dictionary_size(&self) -> u32 {
        unsafe { duckdb_enum_dictionary_size(self.inner) }
    }

    /// Returns the name of the enum member at `index`.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ENUM` logical type and `index` must be
    /// within bounds (requires `DuckDB` runtime).
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_enum_dictionary_value` returns a null pointer.
    #[must_use]
    pub unsafe fn enum_dictionary_value(&self, index: u64) -> String {
        let ptr =
            unsafe { duckdb_enum_dictionary_value(self.inner, index as libduckdb_sys::idx_t) };
        assert!(!ptr.is_null(), "duckdb_enum_dictionary_value returned null");
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { duckdb_free(ptr.cast::<core::ffi::c_void>()) };
        s
    }

    /// Returns the child (element) type of a `LIST` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `LIST` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn list_child_type(&self) -> Self {
        unsafe { Self::from_raw(duckdb_list_type_child_type(self.inner)) }
    }

    /// Returns the key type of a `MAP` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `MAP` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn map_key_type(&self) -> Self {
        unsafe { Self::from_raw(duckdb_map_type_key_type(self.inner)) }
    }

    /// Returns the value type of a `MAP` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `MAP` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn map_value_type(&self) -> Self {
        unsafe { Self::from_raw(duckdb_map_type_value_type(self.inner)) }
    }

    /// Returns the number of child fields in a `STRUCT` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `STRUCT` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn struct_child_count(&self) -> u64 {
        unsafe { duckdb_struct_type_child_count(self.inner) as u64 }
    }

    /// Returns the name of the struct field at `index`.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `STRUCT` logical type and `index` must be
    /// within bounds (requires `DuckDB` runtime).
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_struct_type_child_name` returns a null pointer.
    #[must_use]
    pub unsafe fn struct_child_name(&self, index: u64) -> String {
        unsafe {
            let ptr = duckdb_struct_type_child_name(self.inner, index as libduckdb_sys::idx_t);
            assert!(
                !ptr.is_null(),
                "duckdb_struct_type_child_name returned null"
            );
            let s = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
            duckdb_free(ptr.cast::<core::ffi::c_void>());
            s
        }
    }

    /// Returns the type of the struct field at `index`.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `STRUCT` logical type and `index` must be
    /// within bounds (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn struct_child_type(&self, index: u64) -> Self {
        unsafe {
            Self::from_raw(duckdb_struct_type_child_type(
                self.inner,
                index as libduckdb_sys::idx_t,
            ))
        }
    }

    /// Returns the number of members in a `UNION` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `UNION` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn union_member_count(&self) -> u64 {
        unsafe { duckdb_union_type_member_count(self.inner) as u64 }
    }

    /// Returns the name of the union member at `index`.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `UNION` logical type and `index` must be
    /// within bounds (requires `DuckDB` runtime).
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_union_type_member_name` returns a null pointer.
    #[must_use]
    pub unsafe fn union_member_name(&self, index: u64) -> String {
        unsafe {
            let ptr = duckdb_union_type_member_name(self.inner, index as libduckdb_sys::idx_t);
            assert!(
                !ptr.is_null(),
                "duckdb_union_type_member_name returned null"
            );
            let s = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
            duckdb_free(ptr.cast::<core::ffi::c_void>());
            s
        }
    }

    /// Returns the type of the union member at `index`.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `UNION` logical type and `index` must be
    /// within bounds (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn union_member_type(&self, index: u64) -> Self {
        unsafe {
            Self::from_raw(duckdb_union_type_member_type(
                self.inner,
                index as libduckdb_sys::idx_t,
            ))
        }
    }

    /// Returns the fixed size of an `ARRAY` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ARRAY` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn array_size(&self) -> u64 {
        unsafe { duckdb_array_type_array_size(self.inner) as u64 }
    }

    /// Returns the child (element) type of an `ARRAY` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ARRAY` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn array_child_type(&self) -> Self {
        unsafe { Self::from_raw(duckdb_array_type_child_type(self.inner)) }
    }

    /// Registers this type in the catalog of `con`, making it usable in SQL.
    ///
    /// This is how an extension ships a named type — `CREATE TYPE` from the C
    /// API. Once registered, the alias can be used anywhere a type name can:
    ///
    /// ```sql
    /// SELECT 'happy'::mood;
    /// CREATE TABLE t(m mood);
    /// ```
    ///
    /// `duckdb.h`: "Registers a custom type within the given connection. The
    /// type must have an alias." Set one with
    /// [`try_set_alias`][Self::try_set_alias] first; a type without an alias
    /// has no name to register under and `DuckDB` rejects it.
    ///
    /// The `duckdb_create_type_info` third argument is passed as null: `DuckDB`
    /// declares the handle but exposes no constructor for it in the C API, so
    /// null is the only value an extension can supply.
    ///
    /// # Errors
    ///
    /// Returns an error if `DuckDB` rejects the registration — no alias, or a
    /// name that already exists in the catalog.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::LogicalType;
    ///
    /// # fn demo(con: libduckdb_sys::duckdb_connection)
    /// # -> Result<(), quack_rs::error::ExtensionError> {
    /// let mood = LogicalType::enum_type(&["sad", "ok", "happy"]);
    /// // SAFETY: `con` is the connection DuckDB handed the entry point.
    /// unsafe {
    ///     mood.set_alias("mood");
    ///     mood.register(con)?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub unsafe fn register(
        &self,
        con: libduckdb_sys::duckdb_connection,
    ) -> Result<(), crate::error::ExtensionError> {
        // SAFETY: `con` is valid per the caller's contract, `self.inner` is a
        // live logical type, and the info handle has no C API constructor.
        let state = unsafe {
            libduckdb_sys::duckdb_register_logical_type(con, self.inner, std::ptr::null_mut())
        };
        if state == libduckdb_sys::DuckDBSuccess {
            return Ok(());
        }
        // SAFETY: `self.inner` is live; `get_alias` returns an owned String or None.
        let alias = unsafe { self.get_alias() };
        Err(crate::error::ExtensionError::new(alias.map_or_else(
            || {
                String::from(
                    "duckdb_register_logical_type failed: the type has no alias. Call \
                     `set_alias` or `try_set_alias` before registering — DuckDB has no name \
                     to register it under.",
                )
            },
            |name| {
                format!(
                    "duckdb_register_logical_type failed for alias '{name}': the name is \
                     probably already taken in this catalog"
                )
            },
        )))
    }

    /// Returns the underlying raw `duckdb_logical_type` handle.
    ///
    /// # Safety note
    ///
    /// Do not call `duckdb_destroy_logical_type` on the returned handle; that is
    /// handled by this type's `Drop` implementation.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_logical_type {
        self.inner
    }

    /// Consumes this `LogicalType` and returns the raw handle without destroying it.
    ///
    /// The caller is responsible for calling `duckdb_destroy_logical_type` on the
    /// returned handle.
    #[must_use]
    pub const fn into_raw(self) -> duckdb_logical_type {
        let raw = self.inner;
        // Prevent Drop from running by wrapping in ManuallyDrop
        std::mem::forget(self);
        raw
    }
}

impl Drop for LogicalType {
    #[mutants::skip]
    fn drop(&mut self) {
        // SAFETY: `self.inner` was created by `duckdb_create_logical_type` and has not
        // been transferred elsewhere. It is safe to destroy exactly once here.
        unsafe {
            duckdb_destroy_logical_type(&raw mut self.inner);
        }
    }
}

impl From<TypeId> for LogicalType {
    /// Creates a `LogicalType` from a `TypeId`.
    ///
    /// This is equivalent to calling [`LogicalType::new`].
    fn from(type_id: TypeId) -> Self {
        Self::new(type_id)
    }
}

// LogicalType is not Clone or Copy because the underlying handle is not reference-counted.
// If you need to pass it to multiple places, use `as_raw()` to borrow the handle temporarily.

impl core::fmt::Debug for LogicalType {
    /// Prints the decoded type rather than the handle address — the type id is
    /// the only thing anyone inspects a `LogicalType` to learn.
    ///
    /// This calls into `DuckDB`, so it deliberately avoids anything that could
    /// panic while formatting: a `Debug` impl that panics inside a panic message
    /// aborts the process. A type id from a newer `DuckDB` than this build knows
    /// renders as its numeric value rather than panicking the way
    /// [`get_type_id`][Self::get_type_id] would.
    ///
    /// The null check is belt-and-braces — every constructor, `from_raw`
    /// included, already asserts the handle is non-null — but a formatting
    /// impl is the last place worth being clever about a single comparison.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.inner.is_null() {
            return f.write_str("LogicalType(<null>)");
        }
        // SAFETY: `self.inner` is non-null and valid for this wrapper's lifetime.
        let raw_id = unsafe { duckdb_get_type_id(self.inner) };
        let mut out = f.debug_struct("LogicalType");
        match TypeId::try_from_duckdb_type(raw_id) {
            Some(type_id) => out.field("type_id", &type_id),
            // A type id this build of quack-rs does not know — print the number
            // rather than panicking, which is what `TypeId::from_duckdb_type`
            // would do.
            None => out.field("type_id", &format_args!("<unknown {raw_id}>")),
        };
        // SAFETY: `self.inner` is non-null and valid.
        if let Some(alias) = unsafe { self.get_alias() } {
            out.field("alias", &alias);
        }
        if TypeId::try_from_duckdb_type(raw_id) == Some(TypeId::Decimal) {
            // SAFETY: `self.inner` is a DECIMAL logical type.
            let (width, scale) = unsafe { (self.decimal_width(), self.decimal_scale()) };
            out.field("width", &width).field("scale", &scale);
        }
        out.finish()
    }
}

#[cfg(test)]
mod tests {
    // Note: LogicalType tests that call DuckDB API (duckdb_create_logical_type)
    // require a running DuckDB runtime and are covered in tests/integration_test.rs.
    // The `loadable-extension` feature uses lazy-initialized function pointers
    // that cannot be called without a prior call to duckdb_rs_extension_api_init.

    #[test]
    fn logical_type_error_display() {
        let err = super::LogicalTypeError::null("duckdb_create_logical_type");
        assert_eq!(err.to_string(), "duckdb_create_logical_type returned null");
    }

    #[test]
    fn size_of_logical_type_struct() {
        use super::LogicalType;
        // LogicalType must be pointer-sized (it contains a single pointer).
        assert_eq!(
            std::mem::size_of::<LogicalType>(),
            std::mem::size_of::<*mut ()>()
        );
    }
}

/// Constructor tests that need a live `DuckDB` C API dispatch table.
#[cfg(all(test, feature = "_duckdb-testing"))]
mod live_tests {
    use super::{LogicalType, TypeId};

    #[test]
    fn try_constructors_reject_interior_null_bytes() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        // Every fallible constructor that takes user-supplied names must report
        // an interior NUL instead of panicking: these run inside DuckDB bind
        // callbacks, where a panic cannot be surfaced to the user and aborts the
        // process under `panic = "abort"`.
        assert!(LogicalType::try_struct_type(&[("a\0b", TypeId::BigInt)]).is_err());
        assert!(LogicalType::try_union_type(&[("a\0b", TypeId::BigInt)]).is_err());
        assert!(LogicalType::try_enum_type(&["ok", "a\0b"]).is_err());
        let lt = LogicalType::try_new(TypeId::BigInt).expect("BIGINT");
        // SAFETY: `lt` wraps a valid logical type.
        assert!(unsafe { lt.try_set_alias("a\0b") }.is_err());
    }

    #[test]
    fn try_constructor_errors_name_the_failing_step() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        let Err(err) = LogicalType::try_enum_type(&["a\0b"]) else {
            panic!("try_enum_type must reject an interior NUL");
        };
        assert!(err.to_string().contains("null byte"), "{err}");
    }

    #[test]
    fn try_constructors_build_valid_types() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        let s = LogicalType::try_struct_type(&[("x", TypeId::BigInt), ("y", TypeId::Varchar)])
            .expect("struct");
        // SAFETY: `s` wraps a valid logical type.
        unsafe {
            assert_eq!(s.get_type_id(), TypeId::Struct);
            assert_eq!(s.struct_child_count(), 2);
            assert_eq!(s.struct_child_name(0), "x");
        }

        let u = LogicalType::try_union_type(&[("a", TypeId::BigInt), ("b", TypeId::Double)])
            .expect("union");
        // SAFETY: `u` wraps a valid logical type.
        unsafe {
            assert_eq!(u.get_type_id(), TypeId::Union);
            assert_eq!(u.union_member_count(), 2);
        }

        let e = LogicalType::try_enum_type(&["red", "green", "blue"]).expect("enum");
        // SAFETY: `e` wraps a valid logical type.
        unsafe {
            assert_eq!(e.get_type_id(), TypeId::Enum);
            assert_eq!(e.enum_dictionary_size(), 3);
            assert_eq!(e.enum_dictionary_value(1), "green");
        }

        let nested = LogicalType::try_struct_type_from_logical(&[(
            "inner",
            LogicalType::try_list(TypeId::Integer).expect("list"),
        )])
        .expect("nested struct");
        // SAFETY: `nested` wraps a valid logical type.
        unsafe {
            assert_eq!(nested.get_type_id(), TypeId::Struct);
            assert_eq!(nested.struct_child_type(0).get_type_id(), TypeId::List);
        }
    }
}
