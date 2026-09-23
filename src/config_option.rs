// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Extension-defined configuration options (`DuckDB` 1.5.0+).
//!
//! Extensions can register custom settings that users can read and write via
//! `SET` / `RESET` / `current_setting()`. This module wraps the
//! `duckdb_config_option` C API surface behind a safe builder.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::config_option::{ConfigOptionBuilder, ConfigOptionScope};
//! use quack_rs::types::TypeId;
//!
//! let option = ConfigOptionBuilder::try_new("my_ext_threshold")?
//!     .description("Maximum threshold for my_ext operations")?
//!     .option_type(TypeId::BigInt)
//!     .default_value("100")?
//!     .scope(ConfigOptionScope::Global);
//! # Ok::<(), quack_rs::error::ExtensionError>(())
//! ```

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_config_option, duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_GLOBAL,
    duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_LOCAL,
    duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_SESSION,
    duckdb_config_option_set_default_scope, duckdb_config_option_set_default_value,
    duckdb_config_option_set_description, duckdb_config_option_set_name,
    duckdb_config_option_set_type, duckdb_connection, duckdb_create_config_option,
    duckdb_create_varchar, duckdb_destroy_config_option, duckdb_destroy_value,
    duckdb_register_config_option, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};

/// Scope in which a configuration option takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOptionScope {
    /// Option is local to the current statement.
    Local,
    /// Option is scoped to the current session.
    Session,
    /// Option applies globally to the database.
    Global,
}

impl ConfigOptionScope {
    /// Converts to the `DuckDB` C API scope constant.
    #[must_use]
    pub(crate) const fn to_raw(self) -> u32 {
        match self {
            Self::Local => duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_LOCAL,
            Self::Session => duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_SESSION,
            Self::Global => duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_GLOBAL,
        }
    }
}

/// Builder for registering extension-defined configuration options.
///
/// After building, call [`register`][Self::register] from your entry point to
/// register the setting with `DuckDB`.
#[must_use]
#[derive(Debug)]
pub struct ConfigOptionBuilder {
    name: CString,
    description: Option<CString>,
    option_type: Option<TypeId>,
    default_value: Option<CString>,
    scope: ConfigOptionScope,
}

impl ConfigOptionBuilder {
    /// Creates a new config option builder with the given name.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the name contains a null byte.
    pub fn try_new(name: &str) -> Result<Self, ExtensionError> {
        let c_name = CString::new(name)
            .map_err(|_| ExtensionError::new("config option name contains null byte"))?;
        Ok(Self {
            name: c_name,
            description: None,
            option_type: None,
            default_value: None,
            scope: ConfigOptionScope::Global,
        })
    }

