// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Release profile validation for `DuckDB` loadable extensions.
//!
//! `DuckDB` loadable extensions are shared libraries (`.so`/`.dylib`/`.dll`) that must
//! use specific Cargo release profile settings to produce correct, lean binaries.
//!
//! # Required settings
//!
//! ```toml
//! [profile.release]
//! panic = "abort"    # Required: panics across FFI are UB
//! lto = true         # Recommended: reduces binary size
//! opt-level = 3      # Recommended: maximum optimization
//! ```

use crate::error::ExtensionError;

/// Results of checking a release profile configuration.
///
/// Each field indicates whether the corresponding setting is present and correct.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseProfileCheck {
    /// `panic = "abort"` is set (required for FFI safety).
    pub panic_abort: bool,
    /// `lto = true` or `lto = "fat"` is set (recommended for binary size).
    pub lto_enabled: bool,
    /// `opt-level = 3` is set (recommended for performance).
    pub opt_level_3: bool,
    /// `codegen-units = 1` is set (recommended for optimization quality).
    pub codegen_units_1: bool,
}

impl ReleaseProfileCheck {
    /// Returns `true` if all required settings are satisfied.
    ///
    /// Currently only `panic = "abort"` is strictly required; the other
    /// settings are best practices.
    #[must_use]
    pub const fn is_required_satisfied(&self) -> bool {
        self.panic_abort
    }

    /// Returns `true` if all recommended settings are satisfied.
    #[must_use]
    pub const fn is_fully_optimized(&self) -> bool {
        self.panic_abort && self.lto_enabled && self.opt_level_3 && self.codegen_units_1
    }
}

/// Validates release profile settings from string key-value pairs.
///
/// This function checks whether the given settings match the recommended
/// release profile for `DuckDB` loadable extensions.
///
/// # Arguments
///
/// - `panic`: The value of `panic` (e.g., `"abort"`, `"unwind"`)
/// - `lto`: The value of `lto` (e.g., `"true"`, `"false"`, `"fat"`, `"thin"`)
/// - `opt_level`: The value of `opt-level` (e.g., `"3"`, `"2"`, `"s"`)
/// - `codegen_units`: The value of `codegen-units` (e.g., `"1"`, `"16"`)
///
/// # Errors
///
/// Returns `ExtensionError` if the required setting (`panic = "abort"`) is not met.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_release_profile;
///
/// let check = validate_release_profile("abort", "true", "3", "1").unwrap();
/// assert!(check.is_fully_optimized());
///
/// // Missing panic=abort fails
/// assert!(validate_release_profile("unwind", "true", "3", "1").is_err());
/// ```
pub fn validate_release_profile(
    panic: &str,
    lto: &str,
    opt_level: &str,
    codegen_units: &str,
) -> Result<ReleaseProfileCheck, ExtensionError> {
    let check = ReleaseProfileCheck {
        panic_abort: panic == "abort",
        lto_enabled: matches!(lto, "true" | "fat"),
        opt_level_3: opt_level == "3",
        codegen_units_1: codegen_units == "1",
    };

    if !check.panic_abort {
        return Err(ExtensionError::new(
            "release profile must set panic = \"abort\"; \
             panics across FFI boundaries are undefined behavior in Rust",
        ));
    }

    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_optimized() {
        let check = validate_release_profile("abort", "true", "3", "1").unwrap();
        assert!(check.is_fully_optimized());
        assert!(check.is_required_satisfied());
    }

    #[test]
    fn fat_lto_accepted() {
        let check = validate_release_profile("abort", "fat", "3", "1").unwrap();
        assert!(check.lto_enabled);
    }

    #[test]
    fn thin_lto_not_full() {
        let check = validate_release_profile("abort", "thin", "3", "1").unwrap();
        assert!(!check.lto_enabled);
        assert!(!check.is_fully_optimized());
    }

    #[test]
    fn no_lto_still_passes_required() {
        let check = validate_release_profile("abort", "false", "2", "16").unwrap();
        assert!(check.is_required_satisfied());
        assert!(!check.is_fully_optimized());
    }

    #[test]
    fn panic_unwind_rejected() {
        let err = validate_release_profile("unwind", "true", "3", "1").unwrap_err();
        assert!(err.as_str().contains("panic"));
        assert!(err.as_str().contains("abort"));
    }

    #[test]
    fn empty_panic_rejected() {
        assert!(validate_release_profile("", "true", "3", "1").is_err());
    }

    #[test]
    fn check_fields_independent() {
        let check = validate_release_profile("abort", "false", "2", "4").unwrap();
        assert!(check.panic_abort);
        assert!(!check.lto_enabled);
        assert!(!check.opt_level_3);
        assert!(!check.codegen_units_1);
    }
}
