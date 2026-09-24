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
    duckdb_destroy_config_option, duckdb_register_config_option, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};
use crate::value::Value;
use crate::vector::VectorReader;

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
    /// The string is converted to the option's type at
    /// [`register`][Self::register] time, with the same SQL cast `SET` uses;
    /// a string that does not convert is an error there. Every option needs a
    /// default — see [`register`][Self::register].
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

    /// The checks that need no `DuckDB` call: a concrete option type and a
    /// default value. [`MockRegistrar`][crate::testing::MockRegistrar] runs
    /// them too.
    pub(crate) fn check_parts(&self) -> Result<(), ExtensionError> {
        let name = self.name.to_string_lossy();
        let type_id = self
            .option_type
            .ok_or_else(|| ExtensionError::new("config option type not set"))?;
        if matches!(
            type_id,
            TypeId::Any | TypeId::SqlNull | TypeId::IntegerLiteral | TypeId::StringLiteral
        ) {
            return Err(ExtensionError::new(format!(
                "config option '{name}': {} cannot be the type of a config option; it is a \
                 pseudo-type that no value is stored as. Use a concrete type such as VARCHAR.",
                type_id.sql_name()
            )));
        }
        if self.default_value.is_none() {
            return Err(ExtensionError::new(format!(
                "config option '{name}' has no default value. DuckDB treats an extension option \
                 without one as unset: current_setting('{name}') fails with \"unrecognized \
                 configuration parameter\" until it is SET. Call default_value(...)."
            )));
        }
        Ok(())
    }

    /// Registers this config option with `DuckDB`.
    ///
    /// # The default is converted by SQL, never inside the C API
    ///
    /// `duckdb_config_option_set_default_value` converts a default whose type
    /// differs from the option's with `Value::DefaultCastAs` — a *throwing*
    /// cast with no `try`/`catch` around it — so a string default that does
    /// not convert would abort the process ("Rust cannot catch foreign
    /// exceptions"). Worse, `DefaultCastAs` uses only the built-in casts while
    /// SQL uses the connection's, so a string one accepts can still make the
    /// other throw: with ICU loaded, `'2020-01-01 10:00:00 America/New_York'`
    /// is a valid `TIMESTAMPTZ` in SQL and an abort in `DefaultCastAs`.
    ///
    /// `register` therefore converts the default itself, by running
    /// `SELECT TRY_CAST($1::VARCHAR AS <type>)` on `con` with the default bound
    /// as a parameter, and hands `DuckDB` a value that **already has the
    /// option's type**. `duckdb_config_option_set_default_value` stores such a
    /// value as-is (it only casts when `coption->type != cvalue->type()`,
    /// `src/main/capi/config_options-c.cpp`), so no cast can run inside the C
    /// API. The stored default is exactly what `TRY_CAST('<default>' AS <type>)`
    /// produces in SQL on `con`.
    ///
    /// Option types whose values the C API cannot construct from a query
    /// result (`BIGNUM`, `GEOMETRY`, `VARIANT`, ...) are refused rather than
    /// sent down the throwing path.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError`, before anything is registered, if:
    ///
    /// - the option type was not set, or is a pseudo-type (`ANY`, `SQLNULL`,
    ///   the literal types) that no setting can have;
    /// - no default value was set. `DuckDB` treats an extension option
    ///   without a default as unset: `current_setting` reports it as an
    ///   "unrecognized configuration parameter" until someone `SET`s it, and
    ///   [`ClientContext::config_option`][crate::client_context::ClientContext::config_option]
    ///   on it takes the path a debug build of `DuckDB` asserts on;
    /// - the name is already a setting — built-in (`threads`), an alias of
    ///   one (`worker_threads`), or another extension's. Names are compared
    ///   case-insensitively, as `DuckDB` resolves them. `DuckDB` itself only
    ///   refuses a name another *extension option* has, and silently lets an
    ///   extension option shadow a built-in one for `current_setting`;
    /// - the default does not convert to the option type (or the conversion
    ///   query cannot run on `con`), or the type cannot carry a default;
    /// - `duckdb_register_config_option` fails.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        self.check_parts()?;
        let name = self.name.to_string_lossy();
        let type_id = self
            .option_type
            .ok_or_else(|| ExtensionError::new("config option type not set"))?;
        let Some(ref default) = self.default_value else {
            return Err(ExtensionError::new("config option has no default value"));
        };
        let lt = LogicalType::for_slot(type_id, "config option type")?;
        // SAFETY: `con` is valid per this function's contract.
        unsafe { refuse_existing_setting(con, &name) }?;
        // SAFETY: `con` is valid per this function's contract.
        let default = unsafe { typed_default(con, &name, type_id, default) }?;

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

        // SAFETY: option and the value are valid. The value already has the
        // option's type, so DuckDB copies it without casting (no throw
        // possible); `default` still owns it and destroys it on drop.
        unsafe {
            duckdb_config_option_set_default_value(option, default.as_raw());
        }

        // SAFETY: con is valid per caller's contract, option is fully configured.
        let result = unsafe { duckdb_register_config_option(con, option) };

        // SAFETY: option was created above and must be destroyed after registration.
        let mut option_mut = option;
        // SAFETY: `option_mut` holds the `new CConfigOption` from
        // `duckdb_create_config_option` above, destroyed nowhere else.
        // `duckdb_register_config_option` copied its name, description, type and
        // default into the database config (`AddExtensionOption`,
        // config_options-c.cpp) and kept no pointer to it, so this single
        // `delete` is safe whether registration succeeded or not.
        unsafe {
            duckdb_destroy_config_option(&raw mut option_mut);
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_config_option failed for '{name}'"
            )))
        }
    }
}

