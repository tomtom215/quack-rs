// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

//! SPDX license identifier validation for `DuckDB` community extensions.
//!
//! Extensions must declare a recognized open-source license. This module
//! validates that the `extension.licence` field contains a commonly used
//! SPDX identifier.
//!
//! # Reference
//!
//! <https://spdx.org/licenses/>

use crate::error::ExtensionError;

/// Commonly used SPDX license identifiers accepted by the `DuckDB` community
/// extension repository.
///
/// This list covers the most common open-source licenses. Extensions using
/// a license not in this list should check the full SPDX registry at
/// <https://spdx.org/licenses/>.
pub const COMMON_SPDX_LICENSES: &[&str] = &[
    "0BSD",
    "AAL",
    "AFL-3.0",
    "AGPL-3.0-only",
    "AGPL-3.0-or-later",
    "Apache-2.0",
    "Artistic-2.0",
    "BlueOak-1.0.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CAL-1.0",
    "CAL-1.0-Combined-Work-Exception",
    "CERN-OHL-P-2.0",
    "CERN-OHL-S-2.0",
    "CERN-OHL-W-2.0",
    "CECILL-2.1",
    "ECL-2.0",
    "EFL-2.0",
    "EPL-2.0",
    "EUPL-1.2",
    "GPL-2.0-only",
    "GPL-2.0-or-later",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "ISC",
    "LGPL-2.1-only",
    "LGPL-2.1-or-later",
    "LGPL-3.0-only",
    "LGPL-3.0-or-later",
    "MIT",
    "MIT-0",
    "MPL-2.0",
    "MulanPSL-2.0",
    "NCSA",
    "OSL-3.0",
    "PostgreSQL",
    "RPL-1.5",
    "SSPL-1.0",
    "UPL-1.0",
    "Unlicense",
    "Zlib",
];

/// Validates that a license string is a recognized SPDX identifier.
///
/// Checks against the [`COMMON_SPDX_LICENSES`] list. The comparison is
/// case-sensitive per the SPDX specification.
///
/// # Errors
///
/// Returns `ExtensionError` if the license is empty or not in the recognized list.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_spdx_license;
///
/// assert!(validate_spdx_license("MIT").is_ok());
/// assert!(validate_spdx_license("Apache-2.0").is_ok());
/// assert!(validate_spdx_license("BSD-3-Clause").is_ok());
/// assert!(validate_spdx_license("FAKE-LICENSE").is_err());
/// assert!(validate_spdx_license("").is_err());
/// ```
pub fn validate_spdx_license(license: &str) -> Result<(), ExtensionError> {
    if license.is_empty() {
        return Err(ExtensionError::new("license identifier must not be empty"));
    }

    if COMMON_SPDX_LICENSES.contains(&license) {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "license '{license}' is not a recognized SPDX identifier; \
             see https://spdx.org/licenses/ for the full list"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mit_accepted() {
        assert!(validate_spdx_license("MIT").is_ok());
    }

    #[test]
    fn apache_accepted() {
        assert!(validate_spdx_license("Apache-2.0").is_ok());
    }

    #[test]
    fn bsd_3_clause_accepted() {
        assert!(validate_spdx_license("BSD-3-Clause").is_ok());
    }

    #[test]
    fn gpl_accepted() {
        assert!(validate_spdx_license("GPL-3.0-only").is_ok());
        assert!(validate_spdx_license("GPL-2.0-or-later").is_ok());
    }

    #[test]
    fn unlicense_accepted() {
        assert!(validate_spdx_license("Unlicense").is_ok());
    }

    #[test]
    fn empty_rejected() {
        let err = validate_spdx_license("").unwrap_err();
        assert!(err.as_str().contains("empty"));
    }

    #[test]
    fn unknown_license_rejected() {
        let err = validate_spdx_license("FAKE-LICENSE").unwrap_err();
        assert!(err.as_str().contains("not a recognized SPDX"));
    }

    #[test]
    fn case_sensitive() {
        // SPDX identifiers are case-sensitive
        assert!(validate_spdx_license("mit").is_err());
        assert!(validate_spdx_license("apache-2.0").is_err());
    }

    #[test]
    fn all_listed_licenses_validate() {
        for &license in COMMON_SPDX_LICENSES {
            assert!(
                validate_spdx_license(license).is_ok(),
                "expected '{license}' to be accepted"
            );
        }
    }
}