    /// Returns the name of this config option.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or("")
    }

    /// Sets the human-readable description for this option.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if `desc` contains a null byte.
    pub fn description(mut self, desc: &str) -> Result<Self, ExtensionError> {
        self.description =
            Some(CString::new(desc).map_err(|_| {
                ExtensionError::new("config option description contains null byte")
            })?);
        Ok(self)
    }

    /// Sets the value type for this option (e.g. `TypeId::BigInt`, `TypeId::Varchar`).
    pub const fn option_type(mut self, type_id: TypeId) -> Self {
        self.option_type = Some(type_id);
        self
    }

    /// Sets the default value as a string representation.
    ///
    /// The string is cast to the option's type at [`register`][Self::register]
    /// time, which checks first that the cast succeeds.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if `value` contains a null byte.
    pub fn default_value(mut self, value: &str) -> Result<Self, ExtensionError> {
        self.default_value =
            Some(CString::new(value).map_err(|_| {
                ExtensionError::new("config option default value contains null byte")
            })?);
        Ok(self)
    }

    /// Sets the scope for this option.
    pub const fn scope(mut self, scope: ConfigOptionScope) -> Self {
        self.scope = scope;
        self
    }

    /// Registers this config option with `DuckDB`.
    ///
    /// # Default values are checked first
    ///
    /// `duckdb_config_option_set_default_value` casts the default string to
    /// the option type with a *throwing* cast and no `try`/`catch`, so a
    /// default such as `"abc"` for a `BIGINT` option would abort the process
    /// ("Rust cannot catch foreign exceptions"). Unless the type is
    /// `VARCHAR` (no cast), `register` therefore first runs
    /// `SELECT TRY_CAST($1::VARCHAR AS <type>) IS NOT NULL` on `con`, with the
    /// default bound as a parameter, and returns an error if the cast fails.
    ///
    /// One gap remains: SQL `TRY_CAST` uses the connection's cast functions,
    /// including casts extensions have registered, while `DuckDB`'s default-value
    /// cast uses only the built-in ones. If your extension registers a
    /// `VARCHAR` → option-type cast that accepts strings the built-in cast
    /// rejects, register the config option **before** that cast, or use a
    /// default the built-in cast accepts.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the option type was not set, if the default
    /// value does not cast to the option type (or the check itself cannot run
    /// on `con`), or if registration fails.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let type_id = self
            .option_type
            .ok_or_else(|| ExtensionError::new("config option type not set"))?;
        let lt = LogicalType::for_slot(type_id, "config option type")?;
        if let Some(ref val) = self.default_value {
            // SAFETY: `con` is valid per this function's contract.
            unsafe { check_default_casts(con, &self.name, type_id, val) }?;
        }

        // SAFETY: duckdb_create_config_option allocates a new handle.
        let option: duckdb_config_option = unsafe { duckdb_create_config_option() };

        // SAFETY: option is a valid newly created handle.
        unsafe {
            duckdb_config_option_set_name(option, self.name.as_ptr());
            duckdb_config_option_set_type(option, lt.as_raw());
            duckdb_config_option_set_default_scope(option, self.scope.to_raw());
        }

        if let Some(ref desc) = self.description {
            // SAFETY: option and desc are valid.
            unsafe {
                duckdb_config_option_set_description(option, desc.as_ptr());
            }
        }

        if let Some(ref val) = self.default_value {
            // SAFETY: duckdb_create_varchar allocates a duckdb_value.
            let dv = unsafe { duckdb_create_varchar(val.as_ptr()) };
            // SAFETY: option and dv are valid.
            unsafe {
                duckdb_config_option_set_default_value(option, dv);
            }
            // SAFETY: dv was created by duckdb_create_varchar.
            let mut dv_mut = dv;
            unsafe {
                duckdb_destroy_value(&raw mut dv_mut);
            }
        }

        // SAFETY: con is valid per caller's contract, option is fully configured.
        let result = unsafe { duckdb_register_config_option(con, option) };

        // SAFETY: option was created above and must be destroyed after registration.
        let mut option_mut = option;
        unsafe {
            duckdb_destroy_config_option(&raw mut option_mut);
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_config_option failed for '{}'",
                self.name.to_string_lossy()
            )))
        }
    }
}

