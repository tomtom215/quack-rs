// SPDX-License-Identifier: MIT
//! Fuzzes the community-extension validators against arbitrary text.
//!
//! These run over names, versions and license identifiers that come from a
//! `description.yml` or a `Cargo.toml`, i.e. from outside the extension. Each
//! returns a `Result`, so a panic — an out-of-range slice, a char-boundary
//! split, an arithmetic overflow — is a bug for any input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quack_rs::validate::{
    validate_extension_name, validate_extension_version, validate_function_name, validate_platform,
    validate_semver, validate_spdx_license,
};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = validate_extension_name(text);
    let _ = validate_function_name(text);
    let _ = validate_semver(text);
    let _ = validate_extension_version(text);
    let _ = validate_spdx_license(text);
    let _ = validate_platform(text);
});
