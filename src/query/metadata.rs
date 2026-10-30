// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! What a statement is before it runs: its [`StatementType`], parameter
//! types and result columns, and the tables a query reads.

use libduckdb_sys::{
    duckdb_param_logical_type, duckdb_prepare_error, duckdb_prepared_statement_type,
    duckdb_result_statement_type, duckdb_statement_type, idx_t,
};

use super::{PreparedStatement, QueryResult};
use crate::types::LogicalType;

/// What kind of SQL statement a [`PreparedStatement`] or [`QueryResult`] is,
/// mirroring `duckdb_statement_type`.
///
/// `DuckDB` 1.5.5 added `COPY DATABASE`, `UPDATE EXTENSIONS` and
/// `MERGE INTO` to the C enum; an older engine reports those three
/// statements as [`Invalid`][Self::Invalid] (its `StatementTypeToC` has no
/// case for them). A value from a newer `DuckDB` than this build knows is
/// [`Other`][Self::Other].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StatementType {
    /// `DUCKDB_STATEMENT_TYPE_INVALID`: no statement, a failed prepare, or a
    /// statement the engine's C API has no value for.
    Invalid,
    /// `SELECT` (and `VALUES`, `FROM t`, `WITH ...`).
    Select,
    /// `INSERT`.
    Insert,
    /// `UPDATE`.
    Update,
    /// `EXPLAIN`.
    Explain,
    /// `DELETE`.
    Delete,
    /// `PREPARE`.
    Prepare,
    /// `CREATE` (table, view, schema, sequence, type, …).
    Create,
    /// `EXECUTE`.
    Execute,
    /// `ALTER`.
    Alter,
    /// `BEGIN`, `COMMIT`, `ROLLBACK`.
    Transaction,
    /// `COPY`.
    Copy,
    /// `ANALYZE`.
    Analyze,
    /// `SET VARIABLE`.
    VariableSet,
    /// `CREATE FUNCTION` / `CREATE MACRO`.
    CreateFunc,
    /// `DROP`.
    Drop,
    /// `EXPORT DATABASE`.
    Export,
    /// `PRAGMA`.
    Pragma,
    /// `VACUUM`.
    Vacuum,
    /// `CALL`.
    Call,
    /// `SET` / `RESET`.
    Set,
    /// `LOAD`.
    Load,
    /// A relation built through the C++ relational API.
    Relation,
    /// A statement an extension's parser added.
    Extension,
    /// A logical plan.
    LogicalPlan,
    /// `ATTACH`.
    Attach,
    /// `DETACH`.
    Detach,
    /// Several statements in one.
    Multi,
    /// `COPY FROM DATABASE` (`DuckDB` 1.5.5+).
    CopyDatabase,
    /// `UPDATE EXTENSIONS` (`DuckDB` 1.5.5+).
    UpdateExtensions,
    /// `MERGE INTO` (`DuckDB` 1.5.5+).
    MergeInto,
    /// A `duckdb_statement_type` this build of quack-rs does not know.
    Other(u32),
}

// The three values `duckdb.h` added in 1.5.5, spelled as literals so the
// crate does not depend on the bindings declaring them (the 1.4.4 bindings do
// not); the values are part of the C ABI.
const COPY_DATABASE: duckdb_statement_type = 28;
const UPDATE_EXTENSIONS: duckdb_statement_type = 29;
const MERGE_INTO: duckdb_statement_type = 30;

