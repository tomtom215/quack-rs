// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Detects logical types that `DuckDB` refuses without saying so.
//!
//! `duckdb_bind_add_result_column` silently ignores a column whose type
//! contains `ANY` or `INVALID` anywhere (`TypeVisitor::Contains`), and
//! `duckdb_register_cast_function` returns a bare `DuckDBError` for the same
//! types — before it takes ownership of the cast's `extra_info`. Both are
//! checked here, in Rust, before the C API is called.

use libduckdb_sys::{
    duckdb_array_type_child_type, duckdb_get_type_id, duckdb_list_type_child_type,
    duckdb_logical_type, duckdb_map_type_key_type, duckdb_map_type_value_type,
    duckdb_struct_type_child_count, duckdb_struct_type_child_type, duckdb_union_type_member_count,
    duckdb_union_type_member_type, idx_t, DUCKDB_TYPE, DUCKDB_TYPE_DUCKDB_TYPE_ARRAY,
    DUCKDB_TYPE_DUCKDB_TYPE_INVALID, DUCKDB_TYPE_DUCKDB_TYPE_LIST, DUCKDB_TYPE_DUCKDB_TYPE_MAP,
    DUCKDB_TYPE_DUCKDB_TYPE_STRUCT, DUCKDB_TYPE_DUCKDB_TYPE_UNION,
};

use crate::types::LogicalType;

/// How deep a type may nest before the walks in this crate give up on it.
///
/// Every walk over a logical type here is recursive, and `DuckDB` itself puts
/// no limit on nesting: a type built in a loop from an untrusted schema (a
/// `LIST` of a `LIST` of ... 20,000 times) overflowed the thread's stack in
/// `type_string` with a process abort. Nothing real nests this deep.
///
/// This bounds quack-rs's own walks only. `DuckDB`'s `LogicalType::ToString`
/// recurses too, and overflowed a 2 MiB thread at 6,000 levels on 1.5.6 when
/// a catalog scan listed a function with such a parameter (a debug test
/// build, `hostile_input::a_deeply_nested_type_is_refused_not_recursed_into`
/// probing): an extension that builds types from untrusted input must bound
/// their depth itself.
pub const MAX_TYPE_NESTING: usize = 256;

/// `DUCKDB_TYPE_ANY`. Spelled as a literal because the constant is absent from
/// older `libduckdb-sys` bindings; its value (34) is fixed by `duckdb.h`. A
/// `DuckDB` that predates it reports `ANY` as `INVALID`, which is caught too.
const RAW_ANY: DUCKDB_TYPE = 34;

/// Refuses a scalar or aggregate return type that is, or contains, `ANY` or
/// `INVALID`.
///
/// `duckdb_register_scalar_function` and `duckdb_register_aggregate_function`
/// return a bare `DuckDBError` for such a return type (`TypeVisitor::Contains`
/// in `scalar_function-c.cpp` / `aggregate_function-c.cpp`), which surfaced as
/// a generic registration failure whose hint was about name collisions. `what`
/// names the slot.
///
/// # Errors
///
/// Returns an error naming `what` for such a type.
pub fn refuse_any_return(
    what: &str,
    id: Option<crate::types::TypeId>,
    logical: Option<&LogicalType>,
) -> Result<(), crate::error::ExtensionError> {
    let bad = match (logical, id) {
        // SAFETY: `lt` owns a live handle.
        (Some(lt), _) => unsafe { contains_any_or_invalid(lt.as_raw()) },
        (None, Some(id)) => id == crate::types::TypeId::Any,
        (None, None) => false,
    };
    if bad {
        return Err(crate::error::ExtensionError::new(format!(
            "{what} must not be or contain ANY or INVALID: DuckDB refuses to register a \
             function whose result type it cannot resolve"
        )));
    }
    Ok(())
}

/// Returns `true` if `ty`, or any type nested inside it, is `ANY` or `INVALID`.
///
/// # Safety
///
/// `ty` must be a live `duckdb_logical_type` handle.
pub unsafe fn contains_any_or_invalid(ty: duckdb_logical_type) -> bool {
    // SAFETY: forwarded from this function's own contract.
    unsafe {
        contains_type(ty, &|id| {
            id == DUCKDB_TYPE_DUCKDB_TYPE_INVALID || id == RAW_ANY
        })
    }
}

/// Returns `true` if `ty`, or any type nested inside it (list, array and map
/// children, struct fields, union members), has a type id `matches` accepts.
/// A child `DuckDB` cannot describe counts as a match, so a caller that uses
/// this to refuse something refuses it.
///
/// # Safety
///
/// `ty` must be a live `duckdb_logical_type` handle.
pub unsafe fn contains_type(
    ty: duckdb_logical_type,
    matches: &dyn Fn(DUCKDB_TYPE) -> bool,
) -> bool {
    // SAFETY: forwarded from this function's own contract.
    unsafe { contains_type_at(ty, matches, 0) }
}

