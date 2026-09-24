// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Refusing a scalar registration that would silently replace an existing one,
//! or make calls to it ambiguous.
//!
//! `duckdb_register_scalar_function` adds to the catalog with
//! `ALTER_ON_CONFLICT`. When the name already holds a scalar function the new
//! overloads are merged in with `override = true`
//! (`ScalarFunctionCatalogEntry` → `MergeFunctionSet`), so an overload whose
//! parameter types match an existing one **replaces it for every connection**
//! — a built-in included — and the call still reports success. Registering
//! `abs(BIGINT)` from an extension changes what every `abs(x::BIGINT)` in the
//! database returns. An overload that does not match exactly but accepts the
//! same argument list at some arity (`f(BIGINT)` beside `f(BIGINT,
//! BIGINT...)`) is added, and every such call then fails with "Could not
//! choose a best candidate function".
//!
//! Before registering, the scalar builders render each overload's parameter
//! and varargs types the way `DuckDB` prints them in `duckdb_functions()` —
//! aliases included — and refuse an overload that [`shapes_overlap`] one
//! already listed. Rendering covers every type except `STRUCT`, `UNION`, `ENUM`
//! and the literal pseudo-types; an overload using one of those is not checked
//! (its rendering in `duckdb_functions()` quotes names in ways not worth
//! reproducing), which is documented on the builders.
//! `tests/ffi_roundtrip/lifecycle.rs` holds every rendering to the string the
//! linked `DuckDB` reports.
//!
//! # Cost
//!
//! Listing the catalog is one scan of `duckdb_functions()`, about 13 ms in a
//! release build of `DuckDB` 1.5.5 whatever the filter. A builder's own
//! `register` makes one scan per call, for all its overloads. Registration
//! through the entry point's [`Connection`](crate::connection::Connection)
//! lists the catalog once and then keeps that snapshot up to date with its own
//! registrations, so an extension registering hundreds of functions pays for
//! one scan, not hundreds.

use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_array_type_child_type, duckdb_connection,
    duckdb_decimal_scale, duckdb_decimal_width, duckdb_get_type_id, duckdb_list_type_child_type,
    duckdb_logical_type, duckdb_map_type_key_type, duckdb_map_type_value_type,
    DUCKDB_TYPE_DUCKDB_TYPE_ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL, DUCKDB_TYPE_DUCKDB_TYPE_ENUM,
    DUCKDB_TYPE_DUCKDB_TYPE_LIST, DUCKDB_TYPE_DUCKDB_TYPE_MAP, DUCKDB_TYPE_DUCKDB_TYPE_STRUCT,
    DUCKDB_TYPE_DUCKDB_TYPE_UNION,
};

use std::cell::RefCell;
use std::collections::HashMap;

use super::signature::{shapes_overlap, ParamRef, Shape};
use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};

/// How `DuckDB` prints a non-composite type in `duckdb_functions()`, or `None`
/// for a type this module does not render.
///
/// `LogicalType::ToString` spells two of them differently from
/// [`TypeId::sql_name`]: `TIMESTAMPTZ` and `TIMETZ` print in full.
pub const fn id_string(id: TypeId) -> Option<&'static str> {
    match id {
        TypeId::TimestampTz => Some("TIMESTAMP WITH TIME ZONE"),
        TypeId::TimeTz => Some("TIME WITH TIME ZONE"),
        TypeId::SqlNull | TypeId::IntegerLiteral | TypeId::StringLiteral => None,
        id if id.is_composite() => None,
        id => Some(id.sql_name()),
    }
}