/// Refuses `name` if `duckdb_settings()` already lists it, as a setting or as
/// an alias of one, compared case-insensitively.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
unsafe fn refuse_existing_setting(
    con: duckdb_connection,
    name: &str,
) -> Result<(), ExtensionError> {
    let context = |detail: String| {
        ExtensionError::new(format!(
            "config option '{name}': cannot check whether the name is taken: {detail}"
        ))
    };
    // `aliases` is a VARCHAR[] column; unnesting it avoids lambda syntax,
    // whose spelling DuckDB has been changing.
    // Every function is qualified with `system.main`, so a user macro of the
    // same name cannot change what the check computes.
    let sql = "SELECT (SELECT count(*) FROM system.main.duckdb_settings() \
                  WHERE system.main.lower(name) = system.main.lower($1)) \
               + (SELECT count(*) FROM (SELECT system.main.unnest(aliases) AS alias \
                  FROM system.main.duckdb_settings()) \
                  WHERE system.main.lower(alias) = system.main.lower($1))";
    // SAFETY: `con` is valid per this function's contract.
    let statement =
        unsafe { crate::query::prepare(con, sql) }.map_err(|e| context(e.to_string()))?;
    statement
        .bind_str(1, name)
        .map_err(|e| context(e.to_string()))?;
    let mut result = statement.execute().map_err(|e| context(e.to_string()))?;
    let chunk = result
        .next_chunk()
        .map_err(|e| context(e.to_string()))?
        .ok_or_else(|| context("the check returned no rows".into()))?;
    if chunk.size() != 1 || chunk.column_count() != 1 {
        return Err(context("the check returned an unexpected shape".into()));
    }
    // SAFETY: one row, one BIGINT column; a sum of two counts is never NULL.
    let taken = unsafe { chunk.reader(0).read_i64(0) };
    if taken == 0 {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "config option '{name}' already exists: DuckDB already has a setting (or setting \
             alias) with this name — see `SELECT * FROM duckdb_settings()`. Registering it would \
             fail, or silently shadow the built-in setting in current_setting(). Choose a name \
             prefixed with your extension's name."
        )))
    }
}

