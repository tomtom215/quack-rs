// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Release profile validation for `DuckDB` loadable extensions.
//!
//! `DuckDB` loadable extensions are shared libraries (`.so`/`.dylib`/`.dll`)
//! whose Cargo release profile decides whether a panic in your code becomes a
//! `DuckDB` error or kills the `DuckDB` process.
//!
//! # Required settings
//!
//! ```toml
//! [profile.release]
//! panic = "unwind"    # Required: quack-rs catches panics, and catching needs unwinding
//! lto = true          # Recommended: reduces binary size
//! opt-level = 3       # Recommended: maximum optimization
//! codegen-units = 1   # Recommended: better optimization
//! ```
//!
//! # Why `unwind`, not `abort`
//!
//! quack-rs wraps every `extern "C"` entry point — the extension entry point,
//! every scalar/table/aggregate/cast/copy callback macro — in
//! [`std::panic::catch_unwind`], turning a panic into a `DuckDB` error message
//! instead of a crash. **`catch_unwind` cannot catch anything under
//! `panic = "abort"`**: the runtime aborts before unwinding starts, so the
//! process dies with `SIGABRT` and takes the user's `DuckDB` session with it.
//!
//! Demonstrated directly:
//!
//! ```text
//! $ rustc -O panic_probe.rs && ./panic_probe
//! catch_unwind returned: true
//! process survived the panic          exit=0
//!
//! $ rustc -O -C panic=abort panic_probe.rs && ./panic_probe
//! Aborted                             exit=134
//! ```
//!
//! The older advice to set `panic = "abort"` came from panics escaping an
//! `extern "C"` boundary once being undefined behaviour. They no longer are —
//! Rust defines that as an abort — and quack-rs catches them before the
//! boundary anyway, which is the whole point.

use crate::error::ExtensionError;

/// Results of checking a release profile configuration.
///
/// Each field indicates whether the corresponding setting is present and correct.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseProfileCheck {
    /// `panic = "unwind"` is set (required for quack-rs's panic safety).
    pub panic_unwind: bool,
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
    /// Only `panic = "unwind"` is strictly required; the rest are best
    /// practices.
    #[must_use]
    pub const fn is_required_satisfied(&self) -> bool {
        self.panic_unwind
    }

    /// Returns `true` if all recommended settings are satisfied.
    #[must_use]
    pub const fn is_fully_optimized(&self) -> bool {
        self.panic_unwind && self.lto_enabled && self.opt_level_3 && self.codegen_units_1
    }
}

/// Validates release profile settings from string key-value pairs.
///
/// This function checks whether the given settings match the recommended
/// release profile for `DuckDB` loadable extensions.
///
/// # Arguments
///
/// - `panic`: The value of `panic` (e.g., `"unwind"`, `"abort"`)
/// - `lto`: The value of `lto` (e.g., `"true"`, `"false"`, `"fat"`, `"thin"`)
/// - `opt_level`: The value of `opt-level` (e.g., `"3"`, `"2"`, `"s"`)
/// - `codegen_units`: The value of `codegen-units` (e.g., `"1"`, `"16"`)
///
/// # Errors
///
/// Returns `ExtensionError` if the required setting (`panic = "unwind"`) is not
/// met.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_release_profile;
///
/// let check = validate_release_profile("unwind", "true", "3", "1").unwrap();
/// assert!(check.is_fully_optimized());
///
/// // `abort` makes quack-rs's panic handling inert — see the module docs.
/// assert!(validate_release_profile("abort", "true", "3", "1").is_err());
/// ```
pub fn validate_release_profile(
    panic: &str,
    lto: &str,
    opt_level: &str,
    codegen_units: &str,
) -> Result<ReleaseProfileCheck, ExtensionError> {
    let check = ReleaseProfileCheck {
        // An unset `panic` defaults to unwind, which is what we want; but an
        // extension's profile should say so explicitly, so only the literal
        // "unwind" counts here.
        panic_unwind: panic == "unwind",
        lto_enabled: matches!(lto, "true" | "fat"),
        opt_level_3: opt_level == "3",
        codegen_units_1: codegen_units == "1",
    };

    if !check.panic_unwind {
        return Err(ExtensionError::new(format!(
            "release profile sets panic = \"{panic}\"; it must be \"unwind\". \
             quack-rs catches panics at every FFI boundary and turns them into \
             DuckDB errors, and std::panic::catch_unwind cannot catch anything \
             under panic = \"abort\" — the process aborts instead, taking the \
             user's DuckDB session with it"
        )));
    }

    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_optimized() {
        let check = validate_release_profile("unwind", "true", "3", "1").unwrap();
        assert!(check.is_fully_optimized());
        assert!(check.is_required_satisfied());
    }

    #[test]
    fn fat_lto_accepted() {
        let check = validate_release_profile("unwind", "fat", "3", "1").unwrap();
        assert!(check.lto_enabled);
    }

    #[test]
    fn thin_lto_not_full() {
        let check = validate_release_profile("unwind", "thin", "3", "1").unwrap();
        assert!(!check.lto_enabled);
        assert!(!check.is_fully_optimized());
    }

    #[test]
    fn no_lto_still_passes_required() {
        let check = validate_release_profile("unwind", "false", "2", "16").unwrap();
        assert!(check.is_required_satisfied());
        assert!(!check.is_fully_optimized());
    }

    #[test]
    fn panic_abort_rejected_because_it_disables_catch_unwind() {
        let err = validate_release_profile("abort", "true", "3", "1").unwrap_err();
        assert!(err.as_str().contains("catch_unwind"));
        assert!(err.as_str().contains("must be"));
    }

    #[test]
    fn empty_panic_rejected() {
        // An unset `panic` does default to unwind, but an extension's profile
        // should say so — the setting is load-bearing enough to be explicit.
        assert!(validate_release_profile("", "true", "3", "1").is_err());
    }

    #[test]
    fn check_fields_independent() {
        let check = validate_release_profile("unwind", "false", "2", "4").unwrap();
        assert!(check.panic_unwind);
        assert!(!check.lto_enabled);
        assert!(!check.opt_level_3);
        assert!(!check.codegen_units_1);
    }

    #[test]
    fn the_scaffold_and_the_validator_agree() {
        // These two contradicted each other: the scaffold generated
        // `panic = "unwind"` (correct) while this validator required "abort".
        let cargo_toml = crate::scaffold::generate_scaffold(&crate::scaffold::ScaffoldConfig {
            name: "probe".into(),
            description: "d".into(),
            maintainer: "m".into(),
            github_repo: "o/r".into(),
            ..Default::default()
        })
        .expect("scaffold");
        let manifest = &cargo_toml
            .iter()
            .find(|f| f.path == "Cargo.toml")
            .expect("Cargo.toml")
            .content;
        assert!(
            manifest.contains(r#"panic = "unwind""#),
            "the scaffold must generate the profile this validator requires"
        );
        assert!(!manifest.contains(r#"panic = "abort""#));
    }
}