impl StatementType {
    /// Converts from the C API value.
    #[must_use]
    pub(crate) const fn from_raw(raw: duckdb_statement_type) -> Self {
        use libduckdb_sys as sys;
        match raw {
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_INVALID => Self::Invalid,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_SELECT => Self::Select,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_INSERT => Self::Insert,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_UPDATE => Self::Update,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_EXPLAIN => Self::Explain,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_DELETE => Self::Delete,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_PREPARE => Self::Prepare,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_CREATE => Self::Create,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_EXECUTE => Self::Execute,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_ALTER => Self::Alter,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_TRANSACTION => Self::Transaction,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_COPY => Self::Copy,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_ANALYZE => Self::Analyze,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_VARIABLE_SET => Self::VariableSet,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_CREATE_FUNC => Self::CreateFunc,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_DROP => Self::Drop,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_EXPORT => Self::Export,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_PRAGMA => Self::Pragma,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_VACUUM => Self::Vacuum,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_CALL => Self::Call,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_SET => Self::Set,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_LOAD => Self::Load,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_RELATION => Self::Relation,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_EXTENSION => Self::Extension,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_LOGICAL_PLAN => Self::LogicalPlan,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_ATTACH => Self::Attach,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_DETACH => Self::Detach,
            sys::duckdb_statement_type_DUCKDB_STATEMENT_TYPE_MULTI => Self::Multi,
            COPY_DATABASE => Self::CopyDatabase,
            UPDATE_EXTENSIONS => Self::UpdateExtensions,
            MERGE_INTO => Self::MergeInto,
            other => Self::Other(other),
        }
    }
}

impl PreparedStatement {
    /// What kind of statement this is.
    ///
    /// Returns [`StatementType::Invalid`] for a statement whose prepare
    /// failed (only reachable through
    /// [`from_raw`][Self::from_raw]): on 1.x `duckdb_prepared_statement_type`
    /// dereferences the statement data a failed prepare does not have, so it
    /// is not called for one.
    ///
    /// Works on every supported `DuckDB` (stable C API slot 54).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::query::{OwnedConnection, StatementType};
    ///
    /// # fn demo(con: &OwnedConnection, sql: &str) -> Result<(), quack_rs::error::ExtensionError> {
    /// let stmt = con.prepare(sql)?;
    /// if stmt.statement_type() != StatementType::Select {
    ///     return Err("only SELECT statements are accepted".into());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    // Only reachable with a live statement: FFI-only, covered by
    // tests/ffi_roundtrip/query_metadata.rs (including the failed-prepare guard).
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn statement_type(&self) -> StatementType {
        // SAFETY: `self.statement` is valid for this value's lifetime;
        // `duckdb_prepare_error` returns null for a successful prepare and a
        // borrowed message otherwise, and reads nothing else.
        if !unsafe { duckdb_prepare_error(self.statement) }.is_null() {
            return StatementType::Invalid;
        }
        // SAFETY: a successfully prepared statement has the data
        // `GetStatementType` reads.
        StatementType::from_raw(unsafe { duckdb_prepared_statement_type(self.statement) })
    }

    /// The type `DuckDB` inferred for the parameter at 1-based `index`, or
    /// `None` when `index` is out of range or the binder could not infer a
    /// type without a bound value (`SELECT $1`, `b || $1`).
    ///
    /// Once the statement has executed, `DuckDB` reports the type of the
    /// value last bound for a parameter whose type it could not infer.
    ///
    /// Works on every supported `DuckDB` (stable C API slot 52).
    // Only reachable with a live statement or connection: FFI-only, covered by
    // tests/ffi_roundtrip/query_metadata.rs.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn parameter_type(&self, index: usize) -> Option<LogicalType> {
        if index == 0 || index > self.parameter_count() {
            return None;
        }
        // SAFETY: `self.statement` is valid and `index` is in range; DuckDB
        // returns a new type the caller owns, or null.
        let raw = unsafe { duckdb_param_logical_type(self.statement, index as idx_t) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is a non-null owned handle; `LogicalType` destroys it.
        known(unsafe { LogicalType::from_raw(raw) })
    }
}

/// `ty`, unless its id is `DUCKDB_TYPE_INVALID`, which is how `DuckDB`
/// reports a type it has not resolved (`LogicalType::UNKNOWN` and `INVALID`
/// both convert to it). A type id this build does not name (a `GEOMETRY`
/// without `duckdb-1-5-3`, a type from a newer `DuckDB`) is still a type.
// Only reachable with a live statement or connection: FFI-only, covered by
// tests/ffi_roundtrip/query_metadata.rs.
#[cfg_attr(test, mutants::skip)]
fn known(ty: LogicalType) -> Option<LogicalType> {
    // SAFETY: `ty` is a live handle (the C API is initialised: it came from it).
    let id = unsafe { libduckdb_sys::duckdb_get_type_id(ty.as_raw()) };
    (id != libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_INVALID).then_some(ty)
}

