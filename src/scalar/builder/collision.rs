// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Refusing a scalar registration that would silently replace an existing one.
//!
//! `duckdb_register_scalar_function` adds to the catalog with
//! `ALTER_ON_CONFLICT`. When the name already holds a scalar function the new
//! overloads are merged in with `override = true`
//! (`ScalarFunctionCatalogEntry` → `MergeFunctionSet`), so an overload whose
//! parameter types match an existing one **replaces it for every connection**
//! — a built-in included — and the call still reports success. Registering
//! `abs(BIGINT)` from an extension changes what every `abs(x::BIGINT)` in the
//! database returns.
//!
//! Before registering, the scalar builders render each overload's parameter
//! and varargs types the way `DuckDB` prints them in `duckdb_functions()` and
//! refuse an overload that is already listed. Rendering covers every type
//! except `STRUCT`, `UNION`, `ENUM` and the literal pseudo-types; an overload
//! using one of those is not checked (its rendering in `duckdb_functions()`
//! quotes names in ways not worth reproducing), which is documented on the
//! builders. `tests/ffi_roundtrip/lifecycle.rs` holds every rendering to the
//! string the linked `DuckDB` reports.

use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_array_type_child_type, duckdb_connection,
    duckdb_decimal_scale, duckdb_decimal_width, duckdb_get_type_id, duckdb_list_type_child_type,
    duckdb_logical_type, duckdb_map_type_key_type, duckdb_map_type_value_type,
    DUCKDB_TYPE_DUCKDB_TYPE_ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL, DUCKDB_TYPE_DUCKDB_TYPE_ENUM,
    DUCKDB_TYPE_DUCKDB_TYPE_LIST, DUCKDB_TYPE_DUCKDB_TYPE_MAP, DUCKDB_TYPE_DUCKDB_TYPE_STRUCT,
    DUCKDB_TYPE_DUCKDB_TYPE_UNION,
};

use super::signature::ParamRef;
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

/// Renders one overload's signature: its parameter types, and its varargs type
/// (or `""`), or `None` if any of them is not rendered.
///
/// # Safety
///
/// Every logical parameter must be a live handle.
unsafe fn render_signature(params: &[ParamRef<'_>]) -> Option<(Vec<String>, String)> {
    let mut types = Vec::with_capacity(params.len());
    let mut varargs = String::new();
    for param in params {
        match param {
            ParamRef::Id(id) => types.push(id_string(*id)?.to_owned()),
            // SAFETY: live per this function's contract.
            ParamRef::Logical(lt) => types.push(unsafe { type_string(lt.as_raw()) }?),
            // SAFETY: as above.
            ParamRef::Varargs(lt) => varargs = unsafe { type_string(lt.as_raw()) }?,
        }
    }
    Some((types, varargs))
}

/// Separates rendered parameter types; `DuckDB` type names never contain it.
const SEPARATOR: char = '\u{1f}';

/// Refuses the registration if a scalar function `name` with exactly the
/// signature `params` is already in the system catalog.
///
/// A signature [`render_signature`] cannot render is not checked. `what`
/// names the overload in the error (`"scalar function"`, `"overload 2"`).
///
/// # Safety
///
/// `con` must be a valid, open connection, and every logical parameter a live
/// handle.
pub unsafe fn refuse_replacing_a_scalar(
    con: duckdb_connection,
    name: &str,
    what: &str,
    params: &[ParamRef<'_>],
) -> Result<(), ExtensionError> {
    // SAFETY: forwarded from this function's own contract.
    let Some((types, varargs)) = (unsafe { render_signature(params) }) else {
        return Ok(());
    };
    let context = |detail: String| {
        ExtensionError::new(format!(
            "scalar function '{name}' ({what}): cannot check whether the signature is taken: \
             {detail}"
        ))
    };
    let sql = "SELECT count(*) FROM duckdb_functions() \
               WHERE database_name = 'system' AND schema_name = 'main' \
               AND function_type = 'scalar' AND lower(function_name) = lower($1) \
               AND array_to_string(parameter_types, chr(31)) = $2 \
               AND coalesce(varargs, '') = $3";
    let joined = types.join(&SEPARATOR.to_string());
    // SAFETY: `con` is valid per this function's contract.
    let statement =
        unsafe { crate::query::prepare(con, sql) }.map_err(|e| context(e.to_string()))?;
    statement
        .bind_str(1, name)
        .map_err(|e| context(e.to_string()))?;
    statement
        .bind_str(2, &joined)
        .map_err(|e| context(e.to_string()))?;
    statement
        .bind_str(3, &varargs)
        .map_err(|e| context(e.to_string()))?;
    let mut result = statement.execute().map_err(|e| context(e.to_string()))?;
    let chunk = result
        .next_chunk()
        .map_err(|e| context(e.to_string()))?
        .ok_or_else(|| context("the check returned no rows".into()))?;
    if chunk.size() != 1 || chunk.column_count() != 1 {
        return Err(context("the check returned an unexpected shape".into()));
    }
    // SAFETY: one row, one BIGINT column; `count(*)` is never NULL.
    if unsafe { chunk.reader(0).read_i64(0) } == 0 {
        return Ok(());
    }
    let mut sig = types.join(", ");
    if !varargs.is_empty() {
        sig = if sig.is_empty() {
            format!("{varargs}...")
        } else {
            format!("{sig}, {varargs}...")
        };
    }
    Err(ExtensionError::new(format!(
        "scalar function '{name}' ({what}): a scalar function {name}({sig}) already exists (a \
         built-in, another extension's, or an earlier registration). DuckDB would silently \
         replace it for every query in the database and still report success. Choose a \
         different name or different parameter types."
    )))
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
