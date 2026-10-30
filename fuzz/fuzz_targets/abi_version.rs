// SPDX-License-Identifier: MIT
//! `abi::parse_version` against a strict model of its documented grammar, and
//! the table lookups built on it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quack_rs::abi::{expected_slot_count, parse_version};

fn model(s: &str) -> Option<(u64, u64, u64)> {
    let body = s.strip_prefix('v').unwrap_or(s);
    let parts: Vec<&str> = body.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut n = [0u64; 3];
    for (slot, p) in n.iter_mut().zip(&parts) {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = p.parse().ok()?;
    }
    Some((n[0], n[1], n[2]))
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    assert_eq!(parse_version(s), model(s), "{s:?}");
    let _ = expected_slot_count(s);
});
