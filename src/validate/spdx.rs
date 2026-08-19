// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! SPDX license identifier validation for `DuckDB` community extensions.
//!
//! Extensions must declare a recognized open-source license. This module
//! validates that the `extension.license` field contains a commonly used
//! SPDX identifier.
//!
//! # Reference
//!
//! <https://spdx.org/licenses/>

use crate::error::ExtensionError;

/// Commonly used SPDX license identifiers.
///
/// This is a **curated shortlist, not the SPDX registry** — the registry has
/// over 700 entries, and a license absent from this list is very often still
/// perfectly valid. [`validate_spdx_license`] says so rather than claiming the
/// identifier does not exist.
///
/// Every entry is checked against the official registry by
/// `scripts/check-spdx-list.py`, which fails CI on a typo or on an identifier
/// SPDX has deprecated.
///
/// Sorted, and kept sorted, so additions are easy to review.
///
/// # A note on `SSPL-1.0`
///
/// It is a real SPDX identifier and is listed here, but it is **not
/// OSI-approved** — it is source-available rather than open source. If your
/// extension is bound by a policy that requires an OSI-approved license, this
/// list is not the thing that will tell you.
pub const COMMON_SPDX_LICENSES: &[&str] = &[
    "0BSD",
    "AAL",
    "AFL-3.0",
    "AGPL-3.0-only",
    "AGPL-3.0-or-later",
    "Apache-2.0",
    "Artistic-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "BlueOak-1.0.0",
    "CAL-1.0",
    "CAL-1.0-Combined-Work-Exception",
    "CECILL-2.1",
    "CERN-OHL-P-2.0",
    "CERN-OHL-S-2.0",
    "CERN-OHL-W-2.0",
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
        // Deliberately not "is not a recognized SPDX identifier": this list is
        // a shortlist of ~40 out of 700+, so saying that would be wrong for
        // most valid identifiers.
        Err(ExtensionError::new(format!(
            "license '{license}' is not in quack-rs's list of common SPDX identifiers. \
             It may still be valid — check https://spdx.org/licenses/. \
             Common choices: MIT, Apache-2.0, BSD-3-Clause, GPL-3.0-or-later, MPL-2.0"
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
        assert!(err.as_str().contains("not in quack-rs's list"));
    }

    #[test]
    fn rejection_does_not_claim_the_identifier_is_invalid() {
        // `CC0-1.0` is a real SPDX identifier that this shortlist omits. The
        // message must send the reader to the registry, not tell them their
        // perfectly valid license does not exist.
        let err = validate_spdx_license("CC0-1.0").unwrap_err();
        assert!(
            err.as_str().contains("may still be valid"),
            "misleading message: {err}"
        );
        assert!(err.as_str().contains("spdx.org/licenses"));
    }

    #[test]
    fn list_is_sorted_and_unique() {
        let mut sorted = COMMON_SPDX_LICENSES.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted.as_slice(),
            COMMON_SPDX_LICENSES,
            "keep the list sorted so additions are reviewable"
        );
        sorted.dedup();
        assert_eq!(sorted.len(), COMMON_SPDX_LICENSES.len());
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
