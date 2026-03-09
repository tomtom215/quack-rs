// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Validation of `DuckDB` community extension `description.yml` files.
//!
//! Every extension submitted to the `DuckDB` community extensions repository must
//! include a `description.yml` metadata file. This module provides:
//!
//! - [`DescriptionYml`] — a structured representation of the file's required fields
//! - [`parse_description_yml`] — parse from a `description.yml` string
//! - [`validate_description_yml_str`] — validate a `description.yml` string end-to-end
//!
//! # `description.yml` Format
//!
//! ```yaml
//! extension:
//!   name: my_ext
//!   description: A one-line description of the extension.
//!   version: 0.1.0
//!   language: Rust
//!   build: cargo
//!   license: MIT
//!   requires_toolchains: rust;python3
//!   excluded_platforms: "wasm_mvp;wasm_eh;wasm_threads"   # optional
//!   maintainers:
//!     - Jane Doe
//!
//! repo:
//!   github: janedoe/duckdb-my-ext
//!   ref: main
//! ```
//!
//! # Required vs Optional Fields
//!
//! | Field | Required | Rules |
//! |-------|----------|-------|
//! | `extension.name` | Yes | Must pass [`validate_extension_name`] |
//! | `extension.description` | Yes | Non-empty |
//! | `extension.version` | Yes | Must pass [`validate_extension_version`] |
//! | `extension.language` | Yes | Must be `"Rust"` for Rust extensions |
//! | `extension.build` | Yes | Must be `"cargo"` for Rust extensions |
//! | `extension.license` | Yes | Must pass [`validate_spdx_license`] |
//! | `extension.requires_toolchains` | Yes | Semi-colon list including `"rust"` |
//! | `extension.excluded_platforms` | No | Must pass [`validate_excluded_platforms_str`] |
//! | `extension.maintainers` | Yes | At least one maintainer |
//! | `repo.github` | Yes | Non-empty `owner/repo` format |
//! | `repo.ref` | Yes | Non-empty git ref (branch, tag, or commit) |
//!
//! # Reference
//!
//! - <https://duckdb.org/community_extensions/documentation>
//! - <https://duckdb.org/community_extensions/development>
//!
//! # Example
//!
//! ```rust
//! use quack_rs::validate::description_yml::validate_description_yml_str;
//!
//! let yml = r#"
//! extension:
//!   name: my_ext
//!   description: Fast analytics for DuckDB.
//!   version: 0.1.0
//!   language: Rust
//!   build: cargo
//!   license: MIT
//!   requires_toolchains: rust;python3
//!   maintainers:
//!     - Jane Doe
//!
//! repo:
//!   github: janedoe/duckdb-my-ext
//!   ref: main
//! "#;
//!
//! assert!(validate_description_yml_str(yml).is_ok());
//! ```
//!
//! [`validate_extension_name`]: crate::validate::validate_extension_name
//! [`validate_extension_version`]: crate::validate::validate_extension_version
//! [`validate_spdx_license`]: crate::validate::validate_spdx_license
//! [`validate_excluded_platforms_str`]: crate::validate::validate_excluded_platforms_str

mod model;
mod parser;
mod validator;

pub use model::DescriptionYml;
pub use parser::parse_description_yml;
pub use validator::{validate_description_yml_str, validate_rust_extension};

#[cfg(test)]
mod tests;
