// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! The 512-byte `DuckDB` extension footer, and the optional 22-byte
//! WebAssembly custom-section header that precedes it.

pub const FIELD_SIZE: usize = 32;
pub const NUM_FIELDS: usize = 8;
pub const METADATA_SIZE: usize = 512;

pub const VALID_ABI_TYPES: &[&str] = &["C_STRUCT", "CPP", "C_STRUCT_UNSTABLE"];

/// The bytes `extension-ci-tools`' `append_extension_metadata.py`
/// (`start_signature()`) writes immediately before the footer.
///
/// They open a WebAssembly *custom section* — id `0`, LEB128 payload length
/// 531 (`0x93 0x04`: the 1-byte name length, the 16-byte name, the 2-byte
/// LEB128 length of the footer and the 512-byte footer itself), name
/// `duckdb_signature`, then the footer's own LEB128 length 512 (`0x80 0x04`).
/// A WebAssembly module with bytes appended after its last section is
/// invalid; wrapped this way the footer is a well-formed section that
/// runtimes ignore. Native loaders read only the last 512 bytes, so the
/// header changes nothing there.
pub const WASM_SECTION_HEADER: [u8; 22] = [
    0x00, 0x93, 0x04, 0x10, b'd', b'u', b'c', b'k', b'd', b'b', b'_', b's', b'i', b'g', b'n', b'a',
    b't', b'u', b'r', b'e', 0x80, 0x04,
];

pub fn make_field(s: &str) -> Result<[u8; FIELD_SIZE], String> {
    let b = s.as_bytes();
    if !b.iter().all(u8::is_ascii) {
        return Err(format!("field {s:?} contains non-ASCII bytes"));
    }
    if b.len() >= FIELD_SIZE {
        return Err(format!(
            "field {s:?} is {} bytes but max is {} (must fit including null terminator)",
            b.len(),
            FIELD_SIZE - 1,
        ));
    }
    let mut field = [0u8; FIELD_SIZE];
    field[..b.len()].copy_from_slice(b);
    Ok(field)
}

pub fn build_metadata(
    abi_type: &str,
    extension_version: &str,
    duckdb_version: &str,
    platform: &str,
) -> Result<[u8; METADATA_SIZE], String> {
    let fields: [[u8; FIELD_SIZE]; NUM_FIELDS] = [
        make_field("")?,                // Field 0: reserved
        make_field("")?,                // Field 1: reserved
        make_field("")?,                // Field 2: reserved
        make_field(abi_type)?,          // Field 3: ABI type
        make_field(extension_version)?, // Field 4: extension version
        make_field(duckdb_version)?,    // Field 5: DuckDB C API version / release
        make_field(platform)?,          // Field 6: platform
        make_field("4")?,               // Field 7: magic (must be "4")
    ];

    let mut block = [0u8; METADATA_SIZE];
    for (i, field) in fields.iter().enumerate() {
        block[i * FIELD_SIZE..(i + 1) * FIELD_SIZE].copy_from_slice(field);
    }
    // Bytes 256–511: RSA-2048 signature area; zero-filled = unsigned extension,
    // and already zero from the array initialisation.
    Ok(block)
}

/// The text of footer field `index` (NUL-terminated within its 32 bytes).
pub fn field_text(footer: &[u8], index: usize) -> &str {
    let field = &footer[index * FIELD_SIZE..(index + 1) * FIELD_SIZE];
    let end = field.iter().position(|&b| b == 0).unwrap_or(FIELD_SIZE);
    std::str::from_utf8(&field[..end]).unwrap_or("(invalid utf-8)")
}

/// Length of the stamp already at the end of `data` — 512 for a footer, 534
/// when the WebAssembly section header precedes it — or `None` if `data` does
/// not end in one.
///
/// A footer is recognised the way `DuckDB` recognises it (magic field `"4"`)
/// plus a known ABI type in field 3; 64 specific bytes matching by chance at
/// the end of a real library is not a practical concern.
pub fn existing_stamp_len(data: &[u8]) -> Option<usize> {
    let footer = data.get(data.len().checked_sub(METADATA_SIZE)?..)?;
    let magic = &footer[7 * FIELD_SIZE..8 * FIELD_SIZE];
    let magic_ok = magic[0] == b'4' && magic[1..].iter().all(|&b| b == 0);
    if !magic_ok || !VALID_ABI_TYPES.contains(&field_text(footer, 3)) {
        return None;
    }
    let header_start = data.len() - METADATA_SIZE;
    let has_wasm_header = header_start
        .checked_sub(WASM_SECTION_HEADER.len())
        .is_some_and(|start| data[start..header_start] == WASM_SECTION_HEADER);
    Some(
        METADATA_SIZE
            + if has_wasm_header {
                WASM_SECTION_HEADER.len()
            } else {
                0
            },
    )
}

pub fn dump_fields(metadata: &[u8; METADATA_SIZE]) {
    const FIELD_NAMES: [&str; NUM_FIELDS] = [
        "reserved",
        "reserved",
        "reserved",
        "abi_type",
        "extension_version",
        "duckdb_version",
        "platform",
        "magic",
    ];
    println!("\nMetadata fields (on-disk order):");
    for (i, name) in FIELD_NAMES.iter().enumerate() {
        println!("  Field {i} [{name:20}]: {:?}", field_text(metadata, i));
    }
}
