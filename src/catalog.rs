// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Catalog entry lookup (`DuckDB` 1.5.0+).
//!
//! Provides read-only access to catalog entries (tables, views, types, etc.)
//! from within extension callbacks.
//!
//! # Lookups that could autoload an extension are refused
//!
//! `duckdb_catalog_get_entry` has no `try`/`catch`. When a `TYPE` or
//! `COLLATION` lookup misses, `DuckDB` 1.5.5 tries to autoload the extension
//! that owns the name — `json` and `inet` for types, `icu` for 130 collation
//! names such as `de` or `en_us` — even though the lookup asked for
//! `RETURN_NULL` (`Catalog::GetEntry` → `AutoLoadExtensionByCatalogEntry`,
//! `src/catalog/catalog.cpp`). If that autoload fails (autoinstall off and the
//! extension not installed, no network, a signature problem, ...),
//! `ExtensionHelper::AutoLoadExtension` rethrows, the exception crosses the C
//! API and the process aborts. If it succeeds, a catalog *lookup* has just
//! downloaded and loaded an extension.
//!
//! The C API offers no way to ask whether that extension is already loaded
//! from a client context, so [`CatalogEntry::lookup`] refuses — with an
//! `Err`, without calling `DuckDB` — a `Type` or `Collation` lookup whose name
//! is on those lists while the `autoload_known_extensions` setting is on.
//! With `SET autoload_known_extensions = false` no autoload is attempted and
//! the lookup runs normally. Every other name and entry type is unaffected.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::catalog::{CatalogEntryType, CatalogEntry};
//! ```

use std::ffi::CStr;

use libduckdb_sys::{
    duckdb_catalog, duckdb_catalog_entry, duckdb_catalog_entry_get_name,
    duckdb_catalog_entry_get_type, duckdb_catalog_entry_type,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_COLLATION,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_DATABASE,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_INDEX,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_INVALID,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_PREPARED_STATEMENT,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_SCHEMA,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_SEQUENCE,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_TABLE,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_TYPE,
    duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_VIEW, duckdb_catalog_get_entry,
    duckdb_catalog_get_type_name, duckdb_client_context, duckdb_client_context_get_config_option,
    duckdb_config_option_scope, duckdb_destroy_catalog, duckdb_destroy_catalog_entry,
};

use crate::error::ExtensionError;

/// Type names a miss on which makes `DuckDB` 1.5.5 autoload an extension:
/// `EXTENSION_TYPES` in `src/include/duckdb/main/extension_entries.hpp`.
const AUTOLOADING_TYPE_NAMES: &[&str] = &["json", "inet"];

/// Collation names a miss on which makes `DuckDB` 1.5.5 autoload `icu`:
/// `EXTENSION_COLLATIONS` in `src/include/duckdb/main/extension_entries.hpp`.
const AUTOLOADING_COLLATION_NAMES: &[&str] = &[
    "af", "am", "ar", "ar_sa", "as", "az", "be", "bg", "bn", "bo", "br", "bs", "ca", "ceb", "chr",
    "cs", "cy", "da", "de", "de_at", "dsb", "dz", "ee", "el", "en", "en_us", "eo", "es", "et",
    "fa", "fa_af", "ff", "fi", "fil", "fo", "fr", "fr_ca", "fy", "ga", "gl", "gu", "ha", "haw",
    "he", "he_il", "hi", "hr", "hsb", "hu", "hy", "id", "id_id", "ig", "is", "it", "ja", "ka",
    "kk", "kl", "km", "kn", "ko", "kok", "ku", "ky", "lb", "lkt", "ln", "lo", "lt", "lv", "mk",
    "ml", "mn", "mr", "ms", "mt", "my", "nb", "nb_no", "ne", "nl", "nn", "om", "or", "pa", "pa_in",
    "pl", "ps", "pt", "ro", "ru", "sa", "se", "si", "sk", "sl", "smn", "sq", "sr", "sr_ba",
    "sr_me", "sr_rs", "sv", "sw", "ta", "te", "th", "tk", "to", "tr", "ug", "uk", "ur", "uz", "vi",
    "wae", "wo", "xh", "yi", "yo", "yue", "yue_cn", "zh", "zh_cn", "zh_hk", "zh_mo", "zh_sg",
    "zh_tw", "zu",
];

/// Types of entries in the `DuckDB` catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogEntryType {
    /// Invalid catalog entry.
    Invalid,
    /// A table.
    Table,
    /// A view.
    View,
    /// An index.
    Index,
    /// A schema.
    Schema,
    /// A prepared statement.
    PreparedStatement,
    /// A sequence.
    Sequence,
    /// A collation.
    Collation,
    /// A user-defined type.
    Type,
    /// A database.
    Database,
}

