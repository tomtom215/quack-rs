//! Semantic versioning validation for `DuckDB` community extensions.
//!
//! Extensions submitted to the `DuckDB` community repository must use
//! valid semantic versioning for the `extension.version` field.
//!
//! # Reference
//!
//! <https://semver.org/>

use crate::error::ExtensionError;

/// Validates that a version string is valid semantic versioning.
///
/// Accepts versions in the form `MAJOR.MINOR.PATCH` with optional
/// pre-release (`-alpha.1`) and build metadata (`+build.123`) suffixes.
///
/// # Rules
///
/// - Must have exactly three numeric components separated by dots
/// - Components must not have leading zeros (except `0` itself)
/// - Pre-release identifiers are alphanumeric with dots/hyphens
/// - Build metadata follows a `+` and is alphanumeric with dots/hyphens
///
/// # Errors
///
/// Returns `ExtensionError` if the version is not valid semver.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_semver;
///
/// assert!(validate_semver("1.0.0").is_ok());
/// assert!(validate_semver("0.1.0").is_ok());
/// assert!(validate_semver("1.2.3-alpha.1").is_ok());
/// assert!(validate_semver("1.2.3+build.456").is_ok());
/// assert!(validate_semver("1.2.3-rc.1+build.456").is_ok());
/// assert!(validate_semver("1.2").is_err());
/// assert!(validate_semver("v1.0.0").is_err());
/// assert!(validate_semver("01.0.0").is_err());
/// ```
pub fn validate_semver(version: &str) -> Result<(), ExtensionError> {
    if version.is_empty() {
        return Err(ExtensionError::new("version must not be empty"));
    }

    // Split off build metadata first (after +)
    let (version_pre, _build) = match version.split_once('+') {
        Some((v, b)) => {
            validate_identifiers(b, "build metadata")?;
            (v, Some(b))
        }
        None => (version, None),
    };

    // Split off pre-release (after -)
    let (core, _pre) = match version_pre.split_once('-') {
        Some((c, p)) => {
            validate_identifiers(p, "pre-release")?;
            (c, Some(p))
        }
        None => (version_pre, None),
    };

    // Parse core version: MAJOR.MINOR.PATCH
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(ExtensionError::new(format!(
            "version '{version}' must have exactly three components (MAJOR.MINOR.PATCH), got {}",
            parts.len()
        )));
    }

    for (i, &part) in parts.iter().enumerate() {
        let label = ["major", "minor", "patch"][i];
        validate_numeric_component(part, label, version)?;
    }

    Ok(())
}

/// Validates a single numeric version component (no leading zeros).
fn validate_numeric_component(
    s: &str,
    label: &str,
    full_version: &str,
) -> Result<(), ExtensionError> {
    if s.is_empty() {
        return Err(ExtensionError::new(format!(
            "version '{full_version}': {label} component is empty"
        )));
    }

    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ExtensionError::new(format!(
            "version '{full_version}': {label} component '{s}' is not a valid number"
        )));
    }

    // No leading zeros (except "0" itself)
    if s.len() > 1 && s.starts_with('0') {
        return Err(ExtensionError::new(format!(
            "version '{full_version}': {label} component '{s}' has a leading zero"
        )));
    }

    Ok(())
}

/// Validates dot-separated identifiers (pre-release or build metadata).
fn validate_identifiers(s: &str, label: &str) -> Result<(), ExtensionError> {
    if s.is_empty() {
        return Err(ExtensionError::new(format!(
            "{label} identifier must not be empty"
        )));
    }

    for ident in s.split('.') {
        if ident.is_empty() {
            return Err(ExtensionError::new(format!(
                "{label} contains an empty identifier"
            )));
        }
        if !ident
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(ExtensionError::new(format!(
                "{label} identifier '{ident}' contains invalid characters"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple() {
        assert!(validate_semver("1.0.0").is_ok());
        assert!(validate_semver("0.1.0").is_ok());
        assert!(validate_semver("0.0.1").is_ok());
        assert!(validate_semver("123.456.789").is_ok());
    }

    #[test]
    fn valid_prerelease() {
        assert!(validate_semver("1.0.0-alpha").is_ok());
        assert!(validate_semver("1.0.0-alpha.1").is_ok());
        assert!(validate_semver("1.0.0-0.3.7").is_ok());
        assert!(validate_semver("1.0.0-x.7.z.92").is_ok());
        assert!(validate_semver("1.0.0-rc-1").is_ok());
    }

    #[test]
    fn valid_build_metadata() {
        assert!(validate_semver("1.0.0+build").is_ok());
        assert!(validate_semver("1.0.0+build.456").is_ok());
        assert!(validate_semver("1.0.0+20130313144700").is_ok());
    }

    #[test]
    fn valid_prerelease_and_build() {
        assert!(validate_semver("1.0.0-alpha+001").is_ok());
        assert!(validate_semver("1.0.0-rc.1+build.456").is_ok());
    }

    #[test]
    fn empty_rejected() {
        assert!(validate_semver("").is_err());
    }

    #[test]
    fn two_components_rejected() {
        let err = validate_semver("1.2").unwrap_err();
        assert!(err.as_str().contains("three components"));
    }

    #[test]
    fn four_components_rejected() {
        let err = validate_semver("1.2.3.4").unwrap_err();
        assert!(err.as_str().contains("three components"));
    }

    #[test]
    fn leading_v_rejected() {
        let err = validate_semver("v1.0.0").unwrap_err();
        assert!(err.as_str().contains("not a valid number"));
    }

    #[test]
    fn leading_zero_rejected() {
        assert!(validate_semver("01.0.0").is_err());
        assert!(validate_semver("1.01.0").is_err());
        assert!(validate_semver("1.0.01").is_err());
    }

    #[test]
    fn leading_zero_on_zero_itself_accepted() {
        assert!(validate_semver("0.0.0").is_ok());
    }

    #[test]
    fn non_numeric_rejected() {
        assert!(validate_semver("a.b.c").is_err());
        assert!(validate_semver("1.0.x").is_err());
    }

    #[test]
    fn empty_component_rejected() {
        assert!(validate_semver("1..0").is_err());
        assert!(validate_semver(".1.0").is_err());
    }

    #[test]
    fn single_number_rejected() {
        assert!(validate_semver("1").is_err());
    }
}
