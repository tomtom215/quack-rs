// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Extension name validation per `DuckDB` community extension rules.
//!
//! An extension name must match `^[a-z][a-z0-9_]*$` — a lowercase letter
//! followed by lowercase letters, digits and underscores.
//!
//! # Why no hyphens
//!
//! The name is not just a label. The community build names the binary
//! `<name>.duckdb_extension`, and `DuckDB`'s loader (`TryInitialLoad` and
//! `LoadExternalExtension` in `src/main/extension/extension_load.cpp`, v1.5.5)
//! takes the file's base name, lowercases it, and looks up the entry point
//! `<base name>_init_c_api` in the library. A hyphen would have to appear in
//! that C symbol, which neither C nor Rust can define — so a hyphenated
//! extension can never load, and the scaffold would generate
//! `entry_point!(my-ext_init_c_api, …)`, which does not even compile. None of
//! the 395 published community extensions uses a hyphen.
//!
//! # Names that are taken
//!
//! A well-formed name may still be unusable for a community extension.
//! [`validate_community_extension_name`] also refuses:
//!
//! - [`DUCKDB_EXTENSION_ALIASES`]: `DuckDB` rewrites these to another
//!   extension's name before `INSTALL` and `LOAD`
//!   (`ExtensionHelper::ApplyExtensionAlias`), so `INSTALL s3 FROM community`
//!   downloads `httpfs` — an extension named `s3` can never be installed or
//!   loaded by its name;
//! - [`DUCKDB_CORE_EXTENSION_NAMES`]: the community-extensions FAQ says "if a
//!   core `DuckDB` extension of the same name exists or is created, the
//!   community extension of the same name has to be renamed".
//!
//! Sources, read 2026-10-30: `src/main/extension/extension_alias.cpp`,
//! `internal_extensions[]` in `src/main/extension/extension_helper.cpp` and
//! `AUTOLOADABLE_EXTENSIONS` in `src/include/duckdb/main/extension_entries.hpp`
//! at `DuckDB` v1.4.4, v1.5.6 and v2.0.0 (`main` had the same aliases), and the
//! core-extension table of `docs/2.0/core_extensions/overview.md` in
//! `duckdb/duckdb-web`. `scripts/check-extension-names.py` fetches them again
//! in CI and fails when they and these lists disagree. None of the 395
//! community extensions in `duckdb/community-extensions` (`845779e`) has a name
//! listed here.
//!
//! Uniqueness among community extensions is not checked: that is the
//! author's to look up (`extensions/` in `duckdb/community-extensions`).
//!
//! # Reference
//!
//! <https://duckdb.org/community_extensions/development>,
//! <https://duckdb.org/community_extensions/faq>

use crate::error::ExtensionError;

/// `DuckDB`'s built-in extension aliases, as `(alias, extension)`.
///
/// `INSTALL` and `LOAD` replace the alias with the extension's name, so a
/// community extension called by an alias can never be installed or loaded.
///
/// `src/main/extension/extension_alias.cpp` (`internal_aliases`) at `DuckDB`
/// v2.0.0 and `main`; v1.4.4 has the same list without `odbc`.
pub const DUCKDB_EXTENSION_ALIASES: &[(&str, &str)] = &[
    ("http", "httpfs"),
    ("https", "httpfs"),
    ("md", "motherduck"),
    ("mysql", "mysql_scanner"),
    ("odbc", "odbc_scanner"),
    ("postgres", "postgres_scanner"),
    ("s3", "httpfs"),
    ("sqlite", "sqlite_scanner"),
    ("sqlite3", "sqlite_scanner"),
    ("uc_catalog", "unity_catalog"),
];

