// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Casting a value to any logical type, and reading `ARRAY` and `UNION`
//! values, whose contents the C API has no direct reader for.
//!
//! `duckdb_get_list_size` / `duckdb_get_list_child` read a `LIST` only and
//! `duckdb_get_struct_child` a `STRUCT` only (`duckdb_value-c.cpp`, 1.4.4,
//! 1.5.6 and 2.0.0 alike): an `ARRAY` reads as empty and a `UNION` as having
//! no fields. A cast gets around both, and the cast here cannot throw through
//! the C API: it is the one-element-`LIST` construction the scalar getters'
//! `Value::cast_copy` uses, whose `duckdb_create_list_value` runs
//! `Value::LIST` (and so `DefaultCastAs`) inside `try { } catch (...)` on all
//! three releases.

use libduckdb_sys::{
    duckdb_array_type_child_type, duckdb_create_list_value, duckdb_get_list_child,
    duckdb_get_value_type,
};

use super::temporal_checks::cast_guard;
use super::Value;
use crate::types::{LogicalType, TypeId};

impl Value {
    /// The value cast to `ty`, as a new owned handle, or `None` when `DuckDB`
    /// refuses the cast. No checks of its own: every caller decides first
    /// whether the payload is safe to cast.
    ///
    /// `Value::LIST(child_type, values)` casts each value with
    /// `DefaultCastAs(child_type)`, which throws on a failed cast, and
    /// `duckdb_create_list_value` catches the throw and returns null. The
    /// child is then copied back out (`duckdb_get_list_child`).
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    pub(super) fn cast_into(&self, ty: &LogicalType) -> Option<Self> {
        if self.raw.is_null() {
            return None;
        }
        let mut child = self.raw;
        // SAFETY: `ty` is a live type owned by the caller; `child` points to
        // exactly one live handle, matching `value_count == 1`. DuckDB copies
        // and casts it, and catches every exception the cast throws.
        let list = unsafe { duckdb_create_list_value(ty.as_raw(), &raw mut child, 1) };
        // SAFETY: `list` is null (never dereferenced by `Value`) or an owned
        // handle returned by DuckDB.
        let list = unsafe { Self::from_raw(list) };
        if list.is_null() {
            return None;
        }
        // SAFETY: `list` is a live, non-NULL `LIST` with one child.
        let copy = unsafe { duckdb_get_list_child(list.raw, 0) };
        // SAFETY: `copy` is null or a new owned handle.
        let copy = unsafe { Self::from_raw(copy) };
        (!copy.is_null()).then_some(copy)
    }

