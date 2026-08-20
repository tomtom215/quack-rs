// SPDX-License-Identifier: MIT
//! Fuzzes the `duckdb_string_t` decoder over arbitrary 16-byte inputs.
//!
//! `DuckStringView::inline_from_bytes` is the safe constructor: it takes 16
//! bytes from anywhere and must refuse — rather than dereference — a value whose
//! length field says "pointer format". That refusal is the whole safety
//! argument, so it gets fuzzed against every bit pattern, including lengths of
//! `u32::MAX`, lengths that disagree with the payload, and non-UTF-8 bytes.
//!
//! `from_raw` is deliberately *not* fuzzed: it is `unsafe` precisely because
//! honouring the embedded pointer can only be justified by the caller.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quack_rs::vector::string::{DuckStringView, DUCK_STRING_INLINE_MAX_LEN, DUCK_STRING_SIZE};

fuzz_target!(|data: &[u8]| {
    if data.len() < DUCK_STRING_SIZE {
        return;
    }
    let mut raw = [0u8; DUCK_STRING_SIZE];
    raw.copy_from_slice(&data[..DUCK_STRING_SIZE]);

    let declared = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    match DuckStringView::inline_from_bytes(&raw) {
        Some(view) => {
            assert!(
                declared <= DUCK_STRING_INLINE_MAX_LEN,
                "a pointer-format value must be refused, not accepted"
            );
            assert_eq!(view.len(), declared);
            // The accessor must not panic, whatever the bytes say.
            let _ = view.as_str();
        }
        None => assert!(
            declared > DUCK_STRING_INLINE_MAX_LEN,
            "an inline-length value must be accepted, not refused"
        ),
    }
});
