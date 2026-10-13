// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Table function name collisions.
//!
//! `DuckDB`'s C API has no table function *sets*, and the system catalog keys
//! table functions by name rather than by signature, so a second registration
//! under a name that already belongs to a table function or table macro never
//! becomes an overload: the existing entry keeps answering. What the C API
//! reports about it differs by release — v1.5.x asks for `ALTER_ON_CONFLICT`,
//! which keeps the existing entry and returns success, dropping the new
//! function silently; v1.4.x leaves the default `ERROR_ON_CONFLICT`, so the
//! catalog throws and the C API turns that into a failure it cannot explain
//! (`DuckSchemaEntry::AddEntryInternal`,
//! `src/catalog/catalog_entry/duck_schema_entry.cpp`). Neither names the
//! conflict, so [`TableFunctionBuilder::register`] checks the catalog first.
//!
//! [`TableFunctionBuilder::register`]: crate::table::TableFunctionBuilder::register
//!
//! Listing the catalog is a full scan of `duckdb_functions()` — tens of
//! milliseconds on `DuckDB` 1.5.x, hardware-dependent — and an extension with
//! many table functions paid that once per function, over a second per `LOAD`.
//! [`ExistingTableFunctions`] exists to pay it once per extension load instead,
//! the way
//! [`ExistingScalars`][crate::scalar::builder::collision::ExistingScalars]
//! does for scalars: the first check through a
//! [`Connection`][crate::connection::Connection] lists every table function and
//! table macro name into a snapshot, and later checks are a set lookup.
//!
//! Like the scalar snapshot, that one is a one-time copy: a table function
//! registered on the raw connection after it was listed is not in it, and a
//! later registration under that name passes the check. See the
//! [`Connection`][crate::connection::Connection] field doc.

use core::cell::RefCell;
use std::collections::HashSet;

use libduckdb_sys::duckdb_connection;

use crate::error::ExtensionError;

/// Every table function and table macro name in the `system.main` catalog, as
/// `duckdb_functions()` lists them.
///
/// Names are stored lower-cased: `DuckDB` resolves function names
/// case-insensitively, so `MyFunc` and `myfunc` are the same function.
#[derive(Debug, Default)]
pub struct ExistingTableFunctions {
    by_name: HashSet<String>,
}

impl ExistingTableFunctions {
    /// Lists the catalog's table function and table macro names — all of them —
    /// in one scan of `duckdb_functions()`.
    ///
    /// Every function the query reads is qualified with `system.main`, so a
    /// user macro of the same name (`CREATE MACRO duckdb_functions() AS …`)
    /// cannot change what it lists.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open connection.
    pub unsafe fn load(con: duckdb_connection) -> Result<Self, ExtensionError> {
        let context = |detail: String| {
            ExtensionError::new(format!("cannot check whether the name is taken: {detail}"))
        };
        let sql = "SELECT function_name FROM system.main.duckdb_functions() \
                   WHERE database_name = 'system' AND schema_name = 'main' \
                     AND function_type IN ('table', 'table_macro')";
        // SAFETY: `con` is valid per this function's contract.
        let statement =
            unsafe { crate::query::prepare(con, sql) }.map_err(|e| context(e.to_string()))?;
        let mut result = statement.execute().map_err(|e| context(e.to_string()))?;
        let mut existing = Self::default();
        while let Some(chunk) = result.next_chunk().map_err(|e| context(e.to_string()))? {
            if chunk.column_count() != 1 {
                return Err(context(
                    "the catalog query returned an unexpected shape".into(),
                ));
            }
            // SAFETY: one VARCHAR column; each row's validity is checked before
            // its string is read, and the chunk outlives the reader.
            unsafe {
                let names = chunk.reader(0);
                for row in 0..chunk.size() {
                    if names.is_valid(row) {
                        existing.record(names.read_str(row));
                    }
                }
            }
        }
        Ok(existing)
    }

    /// Adds `name`, as a successful registration does.
    pub fn record(&mut self, name: &str) {
        self.by_name.insert(name.to_lowercase());
    }