/// How `DuckDB` prints `ty` in `duckdb_functions()`, or `None` if it contains
/// a type [`id_string`] does not render (or a `STRUCT`, `UNION` or `ENUM`).
///
/// # Safety
///
/// `ty` must be a live logical type handle.
unsafe fn type_string(ty: duckdb_logical_type) -> Option<String> {
    // SAFETY: `ty` is live per this function's contract; every child handle
    // fetched below is owned by the `LogicalType` that wraps it.
    unsafe {
        let child = |raw: duckdb_logical_type| {
            if raw.is_null() {
                return None;
            }
            let owned = LogicalType::from_raw(raw);
            type_string(owned.as_raw())
        };
        // `LogicalType::ToString` prints an aliased type by its alias, at any
        // depth (`CREATE TYPE myint AS INTEGER` lists as `myint`, and a list
        // of it as `myint[]`), and `ScalarFunction::Equal` compares the alias
        // too: `abs(myint)` is a different overload from `abs(INTEGER)`.
        let alias = libduckdb_sys::duckdb_logical_type_get_alias(ty);
        if !alias.is_null() {
            let text = std::ffi::CStr::from_ptr(alias)
                .to_string_lossy()
                .into_owned();
            libduckdb_sys::duckdb_free(alias.cast());
            return Some(text);
        }
        match duckdb_get_type_id(ty) {
            DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL => Some(format!(
                "DECIMAL({},{})",
                duckdb_decimal_width(ty),
                duckdb_decimal_scale(ty)
            )),
            DUCKDB_TYPE_DUCKDB_TYPE_LIST => {
                Some(format!("{}[]", child(duckdb_list_type_child_type(ty))?))
            }
            DUCKDB_TYPE_DUCKDB_TYPE_ARRAY => Some(format!(
                "{}[{}]",
                child(duckdb_array_type_child_type(ty))?,
                duckdb_array_type_array_size(ty)
            )),
            DUCKDB_TYPE_DUCKDB_TYPE_MAP => Some(format!(
                "MAP({}, {})",
                child(duckdb_map_type_key_type(ty))?,
                child(duckdb_map_type_value_type(ty))?
            )),
            DUCKDB_TYPE_DUCKDB_TYPE_STRUCT
            | DUCKDB_TYPE_DUCKDB_TYPE_UNION
            | DUCKDB_TYPE_DUCKDB_TYPE_ENUM => None,
            raw => id_string(TypeId::try_from_duckdb_type(raw)?).map(str::to_owned),
        }
    }
}

/// A scalar signature as `duckdb_functions()` renders it: its fixed
/// parameter types and its varargs type, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The fixed parameter types, in order.
    pub fixed: Vec<String>,
    /// The varargs type, if the overload takes varargs.
    pub varargs: Option<String>,
}

impl Rendered {
    /// This signature as a [`Shape`] for [`shapes_overlap`].
    fn shape(&self) -> Shape<'_> {
        Shape {
            fixed: &self.fixed,
            varargs: self.varargs.as_deref(),
        }
    }
}