/// The names of `DuckDB`'s core extensions, current and since `DuckDB` v1.4.4,
/// which a community extension may not take (the community-extensions FAQ: it
/// "has to be renamed").
///
/// The union of `internal_extensions[]` (`extension_helper.cpp`) and
/// `AUTOLOADABLE_EXTENSIONS` (`extension_entries.hpp`) at v1.4.4, v1.5.6 and
/// v2.0.0, and the core-extension table of the `DuckDB` 2.0 documentation,
/// without the [aliases][DUCKDB_EXTENSION_ALIASES] (`mysql` is the alias, the
/// extension is `mysql_scanner`). `jemalloc` was a core extension until
/// v1.4.x; `sqlsmith` is autoloadable; `sirius` is documented as "coming
/// soon". Sorted.
pub const DUCKDB_CORE_EXTENSION_NAMES: &[&str] = &[
    "autocomplete",
    "avro",
    "aws",
    "azure",
    "core_functions",
    "delta",
    "ducklake",
    "encodings",
    "excel",
    "fts",
    "httpfs",
    "iceberg",
    "icu",
    "inet",
    "jemalloc",
    "json",
    "lance",
    "legacy_parser",
    "motherduck",
    "mysql_scanner",
    "odbc_scanner",
    "parquet",
    "postgres_scanner",
    "quack",
    "sirius",
    "spatial",
    "sqlite_scanner",
    "sqlsmith",
    "tpcds",
    "tpch",
    "ui",
    "unity_catalog",
    "vortex",
    "vss",
];

/// Every name a community extension may not take: the
/// [aliases][DUCKDB_EXTENSION_ALIASES] and the
/// [core extensions][DUCKDB_CORE_EXTENSION_NAMES]. Sorted.
pub const DUCKDB_RESERVED_EXTENSION_NAMES: &[&str] = &[
    "autocomplete",
    "avro",
    "aws",
    "azure",
    "core_functions",
    "delta",
    "ducklake",
    "encodings",
    "excel",
    "fts",
    "http",
    "httpfs",
    "https",
    "iceberg",
    "icu",
    "inet",
    "jemalloc",
    "json",
    "lance",
    "legacy_parser",
    "md",
    "motherduck",
    "mysql",
    "mysql_scanner",
    "odbc",
    "odbc_scanner",
    "parquet",
    "postgres",
    "postgres_scanner",
    "quack",
    "s3",
    "sirius",
    "spatial",
    "sqlite",
    "sqlite3",
    "sqlite_scanner",
    "sqlsmith",
    "tpcds",
    "tpch",
    "uc_catalog",
    "ui",
    "unity_catalog",
    "vortex",
    "vss",
];

/// Validates a name for a **community** extension.
///
/// The name must be well formed ([`validate_extension_name`]) and not taken by
/// `DuckDB` itself: neither one of its [aliases][DUCKDB_EXTENSION_ALIASES] nor
/// a [core extension's name][DUCKDB_CORE_EXTENSION_NAMES]. The scaffold and
/// `parse_description_yml` use this.
///
/// # Errors
///
/// Returns `ExtensionError` for a malformed name, or one `DuckDB` reserves,
/// saying why.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::extension_name::validate_community_extension_name;
///
/// assert!(validate_community_extension_name("my_s3_tools").is_ok());
/// assert!(validate_community_extension_name("s3").is_err()); // DuckDB means httpfs
/// assert!(validate_community_extension_name("json").is_err()); // a core extension
/// ```
pub fn validate_community_extension_name(name: &str) -> Result<(), ExtensionError> {
    validate_extension_name(name)?;
    if let Some(&(alias, target)) = DUCKDB_EXTENSION_ALIASES.iter().find(|(a, _)| *a == name) {
        return Err(ExtensionError::new(format!(
            "extension name '{alias}' is a DuckDB alias for the core extension '{target}': \
             DuckDB replaces it with '{target}' before INSTALL and LOAD, so an extension of \
             this name could never be installed or loaded; choose another name, for example \
             with your vendor as a prefix ('myorg_{alias}')"
        )));
    }
    if DUCKDB_CORE_EXTENSION_NAMES.contains(&name) {
        return Err(ExtensionError::new(format!(
            "extension name '{name}' is the name of a DuckDB core extension, and the \
             community-extensions FAQ says a community extension with the name of a core \
             extension \"has to be renamed to avoid confusion\"; choose another name, for \
             example with your vendor as a prefix ('myorg_{name}')"
        )));
    }
    Ok(())
}

