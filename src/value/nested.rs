// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Reading nested values: `LIST` elements, `STRUCT` fields, `MAP` entries.

use libduckdb_sys::duckdb_free;

use super::Value;

impl Value {
    // ── LIST / STRUCT / MAP extraction ───────────────────────────────────

    /// Number of elements in a `LIST` value.
    ///
    /// Returns 0 for non-`LIST` values.
    #[inline]
    #[must_use]
    pub fn list_len(&self) -> usize {
        // SAFETY: self.raw is valid per constructor contract.
        usize::try_from(unsafe { libduckdb_sys::duckdb_get_list_size(self.raw) }).unwrap_or(0)
    }

    /// Element `index` of a `LIST` value, or `None` if out of range.
    ///
    /// The returned [`Value`] owns its handle.
    #[must_use]
    pub fn list_child(&self, index: usize) -> Option<Self> {
        if index >= self.list_len() {
            return None;
        }
        // SAFETY: `index` was bounds-checked against `list_len`.
        let raw = unsafe {
            libduckdb_sys::duckdb_get_list_child(self.raw, index as libduckdb_sys::idx_t)
        };
        (!raw.is_null()).then(|| Self { raw })
    }

    /// Collects a `LIST` value into a `Vec` of owned [`Value`]s.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use quack_rs::value::Value;
    /// # fn demo(paths: &Value) {
    /// let files: Vec<String> = paths
    ///     .list_items()
    ///     .iter()
    ///     .filter_map(|v| v.as_str().ok())
    ///     .collect();
    /// # }
    /// ```
    #[must_use]
    pub fn list_items(&self) -> Vec<Self> {
        (0..self.list_len())
            .filter_map(|i| self.list_child(i))
            .collect()
    }

    /// Field names of a `STRUCT` value, in order.
    ///
    /// `DuckDB`'s C API exposes a struct value's children by position only —
    /// the names live on the value's `LogicalType`. Reading them by hand means
    /// calling `duckdb_get_value_type`, whose result `duckdb.h` says "must not
    /// be destroyed" because the *value* owns it, which is exactly the kind of
    /// borrowed handle it is easy to hand to
    /// [`LogicalType::from_raw`][crate::types::LogicalType::from_raw] and then
    /// double-free (pitfall P11). This does the walk without exposing it.
    ///
    /// Returns an empty vector for a null handle or a value whose type has no
    /// struct children. A `UNION` type is stored as a struct of its tag and
    /// its members, so for a `UNION` value it returns `""` for the tag
    /// followed by the member names. Those names cannot be paired with
    /// values: [`struct_child`][Self::struct_child] returns `None` for every
    /// index of a `UNION` value, and the C API has no other way to read its
    /// member.
    ///
    /// Pair it with [`struct_child`][Self::struct_child], which is positional:
    ///
    /// ```rust,no_run
    /// # use quack_rs::value::Value;
    /// # fn demo(options: &Value) -> Option<String> {
    /// let names = options.struct_field_names();
    /// let idx = names.iter().position(|n| n == "compression")?;
    /// options.struct_child(idx)?.as_str().ok()
    /// # }
    /// ```
    #[must_use]
    pub fn struct_field_names(&self) -> Vec<String> {
        if self.raw.is_null() {
            return Vec::new();
        }
        // SAFETY: `self.raw` is a valid duckdb_value per the constructor
        // contract. The returned type is owned by the value — duckdb.h: "The
        // type itself must not be destroyed" — so it is only read through, never
        // wrapped in `LogicalType` and never destroyed here.
        let logical = unsafe { libduckdb_sys::duckdb_get_value_type(self.raw) };
        if logical.is_null() {
            return Vec::new();
        }
        // SAFETY: `logical` is non-null and lives as long as `self`.
        let count = unsafe { libduckdb_sys::duckdb_struct_type_child_count(logical) };
        (0..count)
            .map(|i| {
                // SAFETY: `i < count`. DuckDB allocates the name with
                // `duckdb_malloc`, so it is freed with `duckdb_free`.
                let ptr = unsafe { libduckdb_sys::duckdb_struct_type_child_name(logical, i) };
                if ptr.is_null() {
                    return String::new();
                }
                // SAFETY: `ptr` is a NUL-terminated string owned by us.
                let owned = unsafe { std::ffi::CStr::from_ptr(ptr) }
                    .to_str()
                    .unwrap_or_default()
                    .to_owned();
                // SAFETY: allocated by DuckDB, freed exactly once here.
                unsafe { duckdb_free(ptr.cast::<std::os::raw::c_void>()) };
                owned
            })
            .collect()
    }

    /// Field `index` of a `STRUCT` value, or `None` if the handle is null or the
    /// index is out of range.
    ///
    /// Field names come from the value's `LogicalType`, not from the value
    /// itself; `DuckDB`'s C API exposes children by position.
    #[must_use]
    pub fn struct_child(&self, index: usize) -> Option<Self> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: self.raw is valid; DuckDB returns null for an out-of-range index.
        let raw = unsafe {
            libduckdb_sys::duckdb_get_struct_child(self.raw, index as libduckdb_sys::idx_t)
        };
        (!raw.is_null()).then(|| Self { raw })
    }

    /// Number of key/value pairs in a `MAP` value.
    #[inline]
    #[must_use]
    pub fn map_len(&self) -> usize {
        // SAFETY: self.raw is valid per constructor contract.
        usize::try_from(unsafe { libduckdb_sys::duckdb_get_map_size(self.raw) }).unwrap_or(0)
    }

    /// Key at `index` of a `MAP` value, or `None` if out of range.
    #[must_use]
    pub fn map_key(&self, index: usize) -> Option<Self> {
        if index >= self.map_len() {
            return None;
        }
        // SAFETY: `index` was bounds-checked against `map_len`.
        let raw =
            unsafe { libduckdb_sys::duckdb_get_map_key(self.raw, index as libduckdb_sys::idx_t) };
        (!raw.is_null()).then(|| Self { raw })
    }

    /// Value at `index` of a `MAP` value, or `None` if out of range.
    #[must_use]
    pub fn map_value(&self, index: usize) -> Option<Self> {
        if index >= self.map_len() {
            return None;
        }
        // SAFETY: `index` was bounds-checked against `map_len`.
        let raw =
            unsafe { libduckdb_sys::duckdb_get_map_value(self.raw, index as libduckdb_sys::idx_t) };
        (!raw.is_null()).then(|| Self { raw })
    }
}