impl CatalogEntryType {
    /// Converts to the `DuckDB` C API constant.
    #[must_use]
    pub(crate) const fn to_raw(self) -> duckdb_catalog_entry_type {
        match self {
            Self::Invalid => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_INVALID,
            Self::Table => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_TABLE,
            Self::View => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_VIEW,
            Self::Index => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_INDEX,
            Self::Schema => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_SCHEMA,
            Self::PreparedStatement => {
                duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_PREPARED_STATEMENT
            }
            Self::Sequence => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_SEQUENCE,
            Self::Collation => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_COLLATION,
            Self::Type => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_TYPE,
            Self::Database => duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_DATABASE,
        }
    }

    /// Whether a lookup of `name` as this kind of entry can make `DuckDB`
    /// autoload an extension — see the [module docs](crate::catalog).
    ///
    /// `DuckDB` lower-cases the name before matching it against its lists, so
    /// this does too.
    #[must_use]
    pub fn may_autoload_extension(self, name: &str) -> bool {
        let list = match self {
            Self::Type => AUTOLOADING_TYPE_NAMES,
            Self::Collation => AUTOLOADING_COLLATION_NAMES,
            _ => return false,
        };
        let lower = name.to_lowercase();
        list.contains(&lower.as_str())
    }

    /// Whether `duckdb_catalog_get_entry` can look this kind of entry up.
    ///
    /// Lookups go through a schema's catalog sets, which exist only for
    /// tables, views, indexes, sequences, collations and types. For
    /// `Schema`, `Database`, `PreparedStatement` and `Invalid`, `DuckDB`
    /// 1.5.5 throws an `InternalException` ("Unsupported catalog type in
    /// schema") from inside the C API with no `try`/`catch`, which aborts the
    /// Rust process. [`CatalogEntry::lookup`] therefore refuses them.
    /// (`Type` and `Collation` are supported, but a few of their *names* are
    /// refused too; see [`may_autoload_extension`][Self::may_autoload_extension].)
    #[must_use]
    pub const fn is_lookup_supported(self) -> bool {
        matches!(
            self,
            Self::Table | Self::View | Self::Index | Self::Sequence | Self::Collation | Self::Type
        )
    }

    /// Converts from the `DuckDB` C API constant.
    #[must_use]
    pub(crate) const fn from_raw(raw: duckdb_catalog_entry_type) -> Self {
        match raw {
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_TABLE => Self::Table,
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_VIEW => Self::View,
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_INDEX => Self::Index,
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_SCHEMA => Self::Schema,
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_PREPARED_STATEMENT => {
                Self::PreparedStatement
            }
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_SEQUENCE => {
                Self::Sequence
            }
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_COLLATION => {
                Self::Collation
            }
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_TYPE => Self::Type,
            x if x == duckdb_catalog_entry_type_DUCKDB_CATALOG_ENTRY_TYPE_DATABASE => {
                Self::Database
            }
            _ => Self::Invalid,
        }
    }
}

/// RAII wrapper for a `duckdb_catalog_entry`.
///
/// Automatically destroyed when dropped.
pub struct CatalogEntry {
    entry: duckdb_catalog_entry,
}

impl CatalogEntry {
    /// Look up a catalog entry by type, schema, and name.
    ///
    /// Returns `Ok(None)` if no such entry exists.
    ///
    /// # Errors
    ///
    /// Refuses, without calling `DuckDB`, a lookup that would abort the
    /// process from inside the C API:
    ///
    /// - an `entry_type` that is not a schema-level entry (`Schema`,
    ///   `Database`, `PreparedStatement`, `Invalid`) — see
    ///   [`CatalogEntryType::is_lookup_supported`];
    /// - a `Type` or `Collation` whose name `DuckDB` would try to autoload an
    ///   extension for, while `autoload_known_extensions` is on — see the
    ///   [module docs](crate::catalog) and
    ///   [`CatalogEntryType::may_autoload_extension`].
    ///
    /// # Safety
    ///
    /// - `catalog` must be a valid `duckdb_catalog` handle.
    /// - `context` must be a valid `duckdb_client_context` handle.
    /// - Must be called from within an active transaction.
    pub unsafe fn lookup(
        catalog: duckdb_catalog,
        context: duckdb_client_context,
        schema: &CStr,
        name: &CStr,
        entry_type: CatalogEntryType,
    ) -> Result<Option<Self>, ExtensionError> {
        if !entry_type.is_lookup_supported() {
            return Err(ExtensionError::new(format!(
                "catalog lookup of a {entry_type:?} entry is not supported: DuckDB throws \
                 \"Unsupported catalog type in schema\" through the C API, which would abort \
                 the process"
            )));
        }
        let lossy_name = name.to_string_lossy();
        // SAFETY: `context` is valid per this function's contract.
        if entry_type.may_autoload_extension(&lossy_name) && unsafe { autoload_enabled(context) } {
            return Err(ExtensionError::new(format!(
                "catalog lookup of {entry_type:?} '{lossy_name}' refused: on a miss DuckDB \
                 autoloads the extension that owns this name, and a failed autoload throws \
                 through the C API and aborts the process. Run \
                 `SET autoload_known_extensions = false` first to look it up safely."
            )));
        }
        // SAFETY: catalog, context, schema, and name are valid per caller's contract.
        let entry = unsafe {
            duckdb_catalog_get_entry(
                catalog,
                context,
                entry_type.to_raw(),
                schema.as_ptr(),
                name.as_ptr(),
            )
        };
        if entry.is_null() {
            Ok(None)
        } else {
            Ok(Some(Self { entry }))
        }
    }