#[cfg(feature = "duckdb-1-5")]
impl PreparedStatement {
    /// The name and type of each column the statement returns, before it
    /// runs (`DuckDB` 1.5.0+).
    ///
    /// Returns `None` when the result shape is not known until parameters
    /// are bound: `DuckDB` then reports a single placeholder column named
    /// `unknown` of an unresolved type (`SELECT $1`, `SELECT b || $1 FROM t`),
    /// which is not the shape the statement returns. Also `None` for a
    /// statement whose prepare failed. A statement that changes rows reports
    /// the column its result has: an `INSERT` reports one `BIGINT` column,
    /// `Count`.
    ///
    /// Requires `duckdb-1-5`: the four `duckdb_prepared_statement_column_*`
    /// calls sit in the unstable region of the C API struct.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # fn demo(con: &quack_rs::query::OwnedConnection) -> Result<(), quack_rs::error::ExtensionError> {
    /// let stmt = con.prepare("SELECT 42 AS answer, 'x' AS label")?;
    /// for (name, ty) in stmt.columns().unwrap_or_default() {
    ///     // SAFETY: the C API is initialised (a statement exists).
    ///     println!("{name}: {:?}", unsafe { ty.try_get_type_id() });
    /// }
    /// # Ok(())
    /// # }
    /// ```
    // Only reachable with a live statement or connection: FFI-only, covered by
    // tests/ffi_roundtrip/query_metadata.rs.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn columns(&self) -> Option<Vec<(String, LogicalType)>> {
        // SAFETY: `self.statement` is valid; null means the prepare succeeded.
        if !unsafe { duckdb_prepare_error(self.statement) }.is_null() {
            return None;
        }
        // SAFETY: `self.statement` is a valid, successfully prepared statement.
        let count =
            unsafe { libduckdb_sys::duckdb_prepared_statement_column_count(self.statement) };
        (0..count)
            .map(|i| {
                // SAFETY: `i < count`. DuckDB `strdup`s the name despite the
                // `const char *` return (`prepared-c.cpp`, 1.4.4 to 2.0.0),
                // so it is freed below; it returns null on a failed prepare.
                let ptr = unsafe {
                    libduckdb_sys::duckdb_prepared_statement_column_name(self.statement, i)
                };
                if ptr.is_null() {
                    return None;
                }
                // SAFETY: `ptr` is NUL-terminated.
                let name = unsafe { super::cstr::c_str_to_owned(ptr) };
                // SAFETY: `strdup`ed by DuckDB, freed exactly once here.
                unsafe { libduckdb_sys::duckdb_free(ptr.cast_mut().cast()) };
                // SAFETY: `i < count`; DuckDB returns a new type the caller
                // owns, or null.
                let raw = unsafe {
                    libduckdb_sys::duckdb_prepared_statement_column_logical_type(self.statement, i)
                };
                if raw.is_null() {
                    return None;
                }
                // SAFETY: `raw` is a non-null owned handle.
                let ty = known(unsafe { LogicalType::from_raw(raw) })?;
                Some((name?, ty))
            })
            .collect()
    }
}

impl QueryResult {
    /// What kind of statement produced this result: for a string of several
    /// statements, the one whose result this is (see
    /// [`query`][super::query]).
    ///
    /// Works on every supported `DuckDB` (stable C API slot 17).
    #[must_use]
    pub fn statement_type(&self) -> StatementType {
        // SAFETY: `duckdb_result_statement_type` takes the result by value
        // and only reads it; it returns INVALID for a result with an error.
        StatementType::from_raw(unsafe { duckdb_result_statement_type(self.result) })
    }
}