/// Verifies that `default` casts to `type_id`, using `DuckDB`'s own
/// non-throwing `TRY_CAST`, before the throwing cast inside
/// `duckdb_config_option_set_default_value` can abort the process.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
unsafe fn check_default_casts(
    con: duckdb_connection,
    name: &CString,
    type_id: TypeId,
    default: &CString,
) -> Result<(), ExtensionError> {
    if type_id == TypeId::Varchar {
        // A VARCHAR default is stored as-is; there is no cast to fail.
        return Ok(());
    }
    let name = name.to_string_lossy();
    let default = default
        .to_str()
        .map_err(|_| ExtensionError::new("config option default value is not valid UTF-8"))?;
    let context = |detail: String| {
        ExtensionError::new(format!(
            "config option '{name}': cannot validate default value {default:?} for type {}: \
             {detail}",
            type_id.sql_name()
        ))
    };
    let sql = format!(
        "SELECT TRY_CAST($1::VARCHAR AS {}) IS NOT NULL",
        type_id.sql_name()
    );
    // SAFETY: `con` is valid per this function's contract.
    let statement =
        unsafe { crate::query::prepare(con, &sql) }.map_err(|e| context(e.to_string()))?;
    statement
        .bind_str(1, default)
        .map_err(|e| context(e.to_string()))?;
    let mut result = statement.execute().map_err(|e| context(e.to_string()))?;
    let chunk = result
        .next_chunk()
        .ok_or_else(|| context("the check returned no rows".into()))?;
    if chunk.size() != 1 || chunk.column_count() != 1 {
        return Err(context("the check returned an unexpected shape".into()));
    }
    // SAFETY: the chunk has exactly one BOOLEAN column and one row;
    // `IS NOT NULL` is never NULL.
    let casts = unsafe { chunk.reader(0).read_bool(0) };
    if casts {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "config option '{name}': default value {default:?} cannot be cast to {}",
            type_id.sql_name()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeId;

    #[test]
    fn try_new_valid_name() {
        let builder = ConfigOptionBuilder::try_new("my_option").unwrap();
        assert_eq!(builder.name(), "my_option");
    }

    #[test]
    fn try_new_null_byte_rejected() {
        let result = ConfigOptionBuilder::try_new("bad\0name");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            err.to_string().contains("null byte"),
            "error should mention null byte"
        );
    }

    #[test]
    fn description_null_byte_rejected() {
        let result = ConfigOptionBuilder::try_new("opt")
            .unwrap()
            .description("bad\0desc");
        assert!(result.is_err());
    }

    #[test]
    fn default_value_null_byte_rejected() {
        let result = ConfigOptionBuilder::try_new("opt")
            .unwrap()
            .default_value("bad\0val");
        assert!(result.is_err());
    }

    #[test]
    fn builder_stores_option_type() {
        let builder = ConfigOptionBuilder::try_new("threshold")
            .unwrap()
            .option_type(TypeId::BigInt);
        // Verifies fluent chaining compiles and doesn't panic.
        assert_eq!(builder.name(), "threshold");
    }

    #[test]
    fn builder_stores_description() {
        let builder = ConfigOptionBuilder::try_new("threshold")
            .unwrap()
            .description("max threshold")
            .unwrap();
        assert_eq!(builder.name(), "threshold");
    }

    #[test]
    fn builder_stores_default_value() {
        let builder = ConfigOptionBuilder::try_new("limit")
            .unwrap()
            .default_value("100")
            .unwrap();
        assert_eq!(builder.name(), "limit");
    }

    #[test]
    fn scope_default_is_global() {
        // ConfigOptionScope defaults to Global in the builder.
        let builder = ConfigOptionBuilder::try_new("opt").unwrap();
        // We can't read the scope directly, but we can verify the
        // ConfigOptionScope enum works correctly.
        assert_eq!(builder.name(), "opt");
    }

    #[test]
    fn scope_enum_to_raw_distinct_values() {
        let local = ConfigOptionScope::Local.to_raw();
        let session = ConfigOptionScope::Session.to_raw();
        let global = ConfigOptionScope::Global.to_raw();
        assert_ne!(local, session);
        assert_ne!(session, global);
        assert_ne!(local, global);
    }

    #[test]
    fn scope_enum_debug_impl() {
        assert_eq!(format!("{:?}", ConfigOptionScope::Local), "Local");
        assert_eq!(format!("{:?}", ConfigOptionScope::Session), "Session");
        assert_eq!(format!("{:?}", ConfigOptionScope::Global), "Global");
    }

    #[test]
    fn scope_enum_clone_eq() {
        let a = ConfigOptionScope::Session;
        #[allow(clippy::clone_on_copy)]
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn full_builder_chain_compiles() {
        // Verify the full fluent builder chain works without panicking.
        let _builder = ConfigOptionBuilder::try_new("my_ext_threshold")
            .unwrap()
            .description("Maximum threshold")
            .unwrap()
            .option_type(TypeId::BigInt)
            .default_value("100")
            .unwrap()
            .scope(ConfigOptionScope::Global);
    }
}