    /// Returns the name of this catalog entry.
    ///
    /// Returns `None` if the name is not valid UTF-8.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        // SAFETY: self.entry is valid.
        let ptr = unsafe { duckdb_catalog_entry_get_name(self.entry) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `DuckDB` returns a null-terminated UTF-8 string.
        unsafe { CStr::from_ptr(ptr) }.to_str().ok()
    }

    /// Returns the type of this catalog entry.
    #[must_use]
    pub fn entry_type(&self) -> CatalogEntryType {
        // SAFETY: self.entry is valid.
        let raw = unsafe { duckdb_catalog_entry_get_type(self.entry) };
        CatalogEntryType::from_raw(raw)
    }
}

impl Drop for CatalogEntry {
    fn drop(&mut self) {
        // SAFETY: self.entry was obtained from duckdb_catalog_get_entry.
        unsafe {
            duckdb_destroy_catalog_entry(&raw mut self.entry);
        }
    }
}

/// RAII wrapper for a `duckdb_catalog`.
///
/// Automatically destroyed when dropped.
pub struct Catalog {
    catalog: duckdb_catalog,
}

impl Catalog {
    /// Creates a `Catalog` from a raw handle.
    ///
    /// # Safety
    ///
    /// `catalog` must be a valid, non-null `duckdb_catalog` handle.
    pub(crate) const unsafe fn from_raw(catalog: duckdb_catalog) -> Self {
        Self { catalog }
    }

    /// Returns the raw handle for use with [`CatalogEntry::lookup`].
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_catalog {
        self.catalog
    }

    /// Returns the type name of this catalog (e.g. `"duckdb"`, `"system"`, or a
    /// storage extension's name like `"sqlite"`).
    ///
    /// Returns `None` if the name is not valid UTF-8.
    #[must_use]
    pub fn type_name(&self) -> Option<&str> {
        // SAFETY: self.catalog is valid per constructor contract. The returned
        // pointer is owned by DuckDB and remains valid while the catalog lives.
        let ptr = unsafe { duckdb_catalog_get_type_name(self.catalog) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: ptr is a valid null-terminated UTF-8 string owned by DuckDB.
        unsafe { CStr::from_ptr(ptr) }.to_str().ok()
    }

    /// Look up a catalog entry by type, schema, and name.
    ///
    /// Returns `Ok(None)` if no such entry exists.
    ///
    /// # Errors
    ///
    /// Returns an error, without calling `DuckDB`, for a lookup that would
    /// abort the process; see [`CatalogEntry::lookup`].
    ///
    /// # Safety
    ///
    /// - `context` must be a valid `duckdb_client_context`.
    /// - Must be called from within an active transaction context.
    pub unsafe fn get_entry(
        &self,
        context: duckdb_client_context,
        schema: &CStr,
        name: &CStr,
        entry_type: CatalogEntryType,
    ) -> Result<Option<CatalogEntry>, ExtensionError> {
        // SAFETY: self.catalog and context are valid, caller ensures active transaction.
        unsafe { CatalogEntry::lookup(self.catalog, context, schema, name, entry_type) }
    }
}

impl Drop for Catalog {
    fn drop(&mut self) {
        // SAFETY: self.catalog was obtained from duckdb_client_context_get_catalog.
        unsafe {
            duckdb_destroy_catalog(&raw mut self.catalog);
        }
    }
}

/// Whether `DuckDB` would try to autoload an extension on a catalog miss.
///
/// Reads the `autoload_known_extensions` setting (it always exists, so this
/// never takes the missing-setting path described on
/// [`ClientContext::config_option`][crate::client_context::ClientContext::config_option]).
/// Anything but a readable `false` counts as enabled.
///
/// # Safety
///
/// `context` must be a valid `duckdb_client_context`.
unsafe fn autoload_enabled(context: duckdb_client_context) -> bool {
    let mut scope: duckdb_config_option_scope = 0;
    // SAFETY: `context` is valid per this function's contract.
    let raw = unsafe {
        duckdb_client_context_get_config_option(
            context,
            c"autoload_known_extensions".as_ptr(),
            &raw mut scope,
        )
    };
    if raw.is_null() {
        return true;
    }
    // SAFETY: the call returns an owned value; `Value` destroys it.
    let value = unsafe { crate::value::Value::from_raw(raw) };
    value.as_bool() != Some(false)
}

crate::debug_repr::impl_handle_debug!(CatalogEntry.entry, Catalog.catalog);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_entry_type_round_trip_all_variants() {
        let variants = [
            CatalogEntryType::Invalid,
            CatalogEntryType::Table,
            CatalogEntryType::View,
            CatalogEntryType::Index,
            CatalogEntryType::Schema,
            CatalogEntryType::PreparedStatement,
            CatalogEntryType::Sequence,
            CatalogEntryType::Collation,
            CatalogEntryType::Type,
            CatalogEntryType::Database,
        ];
        for variant in variants {
            let raw = variant.to_raw();
            let back = CatalogEntryType::from_raw(raw);
            assert_eq!(variant, back, "round-trip failed for {variant:?}");
        }
    }

