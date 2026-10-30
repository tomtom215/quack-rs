// SPDX-License-Identifier: MIT
//! A double-quoted YAML scalar, encoded the way JSON encodes a string, must
//! decode to the text it encodes through `parse_description_yml`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quack_rs::validate::description_yml::parse_description_yml;

fn dq(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // PyYAML refuses a file holding a character outside its printable
            // set (C0 controls but TAB/LF/CR, DEL, the C1 controls but NEL,
            // U+FFFE, U+FFFF) even inside quotes; an escape is fine.
            c if (c as u32) < 0x20
                || matches!(
                    c,
                    '\u{7f}'..='\u{9f}' | '\u{fffe}' | '\u{ffff}' | '\u{2028}' | '\u{2029}'
                ) =>
            {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }
    let yml = format!(
        "extension:\n  name: my_ext\n  description: {}\n  version: 0.1.0\n  language: Rust\n  build: cargo\n  license: MIT\n  maintainers:\n    - {}\nrepo:\n  github: a/b\n  ref: 4f1c2e9a7b3d5c8e1f0a2b4c6d8e0f1a3b5c7d9e\n",
        dq(text), dq(text)
    );
    let parsed = parse_description_yml(&yml).unwrap_or_else(|e| panic!("{text:?}: {e}"));
    assert_eq!(parsed.description, text);
    // Maintainer names are trimmed by the reader; the description is not.
    assert_eq!(parsed.maintainers, vec![text.trim().to_string()]);
});
