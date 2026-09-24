// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Constructors for [`LogicalType`].
//!
//! Every constructor comes in two forms: `try_*`, which returns a
//! [`LogicalTypeError`], and a panicking form that calls it. Only the `try_*`
//! form talks to `DuckDB`, so the two cannot disagree about what they accept.

use std::ffi::CString;
use std::os::raw::c_char;

use libduckdb_sys::{
    duckdb_create_array_type, duckdb_create_decimal_type, duckdb_create_enum_type,
    duckdb_create_list_type, duckdb_create_logical_type, duckdb_create_map_type,
    duckdb_create_struct_type, duckdb_create_union_type, duckdb_logical_type, idx_t,
};

use super::{composite_message, LogicalType, LogicalTypeError};
use crate::types::TypeId;

/// The most members a `UNION` may have.
///
/// This is `UnionType::MAX_UNION_MEMBERS` in `DuckDB` (the member tag is a
/// `UINT8`). `duckdb_create_union_type` does not check it; a type with more
/// members is only refused later, by the binder.
pub const MAX_UNION_MEMBERS: usize = 256;

/// Converts `STRUCT` field or `UNION` member names to C strings, enforcing
/// the rules `DuckDB`'s binder applies to the same type written in SQL.
///
/// `duckdb_create_struct_type` / `duckdb_create_union_type` check nothing
/// (`src/main/capi/logical_types-c.cpp`), so without this a type the binder
/// would reject — "Duplicate STRUCT type argument name", "UNION type supports
/// at most 256 type modifiers" — is built successfully, registers, and fails
/// on first use. Names are compared ASCII-case-insensitively, as `DuckDB`
/// does (`a` and `A` collide). Empty names are allowed and exempt from the
/// duplicate check: an unnamed `STRUCT` has an empty name for every field.
fn checked_names<'a>(
    kind: &str,
    api_func: &'static str,
    names: impl ExactSizeIterator<Item = &'a str>,
    max: Option<usize>,
) -> Result<Vec<CString>, LogicalTypeError> {
    if let Some(max) = max {
        if names.len() > max {
            return Err(LogicalTypeError::with_detail(
                api_func,
                format!(
                    "a {kind} has {} names, but DuckDB allows at most {max}",
                    names.len()
                ),
            ));
        }
    }
    let mut seen = std::collections::HashSet::new();
    names
        .map(|name| {
            if !name.is_empty() && !seen.insert(name.to_ascii_lowercase()) {
                return Err(LogicalTypeError::with_detail(
                    api_func,
                    format!(
                        "duplicate {kind} name {name:?}: DuckDB compares these names \
                         case-insensitively and rejects repeats"
                    ),
                ));
            }
            CString::new(name).map_err(|_| {
                LogicalTypeError::with_detail(
                    api_func,
                    format!("{kind} name {name:?} contains a null byte"),
                )
            })
        })
        .collect()
}

/// Calls a `DuckDB` constructor taking parallel `types` / `names` arrays.
///
/// # Safety
///
/// `create` must be `duckdb_create_struct_type` or `duckdb_create_union_type`.
unsafe fn create_named(
    create: unsafe fn(*mut duckdb_logical_type, *mut *const c_char, idx_t) -> duckdb_logical_type,
    api_func: &'static str,
    members: &[(&str, LogicalType)],
    names: &[CString],
) -> Result<LogicalType, LogicalTypeError> {
    let mut type_ptrs: Vec<duckdb_logical_type> = members.iter().map(|(_, t)| t.as_raw()).collect();
    let mut name_ptrs: Vec<*const c_char> = names.iter().map(|s| s.as_ptr()).collect();
    // SAFETY: both arrays hold `members.len()` entries that stay alive for the
    // call; DuckDB copies the types and names and returns an owned handle or
    // null.
    let inner = unsafe {
        create(
            type_ptrs.as_mut_ptr(),
            name_ptrs.as_mut_ptr(),
            members.len() as idx_t,
        )
    };
    LogicalType::owned_or(inner, api_func)
}