/// Converts `default` to a value of type `type_id` with SQL
/// `TRY_CAST($1::VARCHAR AS <type>)` and rebuilds the result as a
/// `duckdb_value` of exactly that type.
///
/// This is what keeps `duckdb_config_option_set_default_value` from running
/// its throwing `DefaultCastAs`: that call only casts when the value's type
/// differs from the option's.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
unsafe fn typed_default(
    con: duckdb_connection,
    name: &str,
    type_id: TypeId,
    default: &CString,
) -> Result<Value, ExtensionError> {
    let default = default
        .to_str()
        .map_err(|_| ExtensionError::new("config option default value is not valid UTF-8"))?;
    let context = |detail: String| {
        ExtensionError::new(format!(
            "config option '{name}': cannot convert default value {default:?} to {}: {detail}",
            type_id.sql_name()
        ))
    };
    let sql = format!("SELECT TRY_CAST($1::VARCHAR AS {})", type_id.sql_name());
    // SAFETY: `con` is valid per this function's contract.
    let statement =
        unsafe { crate::query::prepare(con, &sql) }.map_err(|e| context(e.to_string()))?;
    statement
        .bind_str(1, default)
        .map_err(|e| context(e.to_string()))?;
    let mut result = statement.execute().map_err(|e| context(e.to_string()))?;
    let chunk = result
        .next_chunk()
        .map_err(|e| context(e.to_string()))?
        .ok_or_else(|| context("the conversion returned no rows".into()))?;
    if chunk.size() != 1 || chunk.column_count() != 1 {
        return Err(context(
            "the conversion returned an unexpected shape".into(),
        ));
    }
    // SAFETY: the chunk has one column and one row.
    let reader = unsafe { chunk.reader(0) };
    // SAFETY: row 0 exists.
    if !unsafe { reader.is_valid(0) } {
        return Err(ExtensionError::new(format!(
            "config option '{name}': default value {default:?} cannot be cast to {}",
            type_id.sql_name()
        )));
    }
    // SAFETY: row 0 exists, is valid, and the column has type `type_id`.
    unsafe { value_of_type(&reader, type_id) }.ok_or_else(|| {
        ExtensionError::new(format!(
            "config option '{name}': a default value of type {} is not supported; the C API \
             cannot build one without a cast that may abort the process. Use a VARCHAR option \
             and parse it yourself.",
            type_id.sql_name()
        ))
    })
}

