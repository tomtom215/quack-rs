//! Validation utilities for `DuckDB` community extension compliance.
//!
//! This module provides compile-time and runtime validators that enforce the
//! [`DuckDB` Community Extension](https://duckdb.org/community_extensions/development)
//! requirements. Extensions that pass these checks are ready for submission to the
//! `DuckDB` community extensions repository.
//!
//! # What is validated
//!
//! - **Extension name**: Must be lowercase alphanumeric with hyphens/underscores only
//! - **Function name**: Must be lowercase alphanumeric with underscores only (SQL-safe)
//! - **Semantic versioning**: Extension version must be valid semver
//! - **SPDX license**: Must be a recognized SPDX license identifier
//! - **`description.yml` structure**: All required fields present and well-formed
//! - **Platform targets**: Exclusions must name valid `DuckDB` build targets
//! - **Cargo.toml**: Release profile settings for loadable extensions
//!
//! # Example
//!
//! ```rust
//! use quack_rs::validate::{
//!     validate_extension_name, validate_extension_version,
//!     validate_semver, validate_spdx_license,
//! };
//!
//! assert!(validate_extension_name("my_extension").is_ok());
//! assert!(validate_extension_name("MyExt").is_err()); // uppercase not allowed
//!
//! assert!(validate_semver("1.0.0").is_ok());
//! assert!(validate_semver("not-a-version").is_err());
//!
//! // Extension versions accept both semver and git hashes (unstable)
//! assert!(validate_extension_version("0.1.0").is_ok());
//! assert!(validate_extension_version("690bfc5").is_ok());
//!
//! assert!(validate_spdx_license("MIT").is_ok());
//! assert!(validate_spdx_license("FAKE-LICENSE").is_err());
//! ```

pub mod extension_name;
pub mod function_name;
pub mod platform;
pub mod release_profile;
pub mod semver;
pub mod spdx;

pub use extension_name::validate_extension_name;
pub use function_name::validate_function_name;
pub use platform::{validate_platform, DUCKDB_PLATFORMS};
pub use release_profile::{validate_release_profile, ReleaseProfileCheck};
pub use semver::{validate_extension_version, validate_semver};
pub use spdx::validate_spdx_license;
