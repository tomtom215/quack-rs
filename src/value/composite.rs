// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Composite constructors: `STRUCT`, `LIST`, `ARRAY`, `ENUM`, `MAP`, `UNION`.

use libduckdb_sys::duckdb_value;

use super::Value;
use crate::error::ExtensionError;

impl Value {
    // ── Composite constructors ───────────────────────────────────────────
    //
    // DuckDB *copies* every input value (`UnwrapValue` + `emplace_back` in
    // `duckdb_value-c.cpp`), so the caller keeps ownership of the children and
    // they are freed normally when their `Value`s drop.

    /// Creates a `STRUCT` value.
    ///
    /// `fields` must line up positionally with `struct_type`'s fields.
    ///
    /// # Errors
    ///
    /// - The number of `fields` does not match the type's field count. This
    ///   check is quack-rs's, and it is not cosmetic:
    ///   `duckdb_create_struct_value` takes **no count argument** and reads
    ///   `values[0..StructType::GetChildCount(type)]`, so passing a short slice
    ///   reads past its end.
    /// - `struct_type` is not a `STRUCT`, or contains an `ANY` / `INVALID` child
    ///   type — `duckdb_create_struct_value` returns null for those.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    /// use quack_rs::value::Value;
    ///
    /// # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
    /// let ty = LogicalType::struct_type(&[("x", TypeId::BigInt), ("y", TypeId::Varchar)]);
    /// let v = Value::struct_value(&ty, &[Value::bigint(1), Value::varchar("two")])?;
    /// # let _ = v;
    /// # Ok(())
    /// # }
    /// ```
    pub fn struct_value(
        struct_type: &crate::types::LogicalType,
        fields: &[Self],
    ) -> Result<Self, ExtensionError> {
        // SAFETY: `struct_type` is a live handle for the duration of the call.
        let expected = usize::try_from(unsafe { struct_type.struct_child_count() }).unwrap_or(0);
        if expected != fields.len() {
            return Err(ExtensionError::new(format!(
                "Value::struct_value: the type has {expected} field(s) but {} value(s) were \
                 given; duckdb_create_struct_value reads one value per field and would read \
                 out of bounds",
                fields.len()
            )));
        }
        let mut raws: Vec<duckdb_value> = fields.iter().map(Self::as_raw).collect();
        // SAFETY: `raws` has exactly `expected` entries, which is what DuckDB
        // reads; every entry is a live handle owned by `fields`, and DuckDB
        // copies rather than takes them.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_struct_value(struct_type.as_raw(), raws.as_mut_ptr())
        };
        Self::checked(raw, "duckdb_create_struct_value")
    }

    /// Creates a `LIST` value from its **element** type.
    ///
    /// # `element_type`, not the list type
    ///
    /// `duckdb.h` is self-contradictory here: the prose says "Creates a list
    /// value from a child (element) type", while the `@param` line says "The
    /// type of the list". The implementation settles it —
    /// `duckdb_create_list_value` forwards to
    /// `Value::LIST(const LogicalType &child_type, vector<Value>)`, so it is the
    /// **element** type, and passing `LogicalType::list(..)` instead makes
    /// `DuckDB` try to cast every element to a list and return null.
    ///
    /// For `LIST<BIGINT>`, pass `LogicalType::new(TypeId::BigInt)`. A list of
    /// lists is built the same way: for `BIGINT[][]`, pass
    /// `LogicalType::list(TypeId::BigInt)` and `LIST` items.
    ///
    /// # Errors
    ///
    /// Returns an error when `duckdb_create_list_value` reports failure: an
    /// `ANY` or `INVALID` element type, or an item that will not cast to
    /// `element_type`. When the element type is itself a `LIST`, the error
    /// also points out the likelier mistake — passing the list type with
    /// scalar items — since it is the one the header invites.
    pub fn list_value(
        element_type: &crate::types::LogicalType,
        items: &[Self],
    ) -> Result<Self, ExtensionError> {
        let mut raws: Vec<duckdb_value> = items.iter().map(Self::as_raw).collect();
        // SAFETY: the count is passed explicitly and matches `raws`; every entry
        // is a live handle owned by `items`.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_list_value(
                element_type.as_raw(),
                raws.as_mut_ptr(),
                libduckdb_sys::idx_t::try_from(raws.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self::checked_container(
            raw,
            element_type,
            crate::types::TypeId::List,
            "list_value",
            "duckdb_create_list_value",
        )
    }

    /// Creates an `ARRAY` (fixed-size list) value from its **element** type.
    ///
    /// As with [`list_value`][Self::list_value], this is the element type, not
    /// the array type — `duckdb_create_array_value` forwards to
    /// `Value::ARRAY(const LogicalType &child_type, vector<Value>)`, which
    /// *derives* the array type as `ARRAY(child_type, values.size())`. The
    /// resulting array's size is therefore `items.len()`; there is nothing to
    /// keep in sync. An array of arrays takes an `ARRAY` element type and
    /// `ARRAY` items.
    ///
    /// # Errors
    ///
    /// Returns an error when `duckdb_create_array_value` reports failure: an
    /// `ANY` / `INVALID` element type, an item that will not cast to
    /// `element_type`, or a length at or above `DuckDB`'s maximum array size.
    pub fn array_value(
        element_type: &crate::types::LogicalType,
        items: &[Self],
    ) -> Result<Self, ExtensionError> {
        let mut raws: Vec<duckdb_value> = items.iter().map(Self::as_raw).collect();
        // SAFETY: as in `list_value`.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_array_value(
                element_type.as_raw(),
                raws.as_mut_ptr(),
                libduckdb_sys::idx_t::try_from(raws.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self::checked_container(
            raw,
            element_type,
            crate::types::TypeId::Array,
            "array_value",
            "duckdb_create_array_value",
        )
    }

    /// Like [`checked`][Self::checked], but when `DuckDB` refused a container
    /// whose element type is itself `container`, names the likelier cause:
    /// the "passed the container type instead of the element type" mistake
    /// that `duckdb.h`'s contradictory `@param` text invites. (A nested
    /// element type with nested items is legitimate and succeeds.)
    fn checked_container(
        raw: duckdb_value,
        element_type: &crate::types::LogicalType,
        container: crate::types::TypeId,
        method: &str,
        api_func: &'static str,
    ) -> Result<Self, ExtensionError> {
        if !raw.is_null() {
            return Ok(Self { raw });
        }
        // SAFETY: `element_type` is a live handle for the duration of the call.
        let id = unsafe {
            crate::types::TypeId::try_from_duckdb_type(libduckdb_sys::duckdb_get_type_id(
                element_type.as_raw(),
            ))
        };
        if id != Some(container) {
            return Self::checked(raw, api_func);
        }
        Err(ExtensionError::new(format!(
            "Value::{method} takes the *element* type, and {api_func} could not cast the items \
             to the {sql} element type given. If you passed the {sql} type itself, pass the type \
             of one item instead (duckdb.h's prose says \"child (element) type\" while its \
             @param line says \"the type of the {lower}\"; the implementation takes the element \
             type). For a {lower} of {lower}s, every item must itself be a {sql}.",
            sql = container.sql_name(),
            lower = container.sql_name().to_lowercase()
        )))
    }

    /// Creates an `ENUM` value from its dictionary index.
    ///
    /// # Errors
    ///
    /// Returns an error when `enum_type` is not an `ENUM` or `index` is outside
    /// its dictionary.
    pub fn enum_value(
        enum_type: &crate::types::LogicalType,
        index: u64,
    ) -> Result<Self, ExtensionError> {
        // SAFETY: `enum_type` is a live handle for the duration of the call.
        let raw = unsafe { libduckdb_sys::duckdb_create_enum_value(enum_type.as_raw(), index) };
        Self::checked(raw, "duckdb_create_enum_value")
    }

    /// Wraps a possibly-null constructor result, naming the C function that
    /// produced it.
    fn checked(raw: duckdb_value, api_func: &'static str) -> Result<Self, ExtensionError> {
        if raw.is_null() {
            return Err(ExtensionError::new(format!(
                "{api_func} returned null: the logical type and the supplied values do not \
                 form a valid value (check the type's kind, its child types, and the value count)"
            )));
        }
        Ok(Self { raw })
    }

    /// Creates a `MAP` value from parallel key and value slices.
    ///
    /// Named `map` rather than `map_value` because
    /// [`map_value`][Self::map_value] is already the accessor that reads the
    /// value of the `index`-th entry.
    ///
    /// `map_type` is the `MAP` type itself. Requires `duckdb-1-5`:
    /// `duckdb_create_map_value` sits in the unstable region of the C API
    /// struct.
    ///
    /// # Errors
    ///
    /// Returns an error when the slices differ in length, or when
    /// `duckdb_create_map_value` reports failure — `map_type` is not a `MAP`,
    /// a child type is `ANY` / `INVALID`, or a key repeats.
    #[cfg(feature = "duckdb-1-5")]
    pub fn map(
        map_type: &crate::types::LogicalType,
        keys: &[Self],
        values: &[Self],
    ) -> Result<Self, ExtensionError> {
        if keys.len() != values.len() {
            return Err(ExtensionError::new(format!(
                "Value::map: {} key(s) but {} value(s)",
                keys.len(),
                values.len()
            )));
        }
        let mut key_raws: Vec<duckdb_value> = keys.iter().map(Self::as_raw).collect();
        let mut val_raws: Vec<duckdb_value> = values.iter().map(Self::as_raw).collect();
        // SAFETY: both slices have exactly `entry_count` live handles, checked
        // equal above; DuckDB copies rather than takes them.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_map_value(
                map_type.as_raw(),
                key_raws.as_mut_ptr(),
                val_raws.as_mut_ptr(),
                libduckdb_sys::idx_t::try_from(key_raws.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self::checked(raw, "duckdb_create_map_value")
    }

    /// Creates a `UNION` value for the member at `tag_index`.
    ///
    /// Requires `duckdb-1-5`: `duckdb_create_union_value` sits in the unstable
    /// region of the C API struct.
    ///
    /// # Errors
    ///
    /// Returns an error when `union_type` is not a `UNION`, `tag_index` is out
    /// of range, or `value`'s type does not equal that member's declared type —
    /// `DuckDB` compares them exactly and returns null on a mismatch.
    #[cfg(feature = "duckdb-1-5")]
    pub fn union_value(
        union_type: &crate::types::LogicalType,
        tag_index: u64,
        value: &Self,
    ) -> Result<Self, ExtensionError> {
        // SAFETY: both handles are live for the duration of the call, and
        // DuckDB copies the value rather than taking it.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_union_value(union_type.as_raw(), tag_index, value.as_raw())
        };
        Self::checked(raw, "duckdb_create_union_value")
    }
}
