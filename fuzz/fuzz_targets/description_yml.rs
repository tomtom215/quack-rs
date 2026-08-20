// SPDX-License-Identifier: MIT
//! Fuzzes the hand-rolled `description.yml` parser and validator.
//!
//! An extension author runs this over a file from a repository, and the
//! community-extensions CI runs it over every submission — so the input is
//! attacker-influenced in both directions. The parser is a hand-written section
//! scanner over arbitrary text, which is exactly where an unchecked slice index
//! or a `[..n]` on a multi-byte boundary hides.
//!
//! The property is simply "never panic": both entry points return `Result`, so
//! any panic is a bug regardless of how malformed the input is.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quack_rs::validate::description_yml::{parse_description_yml, validate_description_yml_str};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = parse_description_yml(text);
    let _ = validate_description_yml_str(text);
});
