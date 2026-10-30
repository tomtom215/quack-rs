// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! The entry-point symbol `DuckDB` looks for.
//!
//! `DuckDB` takes the extension's name from the *file name* and, for a C API
//! extension, calls the symbol `<name>_init_c_api`. A library that exports
//! `a_init_c_api` stamped into `b.duckdb_extension` stamps without complaint
//! and then fails at `LOAD` with `did not contain function "b_init_c_api"`.
//!
//! The name is `FileSystem::ExtractBaseName` of the path, lowercased
//! (`extension_load.cpp`, 1.5.6 and 2.0.0): the file name up to its first
//! `.`. A Mach-O symbol table spells the C symbol with a leading `_`.

use std::path::Path;

/// The suffix of the symbol `DuckDB` calls, with the NUL that ends it in the
/// string table of an ELF, Mach-O or PE file.
const ENTRY_SUFFIX: &[u8] = b"_init_c_api\0";

/// The extension name `DuckDB` derives from `output`: its file name up to the
/// first `.`, lowercased, as `ExtractBaseName` and `StringUtil::Lower` make it.
pub fn extension_name(output: &Path) -> Option<String> {
    let name = output.file_name()?.to_str()?;
    let stem = name.split('.').next().unwrap_or(name);
    (!stem.is_empty()).then(|| stem.to_lowercase())
}

/// Whether `data` is a Mach-O image (thin or universal), whose symbol and
/// export tables carry each C symbol with a leading underscore.
fn is_macho(data: &[u8]) -> bool {
    matches!(
        data.get(..4),
        Some(
            [0xfe, 0xed, 0xfa, 0xce | 0xcf]
                | [0xce | 0xcf, 0xfa, 0xed, 0xfe]
                | [0xca, 0xfe, 0xba, 0xbe]
        )
    )
}

/// Every `X` for which `data` holds the NUL-terminated string `X_init_c_api`,
/// in first-seen order and without repeats, at most eight of them.
///
/// `X_init_c_api_v2` (the C API v2 entry point) does not count: its string
/// continues past `_init_c_api`, so the NUL does not follow.
pub fn entry_point_names(data: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(at) = find(&data[from..], ENTRY_SUFFIX) {
        let end = from + at;
        let start = data[..end]
            .iter()
            .rposition(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
            .map_or(0, |p| p + 1);
        if let Ok(name) = std::str::from_utf8(&data[start..end]) {
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
        if names.len() == 8 {
            break;
        }
        from = end + ENTRY_SUFFIX.len();
    }
    names
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// What to tell the user about the entry point of `data` stamped as `output`,
/// or `None` when it is as `DuckDB` will look for it. A note, not an error:
/// stamping to a temporary name and renaming is ordinary, and a file name is
/// all `DuckDB` goes by.
pub fn entry_point_note(data: &[u8], output: &Path) -> Option<String> {
    let expected = extension_name(output)?;
    let mut found = entry_point_names(data);
    if is_macho(data) {
        for name in &mut found {
            if let Some(c_name) = name.strip_prefix('_') {
                *name = c_name.to_owned();
            }
        }
    }
    if found.contains(&expected) {
        return None;
    }
    Some(if found.is_empty() {
        format!(
            "the input exports no `<name>_init_c_api` symbol, so DuckDB cannot load it as \
             `{expected}`: check that the crate builds a cdylib and that the entry point's \
             name is `{expected}_init_c_api`"
        )
    } else {
        let list = found
            .iter()
            .map(|n| format!("`{n}_init_c_api`"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "the input exports {list}, but DuckDB takes the extension name from the file \
             name and will look for `{expected}_init_c_api` in `{}`: LOAD fails with \
             \"did not contain function\" unless the file is named `{}.duckdb_extension`",
            output.display(),
            found[0]
        )
    })
}
