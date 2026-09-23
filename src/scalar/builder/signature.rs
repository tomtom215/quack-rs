// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Detecting overloads that declare the same argument types.
//!
//! `DuckDB` accepts a function set in which two overloads take identical
//! arguments — registration succeeds — and then fails *every* call to it with
//! "Could not choose a best candidate function". Nothing points back at the
//! registration. The set builders call [`reject_duplicate_overloads`] before
//! allocating any `DuckDB` handle so the mistake is reported where it was made.
//!
//! Shared by the scalar and aggregate set builders.

use std::fmt::Write as _;
use std::os::raw::c_char;

use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_array_type_child_type, duckdb_decimal_scale,
    duckdb_decimal_width, duckdb_enum_dictionary_size, duckdb_enum_dictionary_value, duckdb_free,
    duckdb_get_type_id, duckdb_list_type_child_type, duckdb_logical_type,
    duckdb_logical_type_get_alias, duckdb_map_type_key_type, duckdb_map_type_value_type,
    duckdb_struct_type_child_count, duckdb_struct_type_child_name, duckdb_struct_type_child_type,
    duckdb_union_type_member_count, duckdb_union_type_member_name, duckdb_union_type_member_type,
    idx_t, DUCKDB_TYPE_DUCKDB_TYPE_ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL,
    DUCKDB_TYPE_DUCKDB_TYPE_ENUM, DUCKDB_TYPE_DUCKDB_TYPE_LIST, DUCKDB_TYPE_DUCKDB_TYPE_MAP,
    DUCKDB_TYPE_DUCKDB_TYPE_STRUCT, DUCKDB_TYPE_DUCKDB_TYPE_UNION,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};

/// One declared parameter, however it was declared.
///
/// `L` is always [`LogicalType`] outside this module's tests, which use a
/// stand-in so the interleaving can be checked without a live `DuckDB`.
pub enum ParamRef<'a, L = LogicalType> {
    /// Declared with `param(TypeId)`.
    Id(TypeId),
    /// Declared with `param_logical(LogicalType)`.
    Logical(&'a L),
}

/// The parameters in the order the set builders hand them to `DuckDB`.
///
/// This mirrors the interleaving loop in both `register` implementations
/// exactly — including what it does with an out-of-order logical position — so
/// the signature compared here is the one `DuckDB` actually receives.
pub fn merged_params<'a, L>(
    params: &'a [TypeId],
    logical: &'a [(usize, L)],
) -> Vec<ParamRef<'a, L>> {
    let mut out = Vec::with_capacity(params.len() + logical.len());
    let mut simple_idx = 0;
    let mut logical_idx = 0;
    for pos in 0..params.len() + logical.len() {
        if logical_idx < logical.len() && logical[logical_idx].0 == pos {
            out.push(ParamRef::Logical(&logical[logical_idx].1));
            logical_idx += 1;
        } else if simple_idx < params.len() {
            out.push(ParamRef::Id(params[simple_idx]));
            simple_idx += 1;
        }
    }
    out
}

/// Fails if two overloads declare the same argument types.
///
/// `signatures[i]` is overload `i`'s merged parameter list. Types are compared
/// structurally — `DECIMAL(18,2)` and `DECIMAL(18,3)` differ, as do two
/// `STRUCT`s with different field names, two `ENUM`s with different members,
/// or two types with different aliases — so only a set `DuckDB` genuinely
/// cannot resolve is rejected.
///
/// # Safety
///
/// Every [`ParamRef::Logical`] must hold a live logical type, and the C API
/// must be initialised if any is present. A signature made only of
/// [`ParamRef::Id`]s makes no `DuckDB` call.
pub unsafe fn reject_duplicate_overloads(
    name: &str,
    signatures: &[Vec<ParamRef<'_>>],
) -> Result<(), ExtensionError> {
    let keys: Vec<String> = signatures
        .iter()
        // SAFETY: forwarded from this function's own contract.
        .map(|params| unsafe { signature_key(params) })
        .collect();
    match first_duplicate(&keys) {
        None => Ok(()),
        Some((first, second)) => Err(ExtensionError::new(format!(
            "function set '{name}': overload {first} and overload {second} both take \
             ({sig}); DuckDB would accept the set but fail every call to it with \
             \"Could not choose a best candidate function\"",
            sig = keys[first]
        ))),
    }
}

/// The indices of the first pair of equal keys, lowest first.
fn first_duplicate(keys: &[String]) -> Option<(usize, usize)> {
    keys.iter().enumerate().find_map(|(j, key)| {
        keys[..j]
            .iter()
            .position(|earlier| earlier == key)
            .map(|i| (i, j))
    })
}

/// Renders a parameter list as a comparable, human-readable string.
///
/// # Safety
///
/// As [`reject_duplicate_overloads`].
unsafe fn signature_key(params: &[ParamRef<'_>]) -> String {
    let mut out = String::new();
    for (i, param) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match param {
            ParamRef::Id(id) => out.push_str(id.sql_name()),
            // SAFETY: the handle is live per this function's contract.
            ParamRef::Logical(lt) => unsafe { describe(lt.as_raw(), &mut out) },
        }
    }
    out
}

/// A child logical type handle, destroyed on drop.
///
/// A non-null handle is also held as a [`LogicalType`], whose own `Drop`
/// destroys it; a null one (an accessor that could not describe the child) is
/// never passed to `duckdb_destroy_logical_type`.
struct Owned {
    /// The handle, or null.
    raw: duckdb_logical_type,
    /// Destroys `raw` on drop; `None` exactly when `raw` is null.
    _guard: Option<LogicalType>,
}

impl Owned {
    /// Takes ownership of `raw`.
    ///
    /// # Safety
    ///
    /// `raw` must be null, or a handle returned by a `*_child_type` style
    /// accessor, which transfers ownership to the caller.
    unsafe fn new(raw: duckdb_logical_type) -> Self {
        Self {
            raw,
            // SAFETY: non-null, and ours to destroy, per this function's contract.
            _guard: (!raw.is_null()).then(|| unsafe { LogicalType::from_raw(raw) }),
        }
    }
}

/// Copies and frees a `DuckDB`-allocated C string. Null becomes `"?"`.
///
/// # Safety
///
/// `ptr` must be null or a string `DuckDB` allocated for the caller to free.
unsafe fn take_c_string(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        return String::from("?");
    }
    // SAFETY: `ptr` is a live NUL-terminated string per this function's contract.
    let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `ptr` was allocated by DuckDB for us to free, and is freed once.
    unsafe { duckdb_free(ptr.cast()) };
    s
}