    /// Casts the value to `target`, as SQL's `TRY_CAST(value AS target)`
    /// does, and returns the result as a new owned value; `None` when the cast
    /// fails.
    ///
    /// This is the general form of the casting `as_*` getters, for any
    /// [`LogicalType`] — nested ones included — and the way to read the
    /// values the C API has no reader for:
    ///
    /// - an `ARRAY` cast to a `LIST` of its element type reads with
    ///   [`list_items`][Self::list_items] (or use
    ///   [`array_items`][Self::array_items], which does exactly that);
    /// - a `UNION` cast to `VARCHAR` is its active member as text
    ///   (`DuckDB` casts a `UNION` only to `VARCHAR` or to another `UNION`);
    ///   `Value::union_tag` (feature `duckdb-1-5`) says which member it is;
    /// - a `VARCHAR` cast to `LIST(INTEGER)`, `STRUCT(..)` or `DATE` parses
    ///   it the way SQL does.
    ///
    /// A SQL `NULL` casts to a `NULL` of the target type (`Some`), as in SQL.
    /// `self` is never modified.
    ///
    /// Returns `None` when the handle is null, the cast fails (`'abc'` to
    /// `INTEGER`, a `LIST` to `INTEGER`, an `ARRAY` to a `LIST` of the wrong
    /// size), `target` is `ANY` / `INVALID` or a type quack-rs does not
    /// know, or the value holds a payload `DuckDB` cannot convert safely: the
    /// same timestamp, time and `DECIMAL` payloads
    /// [`as_str`][Self::as_str] refuses to render (see
    /// [`UNRENDERABLE`][super::UNRENDERABLE]), anywhere inside the value.
    /// Those payloads do not come from SQL's own constructors, but from
    /// functions such as `make_timestamp(-9223372036854775808)`; converting
    /// one throws, or overflows a signed multiply, inside `DuckDB`. A `UNION`
    /// whose member types include a timestamp, time or `DECIMAL` is refused
    /// whatever its member holds, because the member cannot be read to check
    /// it first.
    ///
    /// Works on every supported `DuckDB` (1.4.4 to 2.0.x): it uses only
    /// stable C API calls.
    ///
    /// # Example
    ///
    /// Reading an `INTEGER[3]` parameter of a table function (needs a live
    /// engine, so it is not run here):
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    /// use quack_rs::value::Value;
    ///
    /// # fn demo(param: &Value) -> Option<Vec<i32>> {
    /// let as_list = param.try_cast(&LogicalType::list(TypeId::Integer))?;
    /// as_list.list_items().iter().map(Value::as_i32).collect()
    /// # }
    /// ```
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn try_cast(&self, target: &LogicalType) -> Option<Self> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: `target` is a live handle (a `LogicalType` invariant), and
        // a non-null `Value` means the C API is initialised.
        let target_id = unsafe { target.try_get_type_id() }?;
        if !self.is_sql_null() && !self.payload_casts_safely(target_id) {
            return None;
        }
        let copy = self.cast_into(target)?;
        (copy.type_id() == Some(target_id)).then_some(copy)
    }

    /// Whether every payload inside this (non-NULL) value is one `DuckDB`
    /// converts without throwing through the C API or overflowing: the
    /// `renderable` check (`render_guard.rs`), element by element, with an
    /// `ARRAY` first turned into a `LIST` of its own element type so that its
    /// elements can be checked too. Then, as in the scalar getters, a
    /// timestamp-to-time pair `DuckDB` converts with a throwing loop is
    /// checked against the exact condition it fails on.
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    fn payload_casts_safely(&self, target: TypeId) -> bool {
        let Some(source) = self.type_id() else {
            return false;
        };
        let checked = if source == TypeId::Array {
            self.array_as_list().is_some_and(|list| list.renderable())
        } else {
            self.renderable()
        };
        if !checked {
            return false;
        }
        cast_guard(source, target).is_none_or(|guard| {
            self.temporal_payload(source)
                .is_some_and(|v| guard.accepts(v))
        })
    }

    /// An `ARRAY` value cast to a `LIST` of its own element type; `None` for
    /// any other value.
    ///
    /// The cast cannot convert an element: `ArrayToListCast` binds its
    /// element cast with `GetCastFunction(child, child)`, which is `NopCast`
    /// when the two types are equal (`cast_function_set.cpp`, 1.4.4, 1.5.6
    /// and 2.0.0), so no element payload is read. That is what lets
    /// [`try_cast`][Self::try_cast] check the elements before a real cast.
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    fn array_as_list(&self) -> Option<Self> {
        if self.type_id() != Some(TypeId::Array) {
            return None;
        }
        // SAFETY: `self.raw` is a live, non-null value (`type_id` returned
        // `Some`). The type is owned by the value and is only read here.
        let array_type = unsafe { duckdb_get_value_type(self.raw) };
        // SAFETY: `array_type` is a live `ARRAY` type. DuckDB returns a new
        // type the caller owns, or null.
        let element = unsafe { duckdb_array_type_child_type(array_type) };
        if element.is_null() {
            return None;
        }
        // SAFETY: `element` is a non-null, owned handle; `LogicalType`
        // destroys it on drop.
        let element = unsafe { LogicalType::from_raw(element) };
        let list_type = LogicalType::try_list_from_logical(&element).ok()?;
        self.cast_into(&list_type)
    }

    /// The elements of an `ARRAY` (or a `LIST`) value, in order; `None` for
    /// a null handle, a SQL `NULL` or a value of any other type.
    ///
    /// [`list_items`][Self::list_items] reads a `LIST` only, and an `ARRAY`
    /// value read through it looks empty: the C API has no `ARRAY` reader.
    /// This casts an `ARRAY(T, n)` to `LIST(T)` first — a cast that copies
    /// the elements unchanged — so a fixed-size-list parameter of a table
    /// function, a `COPY` option or a scalar bind argument can be read.
    /// Each element is an owned [`Value`] of type `T`, read with the usual
    /// getters.
    ///
    /// Works on every supported `DuckDB` (1.4.4 to 2.0.x).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::value::Value;
    ///
    /// # fn demo(point: &Value) -> Option<(f64, f64)> {
    /// // A `DOUBLE[2]` parameter.
    /// let items = point.array_items()?;
    /// Some((items.first()?.as_f64()?, items.get(1)?.as_f64()?))
    /// # }
    /// ```
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn array_items(&self) -> Option<Vec<Self>> {
        if self.is_sql_null() {
            return None;
        }
        match self.type_id()? {
            TypeId::List => Some(self.list_items()),
            TypeId::Array => Some(self.array_as_list()?.list_items()),
            _ => None,
        }
    }
}

