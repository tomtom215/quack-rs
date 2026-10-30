// SPDX-License-Identifier: MIT
//! Fuzzes `append_metadata`'s input handling: the footer detector, the
//! entry-point string-table scan, the stamp round trip and the command-line
//! parser. The binary's modules are included by path (it has no library).
#![no_main]
#![allow(dead_code)]

#[path = "../../src/bin/append_metadata/cli.rs"]
mod cli;
#[path = "../../src/bin/append_metadata/entry.rs"]
mod entry;
#[path = "../../src/bin/append_metadata/footer.rs"]
mod footer;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1. Any bytes: the detector and the scan must return, and agree with themselves.
    if let Some(n) = footer::existing_stamp_len(data) {
        assert!(
            n == footer::METADATA_SIZE
                || n == footer::METADATA_SIZE + footer::WASM_SECTION_HEADER.len()
        );
        assert!(n <= data.len());
    }
    let names = entry::entry_point_names(data);
    assert!(names.len() <= 8);
    for n in &names {
        assert!(
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "{n:?}"
        );
    }
    let _ = entry::entry_point_note(data, std::path::Path::new("x.duckdb_extension"));

    // 2. Stamp round trip: a stamped file is recognised, with the length stamped.
    let block =
        footer::build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").expect("valid");
    let mut stamped = data.to_vec();
    stamped.extend_from_slice(&block);
    // 534, not 512, when the payload itself ends in the 22-byte WebAssembly
    // section header (the detector reads that header as part of the stamp).
    match footer::existing_stamp_len(&stamped) {
        Some(n) if n == footer::METADATA_SIZE => {}
        Some(n) if n == footer::METADATA_SIZE + footer::WASM_SECTION_HEADER.len() => {
            assert!(data.ends_with(&footer::WASM_SECTION_HEADER));
        }
        other => panic!("a stamped file is not recognised as stamped: {other:?}"),
    }
    // The names found before stamping are still found after (the footer adds none).
    assert!(entry::entry_point_names(&stamped).len() >= names.len().min(8));

    // 3. Arguments: NUL-separated strings from the input.
    let args: Vec<String> = String::from_utf8_lossy(data)
        .split('\0')
        .map(str::to_owned)
        .collect();
    let _ = cli::parse(&args);
});