/// Appends a structural rendering of `ty` to `out`.
///
/// # Safety
///
/// `ty` must be a live logical type handle.
unsafe fn describe(ty: duckdb_logical_type, out: &mut String) {
    // SAFETY (whole body): `ty` is live per this function's contract; every
    // accessor is only called for the type id it is documented for, and every
    // child handle is owned by an `Owned` guard.
    unsafe {
        let alias = duckdb_logical_type_get_alias(ty);
        if !alias.is_null() {
            let _ = write!(out, "{}:", take_c_string(alias));
        }
        let raw = duckdb_get_type_id(ty);
        match raw {
            DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL => {
                let _ = write!(
                    out,
                    "DECIMAL({},{})",
                    duckdb_decimal_width(ty),
                    duckdb_decimal_scale(ty)
                );
            }
            DUCKDB_TYPE_DUCKDB_TYPE_LIST => {
                let child = Owned::new(duckdb_list_type_child_type(ty));
                out.push_str("LIST(");
                describe(child.raw, out);
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_ARRAY => {
                let child = Owned::new(duckdb_array_type_child_type(ty));
                out.push_str("ARRAY(");
                describe(child.raw, out);
                let _ = write!(out, ", {})", duckdb_array_type_array_size(ty));
            }
            DUCKDB_TYPE_DUCKDB_TYPE_MAP => {
                let key = Owned::new(duckdb_map_type_key_type(ty));
                let value = Owned::new(duckdb_map_type_value_type(ty));
                out.push_str("MAP(");
                describe(key.raw, out);
                out.push_str(", ");
                describe(value.raw, out);
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_STRUCT => {
                out.push_str("STRUCT(");
                for i in 0..duckdb_struct_type_child_count(ty) {
                    describe_member(
                        out,
                        i,
                        duckdb_struct_type_child_name(ty, i),
                        duckdb_struct_type_child_type(ty, i),
                    );
                }
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_UNION => {
                out.push_str("UNION(");
                for i in 0..duckdb_union_type_member_count(ty) {
                    describe_member(
                        out,
                        i,
                        duckdb_union_type_member_name(ty, i),
                        duckdb_union_type_member_type(ty, i),
                    );
                }
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_ENUM => {
                out.push_str("ENUM(");
                for i in 0..idx_t::from(duckdb_enum_dictionary_size(ty)) {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(
                        out,
                        "{:?}",
                        take_c_string(duckdb_enum_dictionary_value(ty, i))
                    );
                }
                out.push(')');
            }
            other => match TypeId::try_from_duckdb_type(other) {
                Some(id) => out.push_str(id.sql_name()),
                None => {
                    let _ = write!(out, "TYPE#{other}");
                }
            },
        }
    }
}

/// Appends `name type` for one `STRUCT` field or `UNION` member, taking
/// ownership of both handles.
///
/// # Safety
///
/// `name` must be null or a `DuckDB`-allocated string, and `ty` a logical type
/// handle the caller owns.
unsafe fn describe_member(out: &mut String, i: idx_t, name: *mut c_char, ty: duckdb_logical_type) {
    // SAFETY: forwarded from this function's own contract.
    let ty = unsafe { Owned::new(ty) };
    if i > 0 {
        out.push_str(", ");
    }
    // SAFETY: forwarded from this function's own contract.
    let name = unsafe { take_c_string(name) };
    let _ = write!(out, "{name:?} ");
    // SAFETY: `ty` is a live handle owned by the guard above.
    unsafe { describe(ty.raw, out) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(sigs: &[&[TypeId]]) -> Vec<String> {
        sigs.iter()
            .map(|params| {
                let refs: Vec<ParamRef<'_>> = params.iter().map(|id| ParamRef::Id(*id)).collect();
                // SAFETY: only `ParamRef::Id`, so no DuckDB call is made.
                unsafe { signature_key(&refs) }
            })
            .collect()
    }

    #[test]
    fn distinct_signatures_have_no_duplicate() {
        let k = keys(&[
            &[TypeId::BigInt],
            &[TypeId::Integer],
            &[TypeId::BigInt, TypeId::BigInt],
            &[],
        ]);
        assert_eq!(first_duplicate(&k), None);
    }

    #[test]
    fn the_first_duplicate_pair_is_reported_lowest_first() {
        let k = keys(&[
            &[TypeId::Double],
            &[TypeId::BigInt],
            &[TypeId::Varchar],
            &[TypeId::BigInt],
            &[TypeId::Double],
        ]);
        // Overload 3 is the first to repeat an earlier one (1).
        assert_eq!(first_duplicate(&k), Some((1, 3)));
    }

    #[test]
    fn two_zero_argument_overloads_are_duplicates() {
        assert_eq!(first_duplicate(&keys(&[&[], &[]])), Some((0, 1)));
    }

    #[test]
    fn the_error_names_both_overloads_and_the_signature() {
        let sigs = vec![
            vec![ParamRef::Id(TypeId::BigInt), ParamRef::Id(TypeId::Varchar)],
            vec![ParamRef::Id(TypeId::Double)],
            vec![ParamRef::Id(TypeId::BigInt), ParamRef::Id(TypeId::Varchar)],
        ];
        // SAFETY: only `ParamRef::Id`, so no DuckDB call is made.
        let err = unsafe { reject_duplicate_overloads("my_fn", &sigs) }.expect_err("duplicate");
        let msg = err.as_str();
        assert!(msg.contains("my_fn"), "{msg}");
        assert!(msg.contains("overload 0 and overload 2"), "{msg}");
        assert!(msg.contains("(BIGINT, VARCHAR)"), "{msg}");
    }

    /// `merged_params` with `&str` stand-ins for the logical types, rendered
    /// as `Id(..)` / `Logical(..)` so a whole ordering compares at once.
    fn merged(params: &[TypeId], logical: &[(usize, &'static str)]) -> Vec<String> {
        merged_params(params, logical)
            .iter()
            .map(|p| match p {
                ParamRef::Id(id) => format!("Id({})", id.sql_name()),
                ParamRef::Logical(name) => format!("Logical({name})"),
            })
            .collect()
    }

    #[test]
    fn merged_params_places_logical_params_at_their_positions() {
        assert_eq!(
            merged(
                &[TypeId::BigInt, TypeId::Varchar, TypeId::Double],
                &[(0, "a"), (2, "b")]
            ),
            vec![
                "Logical(a)",
                "Id(BIGINT)",
                "Logical(b)",
                "Id(VARCHAR)",
                "Id(DOUBLE)",
            ]
        );
        // A trailing logical parameter, after every simple one.
        assert_eq!(
            merged(&[TypeId::BigInt, TypeId::Varchar], &[(2, "z")]),
            vec!["Id(BIGINT)", "Id(VARCHAR)", "Logical(z)"]
        );
        // Only logical parameters.
        assert_eq!(
            merged(&[], &[(0, "a"), (1, "b")]),
            vec!["Logical(a)", "Logical(b)"]
        );
        assert_eq!(merged(&[], &[]), Vec::<String>::new());
    }

    #[test]
    fn merged_params_mirrors_register_for_out_of_order_positions() {
        // `register` walks positions 0..total and only takes a logical type
        // when the next one's position is the current one. A position past the
        // end is never reached, so that logical type is left out and the slot
        // it would have filled stays empty.
        assert_eq!(
            merged(&[TypeId::BigInt], &[(5, "late")]),
            vec!["Id(BIGINT)"]
        );
        // Positions listed out of order: (2) blocks the queue, so (0) is never
        // reached either, and the simple parameters fill the first slots.
        assert_eq!(
            merged(&[TypeId::BigInt], &[(2, "two"), (0, "zero")]),
            vec!["Id(BIGINT)", "Logical(two)"]
        );
    }

    #[test]
    fn a_null_child_handle_is_held_but_never_destroyed() {
        // SAFETY: null is allowed; dropping it must not reach
        // `duckdb_destroy_logical_type`, which would panic without a live
        // DuckDB here.
        let child = unsafe { Owned::new(std::ptr::null_mut()) };
        assert!(child.raw.is_null());
        drop(child);
    }

    #[test]
    fn merged_params_keeps_simple_params_in_order() {
        let merged = merged_params::<LogicalType>(&[TypeId::BigInt, TypeId::Varchar], &[]);
        let ids: Vec<_> = merged
            .iter()
            .map(|p| match p {
                ParamRef::Id(id) => Some(*id),
                ParamRef::Logical(_) => None,
            })
            .collect();
        assert_eq!(ids, vec![Some(TypeId::BigInt), Some(TypeId::Varchar)]);
    }
}