/// Validates the form of a `DuckDB` extension name.
///
/// This is whether `DuckDB` can load a file of this name. Whether a community
/// extension may *take* the name is [`validate_community_extension_name`],
/// which also refuses the names `DuckDB` itself uses.
///
/// # Rules
///
/// - Must not be empty
/// - Must contain only lowercase ASCII letters (`a-z`), digits (`0-9`), or
///   underscores (`_`) — **no hyphens**: `DuckDB` derives the entry-point
///   symbol `<name>_init_c_api` from the name (see the [module
///   docs][crate::validate::extension_name])
/// - Must start with a letter
/// - Must not exceed 64 characters
///
/// # Errors
///
/// Returns `ExtensionError` describing the first rule violation found.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_extension_name;
///
/// assert!(validate_extension_name("my_extension").is_ok());
/// assert!(validate_extension_name("my_ext_2").is_ok());
/// assert!(validate_extension_name("my-ext").is_err()); // no C symbol can hold '-'
/// assert!(validate_extension_name("").is_err());
/// assert!(validate_extension_name("MyExtension").is_err());
/// assert!(validate_extension_name("my extension").is_err());
/// assert!(validate_extension_name("123start").is_err());
/// ```
pub fn validate_extension_name(name: &str) -> Result<(), ExtensionError> {
    if name.is_empty() {
        return Err(ExtensionError::new("extension name must not be empty"));
    }

    if name.len() > 64 {
        return Err(ExtensionError::new(format!(
            "extension name must not exceed 64 characters, got {}",
            name.len()
        )));
    }

    if !name.as_bytes()[0].is_ascii_lowercase() {
        return Err(ExtensionError::new(format!(
            "extension name must start with a lowercase letter, got '{}'",
            name.chars().next().unwrap_or('?')
        )));
    }

    for (i, ch) in name.chars().enumerate() {
        if ch == '-' {
            return Err(ExtensionError::new(format!(
                "extension name must not contain '-' (position {i}): DuckDB loads the \
                 extension by calling `<name>_init_c_api`, and no C or Rust symbol can \
                 contain a hyphen; use '_' instead ('{}')",
                name.replace('-', "_")
            )));
        }
        if !matches!(ch, 'a'..='z' | '0'..='9' | '_') {
            return Err(ExtensionError::new(format!(
                "extension name contains invalid character '{ch}' at position {i}; \
                 only lowercase letters, digits, and underscores are allowed"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple_name() {
        assert!(validate_extension_name("myext").is_ok());
    }

    #[test]
    fn valid_with_underscore() {
        assert!(validate_extension_name("my_extension").is_ok());
    }

    /// `DuckDB` looks up the entry point as `<file base name>_init_c_api`, and
    /// the community build names the file after `extension.name`, so a hyphen
    /// would require a symbol no C or Rust compiler can emit.
    #[test]
    fn hyphen_rejected() {
        for name in ["my-extension", "my-ext_v2", "a-"] {
            let err = validate_extension_name(name).unwrap_err();
            assert!(err.as_str().contains("'-'"), "{name}: {err}");
            assert!(err.as_str().contains("_init_c_api"), "{name}: {err}");
        }
    }

    #[test]
    fn valid_with_digits() {
        assert!(validate_extension_name("ext2go").is_ok());
    }

    #[test]
    fn valid_mixed() {
        assert!(validate_extension_name("my_ext_v2").is_ok());
    }

    #[test]
    fn empty_name_rejected() {
        let err = validate_extension_name("").unwrap_err();
        assert!(err.as_str().contains("empty"));
    }

    #[test]
    fn uppercase_start_rejected() {
        let err = validate_extension_name("MyExtension").unwrap_err();
        assert!(err.as_str().contains("start with a lowercase letter"));
    }

    #[test]
    fn uppercase_mid_rejected() {
        let err = validate_extension_name("myExtension").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn space_rejected() {
        let err = validate_extension_name("my extension").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn starts_with_digit_rejected() {
        let err = validate_extension_name("123ext").unwrap_err();
        assert!(err.as_str().contains("start with a lowercase letter"));
    }

    #[test]
    fn starts_with_hyphen_rejected() {
        let err = validate_extension_name("-ext").unwrap_err();
        assert!(err.as_str().contains("start with a lowercase letter"));
    }

    #[test]
    fn special_chars_rejected() {
        let err = validate_extension_name("my@ext").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn too_long_rejected() {
        let long_name: String = "a".repeat(65);
        let err = validate_extension_name(&long_name).unwrap_err();
        assert!(err.as_str().contains("64 characters"));
    }

    #[test]
    fn max_length_accepted() {
        let max_name: String = "a".repeat(64);
        assert!(validate_extension_name(&max_name).is_ok());
    }

    /// `INSTALL s3 FROM community` on `DuckDB` 2.0.0 fails with `Failed to
    /// download extension "httpfs" ... (HTTP 404)` (reviewer `conform`): the
    /// alias is applied before the repository is asked.
    #[test]
    fn an_alias_is_refused_and_names_the_extension_it_stands_for() {
        for (alias, target) in [
            ("s3", "httpfs"),
            ("http", "httpfs"),
            ("https", "httpfs"),
            ("md", "motherduck"),
            ("mysql", "mysql_scanner"),
            ("odbc", "odbc_scanner"),
            ("postgres", "postgres_scanner"),
            ("sqlite", "sqlite_scanner"),
            ("sqlite3", "sqlite_scanner"),
            ("uc_catalog", "unity_catalog"),
        ] {
            let err = validate_community_extension_name(alias).expect_err(alias);
            assert!(err.as_str().contains("alias"), "{alias}: {err}");
            assert!(
                err.as_str().contains(&format!("'{target}'")),
                "{alias}: {err}"
            );
            // The name itself is well formed: DuckDB could load a file of it.
            assert!(validate_extension_name(alias).is_ok());
        }
    }

    #[test]
    fn a_core_extension_name_is_refused_with_the_faqs_reason() {
        for name in [
            "json",
            "parquet",
            "httpfs",
            "icu",
            "quack",
            "spatial",
            "core_functions",
            "jemalloc",
            "mysql_scanner",
            "sqlsmith",
            "legacy_parser",
        ] {
            let err = validate_community_extension_name(name).expect_err(name);
            assert!(err.as_str().contains("has to be renamed"), "{name}: {err}");
            assert!(validate_extension_name(name).is_ok());
        }
    }

    /// Only the exact name is taken: 18 of the 395 published community
    /// extensions build on a core name (`cache_httpfs`, `rusty_quack`,
    /// `json_schema`, `ducklake_cdc`, `http_client`, ...).
    #[test]
    fn a_name_built_on_a_reserved_one_is_fine() {
        for name in [
            "cache_httpfs",
            "rusty_quack",
            "json_schema",
            "ducklake_cdc",
            "http_client",
            "my_s3_tools",
            "s3x",
        ] {
            assert!(validate_community_extension_name(name).is_ok(), "{name}");
        }
        assert!(validate_community_extension_name("Bad-Name").is_err());
    }

    #[test]
    fn the_reserved_list_is_exactly_the_aliases_and_core_names_sorted() {
        let mut union: Vec<&str> = DUCKDB_CORE_EXTENSION_NAMES
            .iter()
            .copied()
            .chain(DUCKDB_EXTENSION_ALIASES.iter().map(|(alias, _)| *alias))
            .collect();
        union.sort_unstable();
        union.dedup();
        assert_eq!(union, DUCKDB_RESERVED_EXTENSION_NAMES);
        for list in [DUCKDB_CORE_EXTENSION_NAMES, DUCKDB_RESERVED_EXTENSION_NAMES] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted, list, "keep the list sorted and unique");
        }
        // Every alias points at a core extension, and none is one itself.
        for (alias, target) in DUCKDB_EXTENSION_ALIASES {
            assert!(DUCKDB_CORE_EXTENSION_NAMES.contains(target), "{target}");
            assert!(!DUCKDB_CORE_EXTENSION_NAMES.contains(alias), "{alias}");
        }
        for name in DUCKDB_RESERVED_EXTENSION_NAMES {
            assert!(validate_extension_name(name).is_ok(), "{name}");
        }
    }
}