/// [`contains_type`] at nesting level `depth`. A type nested more than
/// [`MAX_TYPE_NESTING`] deep counts as a match, like a child `DuckDB` cannot
/// describe: every caller uses the answer to refuse something.
///
/// # Safety
///
/// `ty` must be a live `duckdb_logical_type` handle.
unsafe fn contains_type_at(
    ty: duckdb_logical_type,
    matches: &dyn Fn(DUCKDB_TYPE) -> bool,
    depth: usize,
) -> bool {
    if depth > MAX_TYPE_NESTING {
        return true;
    }
    // SAFETY: `ty` is live per the caller's contract.
    let id = unsafe { duckdb_get_type_id(ty) };
    if matches(id) {
        return true;
    }
    if id == DUCKDB_TYPE_DUCKDB_TYPE_LIST {
        // SAFETY: `ty` is a live LIST type; the accessor hands us the child
        // handle and `child_matches` takes ownership of it.
        unsafe { child_matches(duckdb_list_type_child_type(ty), matches, depth) }
    } else if id == DUCKDB_TYPE_DUCKDB_TYPE_ARRAY {
        // SAFETY: `ty` is a live ARRAY type; the accessor hands us the child
        // handle and `child_matches` takes ownership of it.
        unsafe { child_matches(duckdb_array_type_child_type(ty), matches, depth) }
    } else if id == DUCKDB_TYPE_DUCKDB_TYPE_MAP {
        // SAFETY: `ty` is a live MAP type; each accessor hands us a child
        // handle and `child_matches` takes ownership of it.
        unsafe {
            child_matches(duckdb_map_type_key_type(ty), matches, depth)
                || child_matches(duckdb_map_type_value_type(ty), matches, depth)
        }
    } else if id == DUCKDB_TYPE_DUCKDB_TYPE_STRUCT {
        // SAFETY: `ty` is a live STRUCT type.
        let count: idx_t = unsafe { duckdb_struct_type_child_count(ty) };
        // SAFETY: `ty` is a live STRUCT type and `i < count`; `child_matches`
        // takes ownership of the child handle the accessor returns.
        (0..count)
            .any(|i| unsafe { child_matches(duckdb_struct_type_child_type(ty, i), matches, depth) })
    } else if id == DUCKDB_TYPE_DUCKDB_TYPE_UNION {
        // SAFETY: `ty` is a live UNION type.
        let count: idx_t = unsafe { duckdb_union_type_member_count(ty) };
        // SAFETY: `ty` is a live UNION type and `i < count`; `child_matches`
        // takes ownership of the member handle the accessor returns.
        (0..count)
            .any(|i| unsafe { child_matches(duckdb_union_type_member_type(ty, i), matches, depth) })
    } else {
        false
    }
}

/// Takes ownership of a child handle returned by a `duckdb_*_child_type`
/// accessor and checks it. A null handle means `DuckDB` could not describe the
/// child, which counts as a match rather than being silently passed over.
///
/// # Safety
///
/// `raw` must be null, or a live logical type handle the caller owns and does
/// not use again: it is inspected through the C API and destroyed here.
unsafe fn child_matches(
    raw: duckdb_logical_type,
    matches: &dyn Fn(DUCKDB_TYPE) -> bool,
    depth: usize,
) -> bool {
    if raw.is_null() {
        return true;
    }
    // SAFETY: `raw` is a non-null handle the accessor handed to us to own;
    // `LogicalType` destroys it when it drops at the end of this function.
    let child = unsafe { LogicalType::from_raw(raw) };
    // SAFETY: `child` is live for the duration of the call.
    unsafe { contains_type_at(child.as_raw(), matches, depth + 1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_duckdb_could_not_describe_is_bad() {
        // SAFETY: null is explicitly allowed and makes no DuckDB call.
        assert!(unsafe { child_matches(std::ptr::null_mut(), &|_| false, 0) });
    }

    /// A return type given only as a `TypeId` is refused exactly when it is
    /// `ANY`, with a message naming the slot; no type at all is not refused.
    #[test]
    fn an_any_return_type_id_is_refused_by_name() {
        use crate::types::TypeId;
        let err =
            refuse_any_return("return type", Some(TypeId::Any), None).expect_err("ANY is refused");
        assert!(
            err.as_str()
                .starts_with("return type must not be or contain ANY"),
            "{err}"
        );
        assert!(refuse_any_return("return type", Some(TypeId::Integer), None).is_ok());
        assert!(refuse_any_return("return type", None, None).is_ok());
    }
}
