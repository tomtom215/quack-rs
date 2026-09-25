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

mod construct;

pub use construct::MAX_UNION_MEMBERS;

/// Checks one builder slot's [`TypeId`]: [`LogicalType::check_slot`] when
/// registering, `check_slot_offline` in
/// [`MockRegistrar`][crate::testing::MockRegistrar]. Each builder's
/// `check_types` takes one, so both run the same sequence of checks.
pub(crate) type SlotCheck = fn(TypeId, &str) -> Result<(), crate::error::ExtensionError>;

use crate::types::TypeId;
use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_array_type_child_type, duckdb_decimal_internal_type,
    duckdb_decimal_scale, duckdb_decimal_width, duckdb_destroy_logical_type,
    duckdb_enum_dictionary_size, duckdb_enum_dictionary_value, duckdb_enum_internal_type,
    duckdb_free, duckdb_get_type_id, duckdb_list_type_child_type, duckdb_logical_type,
    duckdb_logical_type_get_alias, duckdb_logical_type_set_alias, duckdb_map_type_key_type,
    duckdb_map_type_value_type, duckdb_struct_type_child_count, duckdb_struct_type_child_name,
    duckdb_struct_type_child_type, duckdb_union_type_member_count, duckdb_union_type_member_name,
    duckdb_union_type_member_type,
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

    /// Builds an error whose message is `detail`.
    const fn with_detail(api_func: &'static str, detail: String) -> Self {
        Self {
            api_func,
            detail: Some(detail),
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
         variant of this method (`param_logical` / `returns_logical` / \
         `varargs_logical` on a function builder, \
         `CastFunctionBuilder::new_logical` for a cast).",
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
    /// # Panics
    ///
    /// Panics if `DuckDB` reports a type this build of quack-rs has no variant
    /// for — for example `GEOMETRY` or `VARIANT` without the `duckdb-1-5-3`
    /// feature, or a type added by a newer `DuckDB`. Inside a callback, where
    /// the type comes from the query, prefer
    /// [`try_get_type_id`][Self::try_get_type_id].
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn get_type_id(&self) -> TypeId {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_get_type_id` only reads the type's id.
        TypeId::from_duckdb_type(unsafe { duckdb_get_type_id(self.inner) })
    }

    /// Returns the [`TypeId`] of this logical type, or `None` if this build of
    /// quack-rs has no variant for it (see [`get_type_id`][Self::get_type_id]).
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn try_get_type_id(&self) -> Option<TypeId> {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_get_type_id` only reads the type's id.
        TypeId::try_from_duckdb_type(unsafe { duckdb_get_type_id(self.inner) })
    }

    /// Returns the alias of this logical type, or `None` if no alias is set.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn get_alias(&self) -> Option<String> {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_logical_type_get_alias` dereferences it unconditionally and returns a
        // `strdup` copy of the alias or null (logical_types-c.cpp).
        let ptr = unsafe { duckdb_logical_type_get_alias(self.inner) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `ptr` was checked non-null just above and is a `strdup` result, so it is
        // NUL-terminated; it stays allocated until the `duckdb_free` below.
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `ptr` is the non-null `strdup` allocation DuckDB handed us (duckdb.h:
        // free with `duckdb_free`, which is `free`); `s` already holds an owned copy, and
        // this is the only free.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `c_alias` is a NUL-terminated `CString` that outlives the call, and
        // `LogicalType::SetAlias(string)` copies it, so no pointer is retained.
        unsafe { duckdb_logical_type_set_alias(self.inner, c_alias.as_ptr()) };
    }

    /// Returns the width (total digits) of a `DECIMAL` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `DECIMAL` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn decimal_width(&self) -> u8 {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_decimal_width` checks the id itself and returns 0 for a non-DECIMAL type
        // (logical_types-c.cpp), so a type of another kind is not UB here.
        unsafe { duckdb_decimal_width(self.inner) }
    }

    /// Returns the scale (digits after decimal point) of a `DECIMAL` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `DECIMAL` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn decimal_scale(&self) -> u8 {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_decimal_scale` checks the id itself and returns 0 for a non-DECIMAL type
        // (logical_types-c.cpp), so a type of another kind is not UB here.
        unsafe { duckdb_decimal_scale(self.inner) }
    }

    /// Returns the internal storage type of a `DECIMAL` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `DECIMAL` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn decimal_internal_type(&self) -> TypeId {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_decimal_internal_type` checks the id itself and returns INVALID for a
        // non-DECIMAL type (logical_types-c.cpp), so a type of another kind is not UB here.
        TypeId::from_duckdb_type(unsafe { duckdb_decimal_internal_type(self.inner) })
    }

    /// Returns the internal storage type of an `ENUM` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ENUM` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn enum_internal_type(&self) -> TypeId {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_enum_internal_type` checks the id itself and returns INVALID for a
        // non-ENUM type (logical_types-c.cpp), so a type of another kind is not UB here.
        TypeId::from_duckdb_type(unsafe { duckdb_enum_internal_type(self.inner) })
    }

    /// Returns the number of members in an `ENUM` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ENUM` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn enum_dictionary_size(&self) -> u32 {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_enum_dictionary_size` checks the id itself and returns 0 for a non-ENUM
        // type (logical_types-c.cpp), so a type of another kind is not UB here.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_enum_dictionary_value` returns null for a non-ENUM type (asserted below)
        // and does not bounds-check `index`; this function's `# Safety` clause requires
        // `index` to be within the dictionary.
        let ptr =
            unsafe { duckdb_enum_dictionary_value(self.inner, index as libduckdb_sys::idx_t) };
        assert!(!ptr.is_null(), "duckdb_enum_dictionary_value returned null");
        // SAFETY: `ptr` is non-null (asserted on the line above) and is a `strdup` result,
        // so it is NUL-terminated; it stays allocated until the free below.
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `ptr` is the non-null `strdup` allocation DuckDB handed us (duckdb.h:
        // free with `duckdb_free`, which is `free`); `s` already holds an owned copy, and
        // this is the only free.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_list_type_child_type` returns null for a non-LIST/MAP type, which
        // `from_raw`'s assert turns into a panic; otherwise it returns a fresh heap `new
        // LogicalType(..)` the caller owns (duckdb.h: "must be freed with
        // `duckdb_destroy_logical_type`"), satisfying `from_raw`'s ownership clause.
        unsafe { Self::from_raw(duckdb_list_type_child_type(self.inner)) }
    }

    /// Returns the key type of a `MAP` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `MAP` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn map_key_type(&self) -> Self {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_map_type_key_type` returns null for a non-MAP type, which `from_raw`'s
        // assert turns into a panic; otherwise it returns a fresh heap `new
        // LogicalType(..)` the caller owns (duckdb.h: "must be freed with
        // `duckdb_destroy_logical_type`"), satisfying `from_raw`'s ownership clause.
        unsafe { Self::from_raw(duckdb_map_type_key_type(self.inner)) }
    }

    /// Returns the value type of a `MAP` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `MAP` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn map_value_type(&self) -> Self {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_map_type_value_type` returns null for a non-MAP type, which `from_raw`'s
        // assert turns into a panic; otherwise it returns a fresh heap `new
        // LogicalType(..)` the caller owns (duckdb.h: "must be freed with
        // `duckdb_destroy_logical_type`"), satisfying `from_raw`'s ownership clause.
        unsafe { Self::from_raw(duckdb_map_type_value_type(self.inner)) }
    }

    /// Returns the number of child fields in a `STRUCT` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be a `STRUCT` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn struct_child_count(&self) -> u64 {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_struct_type_child_count` checks the physical type itself and returns 0
        // for a non-STRUCT type (logical_types-c.cpp), so a type of another kind is not UB
        // here.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it). `index`
        // is in bounds per this function's `# Safety` clause:
        // `duckdb_struct_type_child_name` does not check it (the `*Type::Get*Name` helpers
        // in types.cpp only `D_ASSERT` it). A wrong-kind type yields null. The result is
        // checked non-null by the assert before `CStr::from_ptr`; it is a `strdup` copy, so
        // NUL-terminated and ours to release with `duckdb_free` (which is `free`,
        // helper-c.cpp), exactly once, after `to_string_lossy().into_owned()` has copied
        // it.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it). `index`
        // is in bounds per this function's `# Safety` clause:
        // `duckdb_struct_type_child_type` only `D_ASSERT`s it (types.cpp). A non-STRUCT
        // type yields null, which `from_raw`'s assert turns into a panic; otherwise the
        // result is a fresh `new LogicalType(..)` the caller owns, as `from_raw` requires.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_union_type_member_count` checks the id itself and returns 0 for a
        // non-UNION type (logical_types-c.cpp), so a type of another kind is not UB here.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it). `index`
        // is in bounds per this function's `# Safety` clause:
        // `duckdb_union_type_member_name` does not check it (the `*Type::Get*Name` helpers
        // in types.cpp only `D_ASSERT` it). A wrong-kind type yields null. The result is
        // checked non-null by the assert before `CStr::from_ptr`; it is a `strdup` copy, so
        // NUL-terminated and ours to release with `duckdb_free` (which is `free`,
        // helper-c.cpp), exactly once, after `to_string_lossy().into_owned()` has copied
        // it.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it). `index`
        // is in bounds per this function's `# Safety` clause:
        // `duckdb_union_type_member_type` only `D_ASSERT`s it (types.cpp). A non-UNION type
        // yields null, which `from_raw`'s assert turns into a panic; otherwise the result
        // is a fresh `new LogicalType(..)` the caller owns, as `from_raw` requires.
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
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_array_type_array_size` checks the id itself and returns 0 for a non-ARRAY
        // type (logical_types-c.cpp), so a type of another kind is not UB here.
        unsafe { duckdb_array_type_array_size(self.inner) as u64 }
    }

    /// Returns the child (element) type of an `ARRAY` type.
    ///
    /// # Safety
    ///
    /// The inner handle must be an `ARRAY` logical type (requires `DuckDB` runtime).
    #[must_use]
    pub unsafe fn array_child_type(&self) -> Self {
        // SAFETY: `self.inner` is a non-null handle this value owns and keeps live until
        // `Drop` (type invariant: `owned_or` rejects null, `from_raw` asserts it).
        // `duckdb_array_type_child_type` returns null for a non-ARRAY type, which
        // `from_raw`'s assert turns into a panic; otherwise it returns a fresh heap `new
        // LogicalType(..)` the caller owns (duckdb.h: "must be freed with
        // `duckdb_destroy_logical_type`"), satisfying `from_raw`'s ownership clause.
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
    /// Returns an error if the type is or contains `ANY` or `INVALID` (checked
    /// here, since `DuckDB` reports it the same way as a taken name), or if
    /// `DuckDB` rejects the registration — no alias, or a name that already
    /// exists in the catalog.
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
        // `duckdb_register_logical_type` refuses such a type before it looks
        // at the catalog, with the same bare error as a taken name.
        // SAFETY: `self.inner` is a live logical type.
        if unsafe { crate::table::type_check::contains_any_or_invalid(self.inner) } {
            return Err(crate::error::ExtensionError::new(
                "duckdb_register_logical_type: the type is or contains ANY or INVALID, which \
                 DuckDB refuses to register",
            ));
        }
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
    use super::{composite_message, LogicalType, LogicalTypeError, TypeId};

    // `check_slot` is what every builder calls *before* allocating a DuckDB
    // handle, and `composite_message` is the advice it carries. Both are pure,
    // and neither is reachable from the end-to-end suite without a database, so
    // they are pinned here.

    #[test]
    fn check_slot_rejects_a_composite_before_touching_duckdb() {
        // Only the rejection path is unit-testable: a primitive falls through
        // to `duckdb_create_logical_type`, which needs a live dispatch table.
        // Rejecting *before* that call is the whole point of `check_slot`, so
        // this test reaching its assertions at all is part of what it proves.
        let err = LogicalType::check_slot(TypeId::Struct, "parameter 0")
            .expect_err("STRUCT cannot come from a bare TypeId");
        let msg = err.as_str();
        assert!(msg.contains("parameter 0"), "names the slot: {msg}");
        assert!(msg.contains("STRUCT"), "names the type: {msg}");
        assert!(
            msg.contains("LogicalType::struct_type(&fields)"),
            "names the constructor to use instead: {msg}"
        );
    }

    /// `DuckDB` registers a function whose parameter or return type is a
    /// literal type, then invalidates the database on the first query that
    /// returns one. Both are refused before any `DuckDB` call.
    #[test]
    fn check_slot_rejects_the_literal_types_before_touching_duckdb() {
        for (ty, name) in [
            (TypeId::IntegerLiteral, "INTEGER_LITERAL"),
            (TypeId::StringLiteral, "STRING_LITERAL"),
        ] {
            let err = LogicalType::check_slot(ty, "return type").expect_err("literal type");
            let msg = err.as_str();
            assert!(msg.contains("return type"), "names the slot: {msg}");
            assert!(msg.contains(name), "names the type: {msg}");
            assert!(msg.contains("invalidates the database"), "{msg}");
        }
    }

    #[test]
    fn the_composite_message_names_the_type_and_its_constructor() {
        let msg = composite_message(TypeId::Decimal);
        assert_ne!(msg, "");
        assert!(msg.contains("DECIMAL"), "{msg}");
        assert!(msg.contains("LogicalType::decimal(width, scale)"), "{msg}");
        assert!(
            msg.contains("param_logical"),
            "points at the escape hatch: {msg}"
        );

        // A non-composite has no hint, so the message falls back rather than
        // naming a constructor that does not exist.
        let generic = composite_message(TypeId::BigInt);
        assert!(
            generic.contains("a dedicated LogicalType constructor"),
            "{generic}"
        );
    }

    #[test]
    fn a_null_error_reports_the_api_function_that_returned_it() {
        let err = LogicalTypeError::null("duckdb_create_list_type");
        assert_eq!(err.api_func(), "duckdb_create_list_type");
        assert_eq!(err.to_string(), "duckdb_create_list_type returned null");
    }

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
    use super::{LogicalType, TypeId, MAX_UNION_MEMBERS};

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

    /// Regression: these six type ids exist in every `DuckDB` this crate
    /// supports (all are in libduckdb-sys 1.4.4), but their `TypeId` variants
    /// were gated behind `duckdb-1-5`. Without that feature, `get_type_id` on a
    /// `TIME_NS` or `BIGNUM` column panicked. (`DuckDB` 1.4.4's C API reports a
    /// `TIME_NS` column as `INVALID`, so on 1.4.x only `BIGNUM` reaches the
    /// mapping; the fourth audit ran this against 1.4.4 and found the earlier
    /// wording wrong.)
    #[test]
    fn type_ids_from_duckdb_1_4_resolve_without_any_feature() {
        use libduckdb_sys::{
            duckdb_create_logical_type, DUCKDB_TYPE_DUCKDB_TYPE_ANY,
            DUCKDB_TYPE_DUCKDB_TYPE_BIGNUM, DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL,
            DUCKDB_TYPE_DUCKDB_TYPE_TIME_NS,
        };
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        for (raw, want) in [
            (DUCKDB_TYPE_DUCKDB_TYPE_TIME_NS, TypeId::TimeNs),
            (DUCKDB_TYPE_DUCKDB_TYPE_BIGNUM, TypeId::Varint),
            (DUCKDB_TYPE_DUCKDB_TYPE_ANY, TypeId::Any),
            (DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL, TypeId::SqlNull),
        ] {
            // SAFETY: a valid DUCKDB_TYPE for a non-composite type; the handle
            // is owned by the returned `LogicalType`.
            let ty = unsafe { LogicalType::from_raw(duckdb_create_logical_type(raw)) };
            // DuckDB 1.4.x's C API does not know TIME_NS: it hands back an
            // INVALID type (and reports a TIME_NS column as INVALID), so there
            // is no id to resolve. Any other mismatch is a failure.
            // SAFETY: `ty` wraps a valid logical type.
            let got = unsafe { libduckdb_sys::duckdb_get_type_id(ty.as_raw()) };
            if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIME_NS
                && got == libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_INVALID
            {
                continue;
            }
            // SAFETY: `ty` wraps a valid logical type.
            unsafe {
                assert_eq!(ty.try_get_type_id(), Some(want));
                assert_eq!(ty.get_type_id(), want);
            }
        }
    }

    /// Regression: `duckdb_create_struct_type` / `duckdb_create_union_type`
    /// validate nothing, so these returned `Ok`, registered, and then failed
    /// on first use with `DuckDB`'s binder errors ("Duplicate STRUCT type
    /// argument name", "UNION type supports at most 256 type modifiers").
    #[test]
    fn struct_and_union_constructors_apply_duckdbs_name_rules() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        assert!(
            LogicalType::try_struct_type(&[("a", TypeId::Integer), ("a", TypeId::Integer)])
                .is_err()
        );
        assert!(
            LogicalType::try_struct_type(&[("a", TypeId::Integer), ("A", TypeId::Integer)])
                .is_err()
        );
        assert!(
            LogicalType::try_union_type(&[("m", TypeId::Integer), ("M", TypeId::Varchar)]).is_err()
        );
        // Unnamed structs have empty field names, repeated.
        assert!(
            LogicalType::try_struct_type(&[("", TypeId::Integer), ("", TypeId::Integer)]).is_ok()
        );

        let owned: Vec<String> = (0..=MAX_UNION_MEMBERS).map(|i| format!("m{i}")).collect();
        let members: Vec<(&str, TypeId)> = owned
            .iter()
            .map(|n| (n.as_str(), TypeId::Integer))
            .collect();
        assert!(LogicalType::try_union_type(&members[..MAX_UNION_MEMBERS]).is_ok());
        let err = LogicalType::try_union_type(&members).expect_err("257 members");
        assert!(err.to_string().contains("at most 256"), "{err}");
    }

    /// The fallible forms of the four constructors that had none (AUDIT.md
    /// section 5.5) report `DuckDB`'s refusals instead of panicking. The
    /// DECIMAL cases failed against `DuckDB` 1.5.0, whose
    /// `duckdb_create_decimal_type` accepts any width and scale; quack-rs
    /// checks them itself now (fourth audit).
    #[test]
    fn decimal_and_array_have_fallible_forms() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        assert!(LogicalType::try_decimal(18, 3).is_ok());
        for (w, s) in [(0, 0), (39, 0), (5, 6)] {
            let err = LogicalType::try_decimal(w, s).expect_err("out of range");
            assert_eq!(err.api_func(), "duckdb_create_decimal_type");
        }
        assert!(LogicalType::try_array(TypeId::Float, 99_999).is_ok());
        assert!(LogicalType::try_array(TypeId::Float, 100_000).is_err());
        assert!(LogicalType::try_array(TypeId::Struct, 3).is_err());
        let element = LogicalType::new(TypeId::Integer);
        assert!(LogicalType::try_list_from_logical(&element).is_ok());
        assert!(LogicalType::try_map_from_logical(&element, &element).is_ok());
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