#[cfg(feature = "duckdb-1-5")]
impl Value {
    /// The tag of a `UNION` value: the 0-based index of its active member,
    /// in the order the `UNION` type declares them (`DuckDB` 1.5.0+).
    ///
    /// Returns `None` for a null handle, a SQL `NULL` and any value that is
    /// not a `UNION`.
    ///
    /// The member's name is entry `tag + 1` of
    /// [`struct_field_names`][Self::struct_field_names] (a `UNION` lists its
    /// hidden tag field first). Its value reads as text
    /// with [`try_cast`][Self::try_cast] to `VARCHAR`, the only scalar type
    /// `DuckDB` casts a `UNION` to; a member whose value is `NULL` reads as
    /// the text `"NULL"` there (`UnionToVarcharCast`), which cannot be told
    /// apart from a `VARCHAR` member holding that text.
    ///
    /// # How
    ///
    /// The C API reads a `UNION` value's tag nowhere: `duckdb_get_struct_child`
    /// refuses a `UNION`, and no cast yields the tag. A `UNION` is stored as a
    /// `STRUCT` whose field 0 is the `UTINYINT` tag, so this references the
    /// value into a one-row vector of its own type
    /// (`duckdb_vector_reference_value`) and reads field 0 there. Requires
    /// `duckdb-1-5`: `duckdb_create_vector` and
    /// `duckdb_vector_reference_value` sit in the unstable region of the C API
    /// struct.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    /// use quack_rs::value::Value;
    ///
    /// # fn demo(arg: &Value) -> Option<String> {
    /// // A `UNION(num INTEGER, str VARCHAR)` parameter.
    /// let tag = arg.union_tag()?;
    /// let member = arg.struct_field_names().get(usize::from(tag) + 1)?.clone();
    /// let text = arg.try_cast(&LogicalType::new(TypeId::Varchar))?.as_str().ok()?;
    /// Some(format!("{member} = {text}"))
    /// # }
    /// ```
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn union_tag(&self) -> Option<u8> {
        /// Destroys a vector `duckdb_create_vector` returned.
        struct OwnedVector(libduckdb_sys::duckdb_vector);
        impl Drop for OwnedVector {
            fn drop(&mut self) {
                // SAFETY: `self.0` came from `duckdb_create_vector`, is
                // non-null, and is destroyed exactly once, here.
                unsafe { libduckdb_sys::duckdb_destroy_vector(&raw mut self.0) };
            }
        }

        if self.is_sql_null() || self.type_id() != Some(TypeId::Union) {
            return None;
        }
        // SAFETY: `self.raw` is a live, non-null `UNION` value; its type is
        // owned by the value and only read (copied) below.
        let union_type = unsafe { duckdb_get_value_type(self.raw) };
        // SAFETY: `union_type` is live; `duckdb_create_vector` copies it and
        // catches every exception, returning null on failure.
        let vector = unsafe { libduckdb_sys::duckdb_create_vector(union_type, 1) };
        if vector.is_null() {
            return None;
        }
        let vector = OwnedVector(vector);
        // SAFETY: the vector was created from this value's own type, which is
        // what `Vector::Reference(const Value &)` requires (it asserts the
        // type ids match and lays the value out by the vector's type); the
        // value is non-NULL, so every field, the tag included, is written at
        // row 0.
        unsafe { libduckdb_sys::duckdb_vector_reference_value(vector.0, self.raw) };
        // SAFETY: a `UNION` vector is a `STRUCT` vector whose child 0 is the
        // tag; index 0 exists for every `UNION` type.
        let tags = unsafe { libduckdb_sys::duckdb_struct_vector_get_child(vector.0, 0) };
        if tags.is_null() {
            return None;
        }
        // SAFETY: `tags` is a flat or constant `UTINYINT` vector owned by
        // `vector`, which is still alive.
        let data = unsafe { libduckdb_sys::duckdb_vector_get_data(tags) }.cast::<u8>();
        if data.is_null() {
            return None;
        }
        // SAFETY: row 0 of a one-row `UTINYINT` vector, written by
        // `duckdb_vector_reference_value` above.
        Some(unsafe { *data })
    }
}