/// Reads row 0 of `reader` as a [`Value`] of type `type_id`, or `None` for a
/// type the C API has no direct constructor for.
///
/// # Safety
///
/// Row 0 must exist, be valid, and hold a value of type `type_id`.
unsafe fn value_of_type(reader: &VectorReader, type_id: TypeId) -> Option<Value> {
    // SAFETY: (every arm) row 0 exists, is valid and holds a `type_id` value,
    // per this function's `# Safety` contract, so each `read_*(0)` is in
    // bounds and reads the physical layout it expects (`BIT` shares BLOB's
    // `duckdb_string_t` layout). `reader` borrows the chunk, so the string
    // bytes `read_str` / `read_blob` borrow stay live while the constructors
    // copy them; `duckdb_create_bit` copies too (`Value::BIT`), and
    // `Value::from_raw` takes the fresh handle it returns. The temporal
    // constructors validate their payload; a value DuckDB's own `TRY_CAST`
    // produced is always in range, so their `Err` arm is unreachable here and
    // `.ok()?` only satisfies the type.
    let value = unsafe {
        match type_id {
            TypeId::Boolean => Value::boolean(reader.read_bool(0)),
            TypeId::TinyInt => Value::tinyint(reader.read_i8(0)),
            TypeId::SmallInt => Value::smallint(reader.read_i16(0)),
            TypeId::Integer => Value::integer(reader.read_i32(0)),
            TypeId::BigInt => Value::bigint(reader.read_i64(0)),
            TypeId::UTinyInt => Value::utinyint(reader.read_u8(0)),
            TypeId::USmallInt => Value::usmallint(reader.read_u16(0)),
            TypeId::UInteger => Value::uinteger(reader.read_u32(0)),
            TypeId::UBigInt => Value::ubigint(reader.read_u64(0)),
            TypeId::HugeInt => Value::hugeint(reader.read_i128(0)),
            TypeId::UHugeInt => Value::uhugeint(reader.read_u128(0)),
            TypeId::Float => Value::float(reader.read_f32(0)),
            TypeId::Double => Value::double(reader.read_f64(0)),
            TypeId::Date => Value::date(reader.read_date(0)),
            TypeId::Time => Value::time(reader.read_time(0)).ok()?,
            TypeId::TimeNs => Value::time_ns(reader.read_i64(0)).ok()?,
            TypeId::TimeTz => Value::time_tz(reader.read_time_tz(0)).ok()?,
            TypeId::Timestamp => Value::timestamp(reader.read_timestamp(0)).ok()?,
            TypeId::TimestampS => Value::timestamp_s(reader.read_timestamp_s(0)).ok()?,
            TypeId::TimestampMs => Value::timestamp_ms(reader.read_timestamp_ms(0)).ok()?,
            TypeId::TimestampNs => Value::timestamp_ns(reader.read_timestamp_ns(0)).ok()?,
            TypeId::TimestampTz => Value::timestamp_tz(reader.read_timestamp_tz(0)).ok()?,
            TypeId::Interval => Value::interval(reader.read_interval(0)),
            TypeId::Varchar => Value::varchar(reader.read_str(0)),
            TypeId::Blob => Value::blob(reader.read_blob(0)),
            TypeId::Uuid => Value::uuid(reader.read_uuid(0)),
            TypeId::Bit => {
                // A BIT vector stores the same bytes `Value::BIT` takes: the
                // padding byte followed by the bits.
                let bytes = reader.read_blob(0);
                let raw = libduckdb_sys::duckdb_create_bit(libduckdb_sys::duckdb_bit {
                    data: bytes.as_ptr().cast_mut(),
                    size: libduckdb_sys::idx_t::try_from(bytes.len()).ok()?,
                });
                Value::from_raw(raw)
            }
            _ => return None,
        }
    };
    Some(value)
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
        assert_eq!(builder.option_type, Some(TypeId::BigInt));
    }

    #[test]
    fn builder_stores_description() {
        let builder = ConfigOptionBuilder::try_new("threshold")
            .unwrap()
            .description("max threshold")
            .unwrap();
        assert_eq!(
            builder.description.as_deref(),
            Some(c"max threshold"),
            "the description must be stored"
        );
    }

    #[test]
    fn builder_stores_default_value() {
        let builder = ConfigOptionBuilder::try_new("limit")
            .unwrap()
            .default_value("100")
            .unwrap();
        assert_eq!(builder.default_value.as_deref(), Some(c"100"));
    }

    #[test]
    fn scope_default_is_global() {
        let builder = ConfigOptionBuilder::try_new("opt").unwrap();
        assert_eq!(builder.scope, ConfigOptionScope::Global);
        let builder = builder.scope(ConfigOptionScope::Session);
        assert_eq!(builder.scope, ConfigOptionScope::Session);
    }

    /// The pseudo-type and missing-default checks run before `con` is used,
    /// so a null connection is never dereferenced.
    #[test]
    fn pseudo_types_and_missing_defaults_are_refused_before_duckdb_is_called() {
        for ty in [
            TypeId::Any,
            TypeId::SqlNull,
            TypeId::IntegerLiteral,
            TypeId::StringLiteral,
        ] {
            // SAFETY: `register` returns before using `con`.
            let err = unsafe {
                ConfigOptionBuilder::try_new("opt")
                    .unwrap()
                    .option_type(ty)
                    .default_value("1")
                    .unwrap()
                    .register(std::ptr::null_mut())
            }
            .expect_err("a pseudo-type must be refused");
            assert!(
                err.as_str()
                    .contains("cannot be the type of a config option"),
                "{ty:?}: {err}"
            );
        }
        // SAFETY: `register` returns before using `con`.
        let err = unsafe {
            ConfigOptionBuilder::try_new("opt")
                .unwrap()
                .option_type(TypeId::BigInt)
                .register(std::ptr::null_mut())
        }
        .expect_err("an option without a default must be refused");
        assert!(err.as_str().contains("no default value"), "{err}");
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
    fn full_builder_chain_stores_every_field() {
        let builder = ConfigOptionBuilder::try_new("my_ext_threshold")
            .unwrap()
            .description("Maximum threshold")
            .unwrap()
            .option_type(TypeId::BigInt)
            .default_value("100")
            .unwrap()
            .scope(ConfigOptionScope::Local);
        assert_eq!(builder.name(), "my_ext_threshold");
        assert_eq!(builder.description.as_deref(), Some(c"Maximum threshold"));
        assert_eq!(builder.option_type, Some(TypeId::BigInt));
        assert_eq!(builder.default_value.as_deref(), Some(c"100"));
        assert_eq!(builder.scope, ConfigOptionScope::Local);
    }
}