/// Renders one overload's signature, or `None` if any of its types is not
/// rendered.
///
/// # Safety
///
/// Every logical parameter must be a live handle.
pub unsafe fn render_signature(params: &[ParamRef<'_>]) -> Option<Rendered> {
    let mut fixed = Vec::with_capacity(params.len());
    let mut varargs = None;
    for param in params {
        match param {
            ParamRef::Id(id) => fixed.push(id_string(*id)?.to_owned()),
            // SAFETY: live per this function's contract.
            ParamRef::Logical(lt) => fixed.push(unsafe { type_string(lt.as_raw()) }?),
            // SAFETY: as above.
            ParamRef::Varargs(lt) => varargs = Some(unsafe { type_string(lt.as_raw()) }?),
            ParamRef::VarargsId(id) => varargs = Some(id_string(*id)?.to_owned()),
        }
    }
    Some(Rendered { fixed, varargs })
}

/// Separates rendered parameter types; `DuckDB` type names never contain it.
const SEPARATOR: char = '\u{1f}';

/// Every scalar function signature in the `system.main` catalog, by
/// lower-cased name, as `duckdb_functions()` lists them.
#[derive(Debug, Default)]
pub struct ExistingScalars {
    by_name: HashMap<String, Vec<Rendered>>,
    /// The engine cannot add an overload to an existing name: before v1.5.0,
    /// `duckdb_register_scalar_function` creates the function without
    /// `OnCreateConflict::ALTER_ON_CONFLICT` (checked in the source at v1.4.4
    /// and v1.5.0), so any name already in the catalog fails with a bare
    /// `DuckDBError`.
    refuses_existing_names: bool,
}

impl ExistingScalars {
    /// Lists the catalog's scalar signatures — all of them, or only those of
    /// `name` — in one scan of `duckdb_functions()`.
    ///
    /// Every function the query calls is qualified with `system.main`, so a
    /// user macro of the same name (`CREATE MACRO lower(x) AS …`) cannot
    /// change what it computes.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open connection.
    pub unsafe fn load(con: duckdb_connection, name: Option<&str>) -> Result<Self, ExtensionError> {
        let context = |detail: String| {
            ExtensionError::new(format!(
                "cannot check whether a scalar function signature is already taken: {detail}"
            ))
        };
        let mut sql = String::from(
            "SELECT function_name, \
                    system.main.array_to_string(parameter_types, system.main.chr(31)), \
                    varargs \
             FROM system.main.duckdb_functions() \
             WHERE database_name = 'system' AND schema_name = 'main' \
               AND function_type = 'scalar'",
        );
        if name.is_some() {
            sql.push_str(" AND system.main.lower(function_name) = system.main.lower($1)");
        }
        // SAFETY: `con` is valid per this function's contract.
        let statement =
            unsafe { crate::query::prepare(con, &sql) }.map_err(|e| context(e.to_string()))?;
        if let Some(name) = name {
            statement
                .bind_str(1, name)
                .map_err(|e| context(e.to_string()))?;
        }
        let mut result = statement.execute().map_err(|e| context(e.to_string()))?;
        // SAFETY: `con` is open, so the dispatch table is initialised.
        let engine = unsafe { crate::abi::engine_version() };
        let mut existing = Self {
            refuses_existing_names: engine
                .as_deref()
                .and_then(crate::abi::parse_version)
                .is_some_and(|version| version < (1, 5, 0)),
            ..Self::default()
        };
        while let Some(chunk) = result.next_chunk().map_err(|e| context(e.to_string()))? {
            if chunk.column_count() != 3 {
                return Err(context(
                    "the catalog query returned an unexpected shape".into(),
                ));
            }
            // SAFETY: three VARCHAR columns; each row's validity is checked
            // before its string is read, and the chunk outlives the readers.
            unsafe {
                let (names, params, varargs) = (chunk.reader(0), chunk.reader(1), chunk.reader(2));
                for row in 0..chunk.size() {
                    if !names.is_valid(row) {
                        continue;
                    }
                    let joined = if params.is_valid(row) {
                        params.read_str(row)
                    } else {
                        ""
                    };
                    let rendered = Rendered {
                        fixed: if joined.is_empty() {
                            Vec::new()
                        } else {
                            joined.split(SEPARATOR).map(str::to_owned).collect()
                        },
                        varargs: varargs
                            .is_valid(row)
                            .then(|| varargs.read_str(row).to_owned()),
                    };
                    existing.record(names.read_str(row), rendered);
                }
            }
        }
        Ok(existing)
    }

    /// Adds `signature` under `name`, as a successful registration does.
    pub fn record(&mut self, name: &str, signature: Rendered) {
        self.by_name
            .entry(name.to_lowercase())
            .or_default()
            .push(signature);
    }

    /// Refuses `signature` if it overlaps one already listed under `name`.
    /// `what` names the overload in the error (`"scalar function"`,
    /// `"overload 2"`).
    pub fn check(
        &self,
        name: &str,
        what: &str,
        signature: &Rendered,
    ) -> Result<(), ExtensionError> {
        let Some(existing) = self.by_name.get(&name.to_lowercase()) else {
            return Ok(());
        };
        if self.refuses_existing_names && !existing.is_empty() {
            return Err(ExtensionError::new(format!(
                "scalar function '{name}' ({what}): a scalar function named '{name}' already \
                 exists, and DuckDB before v1.5.0 cannot add an overload to an existing name \
                 (its C API registers with CREATE, not ALTER_ON_CONFLICT), so the registration \
                 would fail with no reason given. Choose a different name, or register every \
                 overload in one ScalarFunctionSetBuilder"
            )));
        }
        let Some(taken) = existing
            .iter()
            .find(|e| shapes_overlap(e.shape(), signature.shape()))
        else {
            return Ok(());
        };
        let theirs = display(taken);
        if taken == signature {
            return Err(ExtensionError::new(format!(
                "scalar function '{name}' ({what}): a scalar function {name}({theirs}) already \
                 exists (a built-in, another extension's, or an earlier registration). DuckDB \
                 would silently replace it for every query in the database, or make every call \
                 ambiguous if the return type differs, and still report success. Choose a \
                 different name or different parameter types."
            )));
        }
        Err(ExtensionError::new(format!(
            "scalar function '{name}' ({what}): {name}({ours}) accepts the same arguments as the \
             existing {name}({theirs}) for some calls, and DuckDB would fail every such call \
             with \"Could not choose a best candidate function\". Choose a different name or \
             parameter types that no call can match both ways.",
            ours = display(signature),
        )))
    }
}

/// Renders a signature for an error message: `BIGINT, VARCHAR...`.
fn display(signature: &Rendered) -> String {
    let mut out = signature.fixed.join(", ");
    if let Some(ref varargs) = signature.varargs {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(varargs);
        out.push_str("...");
    }
    out
}

/// Refuses the registration of `signatures` (one per overload) if any
/// overlaps a scalar signature already in the catalog.
///
/// With a `snapshot`, the catalog is listed into it once and reused; without
/// one, it is listed now, for `name` only. An overload that cannot be rendered
/// is not checked, and when none can be, the catalog is not listed at all.
///
/// # Safety
///
/// `con` must be a valid, open connection.
pub unsafe fn refuse_taken_signatures(
    con: duckdb_connection,
    snapshot: Option<&RefCell<Option<ExistingScalars>>>,
    name: &str,
    signatures: &[(String, Option<Rendered>)],
) -> Result<(), ExtensionError> {
    if signatures.iter().all(|(_, rendered)| rendered.is_none()) {
        return Ok(());
    }
    let check = |existing: &ExistingScalars| {
        signatures.iter().try_for_each(|(what, rendered)| {
            rendered
                .as_ref()
                .map_or(Ok(()), |r| existing.check(name, what, r))
        })
    };
    match snapshot {
        Some(cell) => {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                // SAFETY: `con` is valid per this function's contract.
                *slot = Some(unsafe { ExistingScalars::load(con, None) }?);
            }
            slot.as_ref().map_or(Ok(()), check)
        }
        // SAFETY: as above.
        None => check(&unsafe { ExistingScalars::load(con, Some(name)) }?),
    }
}

/// Records a successful registration of `signatures` in `snapshot`, if it has
/// been loaded, so later checks through the same snapshot see them.
pub fn record_registered(
    snapshot: Option<&RefCell<Option<ExistingScalars>>>,
    name: &str,
    signatures: Vec<(String, Option<Rendered>)>,
) {
    let Some(cell) = snapshot else { return };
    if let Some(existing) = cell.borrow_mut().as_mut() {
        for (_, rendered) in signatures {
            if let Some(rendered) = rendered {
                existing.record(name, rendered);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::id_string;
    use crate::types::TypeId;

    #[test]
    fn ids_render_as_duckdb_prints_them() {
        assert_eq!(id_string(TypeId::BigInt), Some("BIGINT"));
        assert_eq!(
            id_string(TypeId::TimestampTz),
            Some("TIMESTAMP WITH TIME ZONE")
        );
        assert_eq!(id_string(TypeId::TimeTz), Some("TIME WITH TIME ZONE"));
        assert_eq!(id_string(TypeId::Varint), Some("BIGNUM"));
        assert_eq!(id_string(TypeId::Struct), None);
        assert_eq!(id_string(TypeId::Decimal), None);
        assert_eq!(id_string(TypeId::SqlNull), None);
    }
}