    #[test]
    fn only_schema_level_entry_types_are_lookup_supported() {
        use CatalogEntryType as T;
        for t in [
            T::Table,
            T::View,
            T::Index,
            T::Sequence,
            T::Collation,
            T::Type,
        ] {
            assert!(t.is_lookup_supported(), "{t:?}");
        }
        for t in [T::Schema, T::Database, T::PreparedStatement, T::Invalid] {
            assert!(!t.is_lookup_supported(), "{t:?}");
        }
    }

    #[test]
    fn autoloading_names_match_duckdb_case_insensitively() {
        assert!(CatalogEntryType::Type.may_autoload_extension("inet"));
        assert!(CatalogEntryType::Type.may_autoload_extension("JSON"));
        assert!(CatalogEntryType::Collation.may_autoload_extension("de"));
        assert!(CatalogEntryType::Collation.may_autoload_extension("EN_US"));
        assert!(CatalogEntryType::Collation.may_autoload_extension("zu"));
        assert!(!CatalogEntryType::Type.may_autoload_extension("my_type"));
        assert!(!CatalogEntryType::Type.may_autoload_extension("de"));
        assert!(!CatalogEntryType::Table.may_autoload_extension("inet"));
        assert!(!CatalogEntryType::Collation.may_autoload_extension("nocase"));
        assert_eq!(AUTOLOADING_COLLATION_NAMES.len(), 130);
    }

    #[test]
    fn lookup_of_an_unsupported_type_is_refused_without_calling_duckdb() {
        // Null handles and no live DuckDB: this only passes because `lookup`
        // returns before making any FFI call for these types.
        for t in [
            CatalogEntryType::Schema,
            CatalogEntryType::Database,
            CatalogEntryType::PreparedStatement,
            CatalogEntryType::Invalid,
        ] {
            // SAFETY: returns before touching either handle.
            let entry = unsafe {
                CatalogEntry::lookup(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    c"main",
                    c"main",
                    t,
                )
            };
            let err = entry
                .err()
                .unwrap_or_else(|| panic!("{t:?} must be refused"));
            assert!(err.as_str().contains("not supported"), "{t:?}: {err}");
        }
    }

    #[test]
    fn catalog_entry_type_unknown_raw_maps_to_invalid() {
        // Any unknown value should map to Invalid.
        let result = CatalogEntryType::from_raw(9999);
        assert_eq!(result, CatalogEntryType::Invalid);
    }

    #[test]
    fn catalog_entry_type_debug_impl() {
        let s = format!("{:?}", CatalogEntryType::View);
        assert_eq!(s, "View");
    }

    #[test]
    fn catalog_entry_type_distinct_raw_values() {
        // Ensure no two variants share the same raw value.
        let variants = [
            CatalogEntryType::Invalid,
            CatalogEntryType::Table,
            CatalogEntryType::View,
            CatalogEntryType::Index,
            CatalogEntryType::Schema,
            CatalogEntryType::PreparedStatement,
            CatalogEntryType::Sequence,
            CatalogEntryType::Collation,
            CatalogEntryType::Type,
            CatalogEntryType::Database,
        ];
        let raws: Vec<duckdb_catalog_entry_type> = variants.iter().map(|v| v.to_raw()).collect();
        // Invalid is 0; every non-Invalid variant must differ from each other.
        for (i, a) in raws.iter().enumerate().skip(1) {
            for b in raws.iter().skip(i + 1) {
                assert_ne!(a, b, "two non-Invalid variants share raw value {a}");
            }
        }
    }
}