    /// Refuses `name` if the catalog already holds a table function or table
    /// macro with it.
    pub fn check(&self, name: &str) -> Result<(), ExtensionError> {
        if !self.by_name.contains(&name.to_lowercase()) {
            return Ok(());
        }
        Err(ExtensionError::new(format!(
            "table function '{name}' already exists (a built-in, another extension's, or an \
             earlier registration). DuckDB's C API cannot add overloads to an existing table \
             function — the catalog keys them by name — so this registration would not take \
             effect: v1.5.x drops it and reports success, v1.4.x fails the call, and the \
             existing function keeps answering either way. Choose a different name."
        )))
    }
}

/// Refuses `name` if the system catalog already holds a table function or table
/// macro with that name (case-insensitively).
///
/// With a `snapshot`, the catalog is listed into it once and reused; without
/// one, it is listed now.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
pub unsafe fn refuse_taken_table_function_name(
    con: duckdb_connection,
    snapshot: Option<&RefCell<Option<ExistingTableFunctions>>>,
    name: &str,
) -> Result<(), ExtensionError> {
    // A listing failure is not this name's fault, but the error reads better
    // saying which registration it interrupted.
    let named = |e: ExtensionError| ExtensionError::new(format!("table function '{name}': {e}"));
    match snapshot {
        Some(cell) => {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                // SAFETY: `con` is valid per this function's contract.
                let listed = unsafe { ExistingTableFunctions::load(con) }.map_err(named)?;
                *slot = Some(listed);
            }
            slot.as_ref()
                .map_or(Ok(()), |existing| existing.check(name))
        }
        // SAFETY: as above.
        None => unsafe { ExistingTableFunctions::load(con) }
            .map_err(named)?
            .check(name),
    }
}

/// Records a successful registration in `snapshot`, if it has been loaded, so
/// later checks through the same snapshot see it.
///
/// A snapshot that has not been loaded needs no update: loading it later reads
/// the catalog, which already holds this registration.
pub fn record_registered(snapshot: Option<&RefCell<Option<ExistingTableFunctions>>>, name: &str) {
    if let Some(cell) = snapshot {
        if let Some(existing) = cell.borrow_mut().as_mut() {
            existing.record(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{record_registered, refuse_taken_table_function_name, ExistingTableFunctions};

    /// A loaded snapshot is used as-is: the check passes with a null
    /// connection, which it can only do if the catalog is never listed — and
    /// it does have to list the catalog to load a snapshot. This is what makes
    /// one catalog scan per extension load rather than one per function.
    #[test]
    fn a_loaded_snapshot_is_used_without_touching_the_connection() {
        let mut existing = ExistingTableFunctions::default();
        existing.record("taken");
        let cell = RefCell::new(Some(existing));
        // SAFETY: the contract asks for a valid connection, and this passes a
        // null one precisely because a loaded snapshot must not use it. If the
        // check ever lists the catalog here, this test crashes instead of
        // quietly costing a scan per function.
        unsafe {
            assert!(
                refuse_taken_table_function_name(core::ptr::null_mut(), Some(&cell), "fresh")
                    .is_ok()
            );
            assert!(
                refuse_taken_table_function_name(core::ptr::null_mut(), Some(&cell), "TAKEN")
                    .is_err()
            );
        }
    }

    /// A recorded name is refused, a fresh one is accepted, and names match
    /// case-insensitively, as `DuckDB`'s catalog does.
    #[test]
    fn check_tells_a_taken_name_from_a_fresh_one() {
        let mut existing = ExistingTableFunctions::default();
        existing.record("MyFunc");

        let err = existing.check("myfunc").expect_err("same name, other case");
        assert!(
            err.as_str()
                .contains("table function 'myfunc' already exists"),
            "{err}"
        );
        assert!(existing.check("another").is_ok());
    }

    /// A loaded snapshot learns each registered name; an unloaded one stays
    /// unloaded, so the next check lists the catalog itself.
    #[test]
    fn record_registered_updates_only_a_loaded_snapshot() {
        let loaded = RefCell::new(Some(ExistingTableFunctions::default()));
        record_registered(Some(&loaded), "greet");
        assert!(loaded
            .borrow()
            .as_ref()
            .expect("still loaded")
            .check("greet")
            .is_err());

        let unloaded = RefCell::new(None);
        record_registered(Some(&unloaded), "greet");
        assert!(unloaded.borrow().is_none());
        record_registered(None, "greet");
    }
}