/// Resolves `(name, TypeId)` pairs into `(name, LogicalType)` pairs.
fn resolve<'a>(
    members: &[(&'a str, TypeId)],
) -> Result<Vec<(&'a str, LogicalType)>, LogicalTypeError> {
    members
        .iter()
        .map(|&(n, t)| LogicalType::try_new(t).map(|lt| (n, lt)))
        .collect()
}

impl LogicalType {
    /// Wraps a handle a `duckdb_create_*` function just returned, or reports
    /// the null it returned instead.
    const fn owned_or(
        inner: duckdb_logical_type,
        api_func: &'static str,
    ) -> Result<Self, LogicalTypeError> {
        if inner.is_null() {
            Err(LogicalTypeError::null(api_func))
        } else {
            Ok(Self { inner })
        }
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
    /// - Panics if `type_id` is [`TypeId::IntegerLiteral`] or
    ///   [`TypeId::StringLiteral`]: `DuckDB` accepts either in a function or
    ///   type registration, but the first query returning one fails with an
    ///   internal error that invalidates the database (checked on 1.4.4,
    ///   1.5.0 and 1.5.5).
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
        Self::try_new(type_id).unwrap_or_else(|e| {
            panic!("LogicalType::new({type_id:?}) would build an invalid type: {e}")
        })
    }

    /// Fallible version of [`LogicalType::new`].
    ///
    /// Returns an error instead of panicking when `type_id` is composite (see
    /// [`LogicalType::new`]), is [`TypeId::IntegerLiteral`] or
    /// [`TypeId::StringLiteral`], when the `DuckDB` C API returns a null
    /// pointer, or when the running `DuckDB` does not support the type
    /// (`TIME_NS` before 1.5.0, whose C API returns an `INVALID` type for it).
    pub fn try_new(type_id: TypeId) -> Result<Self, LogicalTypeError> {
        if type_id.is_composite() {
            return Err(LogicalTypeError::with_detail(
                "duckdb_create_logical_type",
                composite_message(type_id),
            ));
        }
        if matches!(type_id, TypeId::IntegerLiteral | TypeId::StringLiteral) {
            return Err(LogicalTypeError::with_detail(
                "duckdb_create_logical_type",
                format!(
                    "{type_id} is the binder's type for an unbound literal, not a column type: \
                     DuckDB registers a function or type that uses it, but the first query \
                     that returns it fails with an internal error that invalidates the \
                     database, and a parameter of it can never be called. Use BIGINT or \
                     VARCHAR"
                ),
            ));
        }
        // SAFETY: any DUCKDB_TYPE value is accepted; DuckDB returns an owned
        // handle or null.
        let inner = unsafe { duckdb_create_logical_type(type_id.to_duckdb_type()) };
        let created = Self::owned_or(inner, "duckdb_create_logical_type")?;
        // An engine whose C API does not know the id hands back an INVALID
        // type rather than null: DuckDB 1.4.x does this for TIME_NS, which
        // its SQL has but its C API reports as INVALID.
        // SAFETY: `created` owns a live handle.
        let got = unsafe { libduckdb_sys::duckdb_get_type_id(created.as_raw()) };
        if got != type_id.to_duckdb_type() {
            return Err(LogicalTypeError::with_detail(
                "duckdb_create_logical_type",
                format!(
                    "this DuckDB's C API does not support {type_id}: it created type id {got} \
                     instead (TIME_NS, for one, needs DuckDB 1.5.0 or later)"
                ),
            ));
        }
        Ok(created)
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
    /// Panics if `element_type` is composite (use
    /// [`list_from_logical`][Self::list_from_logical]) or if
    /// `duckdb_create_list_type` returns null. See [`try_list`][Self::try_list].
    #[must_use]
    pub fn list(element_type: TypeId) -> Self {
        Self::try_list(element_type).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::list`].
    pub fn try_list(element_type: TypeId) -> Result<Self, LogicalTypeError> {
        Self::try_list_from_logical(&Self::try_new(element_type)?)
    }

    /// Creates a `LIST<element>` logical type from an existing [`LogicalType`].
    ///
    /// Use this when the element type is itself a complex type (e.g.
    /// `LIST(STRUCT(...))`) that cannot be expressed as a simple [`TypeId`].
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_list_type` returns null. See
    /// [`try_list_from_logical`][Self::try_list_from_logical].
    #[must_use]
    pub fn list_from_logical(element: &Self) -> Self {
        Self::try_list_from_logical(element).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::list_from_logical`].
    pub fn try_list_from_logical(element: &Self) -> Result<Self, LogicalTypeError> {
        // SAFETY: `element` is a live handle for the call; DuckDB copies it and
        // returns an owned handle or null.
        let inner = unsafe { duckdb_create_list_type(element.as_raw()) };
        Self::owned_or(inner, "duckdb_create_list_type")
    }

    /// Creates a `MAP<key_type, value_type>` logical type.
    ///
    /// `DuckDB` maps are stored as `LIST<STRUCT{key: K, value: V}>`.
    ///
    /// # Panics
    ///
    /// Panics if either type is composite (use
    /// [`map_from_logical`][Self::map_from_logical]) or if
    /// `duckdb_create_map_type` returns null. See [`try_map`][Self::try_map].
    #[must_use]
    pub fn map(key_type: TypeId, value_type: TypeId) -> Self {
        Self::try_map(key_type, value_type).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::map`].
    pub fn try_map(key_type: TypeId, value_type: TypeId) -> Result<Self, LogicalTypeError> {
        Self::try_map_from_logical(&Self::try_new(key_type)?, &Self::try_new(value_type)?)
    }

    /// Creates a `MAP<key, value>` logical type from existing [`LogicalType`]s.
    ///
    /// Use this when the key or value types are complex types that cannot be
    /// expressed as simple [`TypeId`] values.
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_map_type` returns null. See
    /// [`try_map_from_logical`][Self::try_map_from_logical].
    #[must_use]
    pub fn map_from_logical(key: &Self, value: &Self) -> Self {
        Self::try_map_from_logical(key, value).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::map_from_logical`].
    pub fn try_map_from_logical(key: &Self, value: &Self) -> Result<Self, LogicalTypeError> {
        // SAFETY: both handles are live for the call; DuckDB copies them and
        // returns an owned handle or null.
        let inner = unsafe { duckdb_create_map_type(key.as_raw(), value.as_raw()) };
        Self::owned_or(inner, "duckdb_create_map_type")
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
    /// Panics where [`try_struct_type`][Self::try_struct_type] would return
    /// an error.
    #[must_use]
    pub fn struct_type(fields: &[(&str, TypeId)]) -> Self {
        Self::try_struct_type(fields).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::struct_type`].
    ///
    /// # Errors
    ///
    /// See [`try_struct_type_from_logical`][Self::try_struct_type_from_logical];
    /// also fails if a field's `TypeId` is composite.
    pub fn try_struct_type(fields: &[(&str, TypeId)]) -> Result<Self, LogicalTypeError> {
        Self::try_struct_type_from_logical(&resolve(fields)?)
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
    /// Panics where
    /// [`try_struct_type_from_logical`][Self::try_struct_type_from_logical]
    /// would return an error.
    #[must_use]
    pub fn struct_type_from_logical(fields: &[(&str, Self)]) -> Self {
        Self::try_struct_type_from_logical(fields).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::struct_type_from_logical`].
    ///
    /// # Errors
    ///
    /// Returns an error if two field names are equal ignoring ASCII case
    /// (`DuckDB` rejects that type as soon as it is used), if a name contains
    /// a null byte, or if `duckdb_create_struct_type` returns null. Empty names
    /// are allowed, as in an unnamed `STRUCT`.
    pub fn try_struct_type_from_logical(fields: &[(&str, Self)]) -> Result<Self, LogicalTypeError> {
        const API: &str = "duckdb_create_struct_type";
        let names = checked_names("STRUCT field", API, fields.iter().map(|&(n, _)| n), None)?;
        // SAFETY: `duckdb_create_struct_type` is one of the two functions
        // `create_named` accepts.
        unsafe { create_named(duckdb_create_struct_type, API, fields, &names) }
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
    /// Panics where [`try_decimal`][Self::try_decimal] would return an error.
    #[must_use]
    pub fn decimal(width: u8, scale: u8) -> Self {
        Self::try_decimal(width, scale).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::decimal`].
    ///
    /// # Errors
    ///
    /// Returns an error unless `1 <= width <= 38` and `scale <= width`
    /// (`Decimal::IsValidWidthScale`). The check is made here: `DuckDB` makes
    /// it only from v1.5.4, and before that `duckdb_create_decimal_type`
    /// returns a `DECIMAL(0, 0)` or `DECIMAL(5, 6)` it cannot use.
    pub fn try_decimal(width: u8, scale: u8) -> Result<Self, LogicalTypeError> {
        if !(1..=38).contains(&width) || scale > width {
            return Err(LogicalTypeError::with_detail(
                "duckdb_create_decimal_type",
                format!(
                    "DECIMAL({width}, {scale}) is not a DECIMAL type: the width must be 1 to 38 \
                     and the scale no larger than the width"
                ),
            ));
        }
        // SAFETY: plain values; DuckDB returns an owned handle or null.
        let inner = unsafe { duckdb_create_decimal_type(width, scale) };
        Self::owned_or(inner, "duckdb_create_decimal_type")
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
    /// Panics where [`try_array`][Self::try_array] would return an error.
    #[must_use]
    pub fn array(element_type: TypeId, size: u64) -> Self {
        Self::try_array(element_type, size).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::array`].
    ///
    /// # Errors
    ///
    /// See [`try_array_from_logical`][Self::try_array_from_logical]; also fails
    /// if `element_type` is composite.
    pub fn try_array(element_type: TypeId, size: u64) -> Result<Self, LogicalTypeError> {
        Self::try_array_from_logical(&Self::try_new(element_type)?, size)
    }

    /// Creates an `ARRAY<element>[size]` logical type from an existing [`LogicalType`].
    ///
    /// Use this when the element type is itself a complex type.
    ///
    /// # Panics
    ///
    /// Panics where [`try_array_from_logical`][Self::try_array_from_logical]
    /// would return an error.
    #[must_use]
    pub fn array_from_logical(element: &Self, size: u64) -> Self {
        Self::try_array_from_logical(element, size).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::array_from_logical`].
    ///
    /// # Errors
    ///
    /// Returns an error if `size` is 0, or if `DuckDB` refuses the size:
    /// `duckdb_create_array_type` returns null for `size >= 100_000`
    /// (`ArrayType::MAX_ARRAY_SIZE`). SQL rejects a size of 0 ("ARRAY type size
    /// must be at least 1"), and so does a `DuckDB` built with assertions (a
    /// null handle); a release build creates the type anyway, so it is refused
    /// here to behave the same on both.
    pub fn try_array_from_logical(element: &Self, size: u64) -> Result<Self, LogicalTypeError> {
        if size == 0 {
            return Err(LogicalTypeError::with_detail(
                "duckdb_create_array_type",
                "an ARRAY's size must be at least 1, as in SQL".to_owned(),
            ));
        }
        // SAFETY: `element` is live for the call; DuckDB copies it and returns
        // an owned handle or null.
        let inner = unsafe { duckdb_create_array_type(element.as_raw(), size as idx_t) };
        Self::owned_or(inner, "duckdb_create_array_type")
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
    /// Panics where [`try_union_type`][Self::try_union_type] would return an
    /// error.
    #[must_use]
    pub fn union_type(members: &[(&str, TypeId)]) -> Self {
        Self::try_union_type(members).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::union_type`].
    ///
    /// # Errors
    ///
    /// See [`try_union_type_from_logical`][Self::try_union_type_from_logical];
    /// also fails if a member's `TypeId` is composite.
    pub fn try_union_type(members: &[(&str, TypeId)]) -> Result<Self, LogicalTypeError> {
        Self::try_union_type_from_logical(&resolve(members)?)
    }

    /// Creates a `UNION` logical type from a slice of `(name, LogicalType)` members.
    ///
    /// Use this when members have complex types.
    ///
    /// # Panics
    ///
    /// Panics where
    /// [`try_union_type_from_logical`][Self::try_union_type_from_logical]
    /// would return an error.
    #[must_use]
    pub fn union_type_from_logical(members: &[(&str, Self)]) -> Self {
        Self::try_union_type_from_logical(members).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::union_type_from_logical`].
    ///
    /// # Errors
    ///
    /// Returns an error if there are no members or more than
    /// [`MAX_UNION_MEMBERS`], if two member names are equal ignoring ASCII
    /// case, if a name contains a null byte, or if `duckdb_create_union_type`
    /// returns null. `DuckDB`'s constructor checks none of these; its binder
    /// rejects all but the empty union the first time the type is used. SQL
    /// cannot write a `UNION` without members, and a `DuckDB` built with
    /// assertions refuses one, so it is refused here too.
    pub fn try_union_type_from_logical(members: &[(&str, Self)]) -> Result<Self, LogicalTypeError> {
        const API: &str = "duckdb_create_union_type";
        if members.is_empty() {
            return Err(LogicalTypeError::with_detail(
                API,
                "a UNION needs at least one member, as in SQL".to_owned(),
            ));
        }
        let names = checked_names(
            "UNION member",
            API,
            members.iter().map(|&(n, _)| n),
            Some(MAX_UNION_MEMBERS),
        )?;
        // SAFETY: `duckdb_create_union_type` is one of the two functions
        // `create_named` accepts.
        unsafe { create_named(duckdb_create_union_type, API, members, &names) }
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
    /// Panics where [`try_enum_type`][Self::try_enum_type] would return an
    /// error.
    #[must_use]
    pub fn enum_type(members: &[&str]) -> Self {
        Self::try_enum_type(members).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible version of [`LogicalType::enum_type`].
    ///
    /// # Errors
    ///
    /// Returns an error if a member name contains a null byte, or if
    /// `duckdb_create_enum_type` returns null (it does for duplicate members).
    pub fn try_enum_type(members: &[&str]) -> Result<Self, LogicalTypeError> {
        const API: &str = "duckdb_create_enum_type";
        let c_names: Vec<CString> = members
            .iter()
            .map(|&n| {
                CString::new(n).map_err(|_| {
                    LogicalTypeError::with_detail(
                        API,
                        format!("ENUM member name {n:?} contains a null byte"),
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        let mut name_ptrs: Vec<*const c_char> = c_names.iter().map(|s| s.as_ptr()).collect();
        // SAFETY: `name_ptrs` holds `members.len()` NUL-terminated strings that
        // outlive the call; DuckDB copies them and returns an owned handle or
        // null.
        let inner =
            unsafe { duckdb_create_enum_type(name_ptrs.as_mut_ptr(), members.len() as idx_t) };
        Self::owned_or(inner, API)
    }
}

#[cfg(test)]
mod tests {
    use super::{checked_names, LogicalType, TypeId, MAX_UNION_MEMBERS};

    fn names(list: &[&str], max: Option<usize>) -> Result<usize, String> {
        checked_names("STRUCT field", "api", list.iter().copied(), max)
            .map(|v| v.len())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn duplicate_names_are_refused_ignoring_ascii_case() {
        assert_eq!(names(&["a", "b"], None), Ok(2));
        let err = names(&["a", "b", "A"], None).expect_err("A collides with a");
        assert!(err.contains("duplicate STRUCT field name \"A\""), "{err}");
        assert!(names(&["x", "x"], None).is_err());
    }

    #[test]
    fn empty_names_are_allowed_and_may_repeat() {
        // An unnamed STRUCT (e.g. `row(1, 2)`) has an empty name per field.
        assert_eq!(names(&["", ""], None), Ok(2));
    }

    #[test]
    fn a_null_byte_is_refused() {
        let err = names(&["a\0b"], None).expect_err("interior NUL");
        assert!(err.contains("null byte"), "{err}");
    }

    #[test]
    fn the_member_limit_is_inclusive() {
        let owned: Vec<String> = (0..=MAX_UNION_MEMBERS).map(|i| format!("m{i}")).collect();
        let all: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert_eq!(
            names(&all[..MAX_UNION_MEMBERS], Some(MAX_UNION_MEMBERS)),
            Ok(256)
        );
        let err = names(&all, Some(MAX_UNION_MEMBERS)).expect_err("257 names");
        assert!(
            err.contains("257 names") && err.contains("at most 256"),
            "{err}"
        );
    }

    /// A composite `TypeId` among the `(name, TypeId)` members is refused
    /// while resolving them, before any `DuckDB` call (so no live runtime is
    /// needed), with the message that says which constructor to use instead.
    #[test]
    fn a_composite_member_id_is_refused_for_struct_and_union() {
        let members = [("xs", TypeId::List)];
        for err in [
            LogicalType::try_struct_type(&members).expect_err("STRUCT with a LIST id"),
            LogicalType::try_union_type(&members).expect_err("UNION with a LIST id"),
        ] {
            let msg = err.to_string();
            assert!(msg.contains("LIST"), "{msg}");
            assert!(msg.contains("bare TypeId"), "{msg}");
        }
    }
}
