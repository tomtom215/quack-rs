// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Saying why an aggregate registration failed when the name was taken.
//!
//! An aggregate can neither extend nor replace an existing catalog entry:
//! from v1.5.0 `duckdb_register_aggregate_function(_set)` registers with
//! `ALTER_ON_CONFLICT`, and `CreateAggregateFunctionInfo` has no
//! `GetAlterInfo`, so `CreateInfo::GetAlterInfo` throws for **any** existing
//! scalar function, aggregate or macro of that name; before v1.5.0 it
//! registers with `CREATE` (`ERROR_ON_CONFLICT`). Either way the C API returns
//! a bare `DuckDBError` (reviewer `ffi`, FFI-8, read at v1.5.6 and v2.0.0).
//!
//! Unlike a scalar, whose collision is a silent replacement and so has to be
//! caught before registering (`scalar::builder::collision`), an aggregate's
//! collision is a failure. So the catalog is consulted only after one, to say
//! which entry holds the name: no scan is paid on the success path, and
//! nothing about the registration changes.

use libduckdb_sys::duckdb_connection;

/// A `duckdb_functions()` function type in the words of the error message,
/// with its article.
fn kind(function_type: &str) -> String {
    match function_type {
        "scalar" => "a scalar function".to_owned(),
        "aggregate" => "an aggregate function".to_owned(),
        "macro" => "a macro".to_owned(),
        other => format!("a {other}"),
    }
}

/// The reason a failed registration of the aggregate `name` failed for, if
/// the catalog shows a scalar function, aggregate or macro holding the name
/// in `system.main` (where the C API registers): a sentence for the error
/// message. `None` when it shows none, or cannot be read (the caller then
/// falls back to the generic hint).
///
/// # Safety
///
/// `con` must be a valid, open connection.
pub(super) unsafe fn taken_name_reason(con: duckdb_connection, name: &str) -> Option<String> {
    let sql = "SELECT function_name, function_type \
               FROM system.main.duckdb_functions() \
               WHERE database_name = 'system' AND schema_name = 'main' \
                 AND function_type IN ('scalar', 'aggregate', 'macro') \
                 AND system.main.lower(function_name) = system.main.lower($1) \
               ORDER BY function_type LIMIT 1";
    // SAFETY: `con` is valid per this function's contract.
    let statement = unsafe { crate::query::prepare(con, sql) }.ok()?;
    statement.bind_str(1, name).ok()?;
    let mut result = statement.execute().ok()?;
    let chunk = result.next_chunk().ok()??;
    if chunk.size() == 0 || chunk.column_count() != 2 {
        return None;
    }
    // SAFETY: two VARCHAR columns and row 0 exists; each value's validity is
    // checked before it is read, and the chunk outlives the readers.
    let (existing, function_type) = unsafe {
        let (names, types) = (chunk.reader(0), chunk.reader(1));
        if !names.is_valid(0) || !types.is_valid(0) {
            return None;
        }
        (names.read_str(0).to_owned(), types.read_str(0).to_owned())
    };
    // `internal` cannot tell a built-in from an extension's function: the C
    // API registers into the system catalog as internal too.
    Some(format!(
        "{kind} named '{existing}' already exists (a built-in, another extension's, or an \
         earlier registration of this one, e.g. by a LOAD that failed part-way or ran twice), \
         and DuckDB cannot register an \
         aggregate under a taken name: from v1.5.0 it would have to alter the existing entry, \
         which it does not implement for aggregates, and before v1.5.0 it refuses any existing \
         name. Choose a different name; to give one aggregate several signatures, register \
         them all in one AggregateFunctionSetBuilder",
        kind = kind(&function_type)
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn function_types_read_as_words() {
        assert_eq!(super::kind("scalar"), "a scalar function");
        assert_eq!(super::kind("aggregate"), "an aggregate function");
        assert_eq!(super::kind("macro"), "a macro");
        assert_eq!(super::kind("pragma"), "a pragma");
    }
}