/// The tables `sql` reads or writes, as `DuckDB`'s binder resolves them
/// (`DuckDB` 1.5.0+).
///
/// With `qualified`, each name carries its catalog and schema as written —
/// `main.b` for `FROM main.b`, `a` for `FROM a`; without, only the table
/// name. Names come back in no particular order. A table function call or
/// a CTE is not a table.
///
/// `sql` must be exactly one statement. `DuckDB` parses it and binds it in a
/// names-only mode (`BindingMode::EXTRACT_NAMES`, `ClientContext::GetTableNames`),
/// so nothing runs and the tables need not exist — but a statement whose
/// binding needs a table's columns, such as a `JOIN … USING`, fails. It
/// runs on `con` like a statement and needs it idle: on `DuckDB` 2.0 a
/// streaming result open on `con` makes it fail, and before 2.0 it ends
/// that stream.
///
/// # Errors
///
/// - `sql` contains an interior NUL byte;
/// - `DuckDB` could not list the tables: the SQL does not parse, holds
///   more than one statement, or binding it in that mode threw. `DuckDB`
///   gives no reason (`duckdb_get_table_names` returns null from a
///   `catch (...)`).
///
/// Requires `duckdb-1-5`: `duckdb_get_table_names` sits in the unstable
/// region of the C API struct.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
// Only reachable with a live statement or connection: FFI-only, covered by
// tests/ffi_roundtrip/query_metadata.rs.
#[cfg_attr(test, mutants::skip)]
#[cfg(feature = "duckdb-1-5")]
pub unsafe fn table_names(
    con: libduckdb_sys::duckdb_connection,
    sql: &str,
    qualified: bool,
) -> Result<Vec<String>, crate::error::ExtensionError> {
    let c_sql = super::cstr::to_c_sql(sql)?;
    // SAFETY: `con` is valid per the caller's contract and `c_sql` outlives
    // the call; DuckDB catches every exception and returns null, or an owned
    // `LIST(VARCHAR)` value.
    let raw = unsafe { libduckdb_sys::duckdb_get_table_names(con, c_sql.as_ptr(), qualified) };
    if raw.is_null() {
        return Err(crate::error::ExtensionError::new(
            "duckdb_get_table_names failed: the SQL does not parse or bind (DuckDB reports no \
             reason; prepare the statement to see the error)",
        ));
    }
    // SAFETY: `raw` is a non-null owned value; `Value` destroys it.
    let list = unsafe { crate::value::Value::from_raw(raw) };
    list.list_items()
        .iter()
        .map(|name| {
            name.as_str()
                .map_err(|e| crate::error::ExtensionError::new(format!("table name: {e}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_statement_type_value_maps_to_its_variant() {
        let named = [
            StatementType::Invalid,
            StatementType::Select,
            StatementType::Insert,
            StatementType::Update,
            StatementType::Explain,
            StatementType::Delete,
            StatementType::Prepare,
            StatementType::Create,
            StatementType::Execute,
            StatementType::Alter,
            StatementType::Transaction,
            StatementType::Copy,
            StatementType::Analyze,
            StatementType::VariableSet,
            StatementType::CreateFunc,
            StatementType::Drop,
            StatementType::Export,
            StatementType::Pragma,
            StatementType::Vacuum,
            StatementType::Call,
            StatementType::Set,
            StatementType::Load,
            StatementType::Relation,
            StatementType::Extension,
            StatementType::LogicalPlan,
            StatementType::Attach,
            StatementType::Detach,
            StatementType::Multi,
            StatementType::CopyDatabase,
            StatementType::UpdateExtensions,
            StatementType::MergeInto,
        ];
        // `duckdb.h` numbers them 0..=30 in this order.
        for (raw, expected) in (0_u32..).zip(named) {
            assert_eq!(StatementType::from_raw(raw), expected, "value {raw}");
        }
        assert_eq!(StatementType::from_raw(31), StatementType::Other(31));
        assert_eq!(
            StatementType::from_raw(u32::MAX),
            StatementType::Other(u32::MAX)
        );
    }
}
