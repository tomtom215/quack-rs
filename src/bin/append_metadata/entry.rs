// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! The entry-point symbol `DuckDB` looks for, and writing the stamped file.
//!
//! `DuckDB` takes the extension's name from the *file name* and, for a C API
//! extension, calls the symbol `<name>_init_c_api`. A library that exports
//! `a_init_c_api` stamped into `b.duckdb_extension` stamps without complaint
//! and then fails at `LOAD` with `undefined symbol: b_init_c_api`.

use std::fs;
use std::path::{Path, PathBuf};

/// The suffix of the symbol `DuckDB` calls, with the NUL that ends it in the
/// string table of an ELF, Mach-O or PE file.
const ENTRY_SUFFIX: &[u8] = b"_init_c_api\0";

/// The extension name `DuckDB` derives from `output`: its file name without
/// the `.duckdb_extension` suffix (or, failing that, without any extension).
pub fn extension_name(output: &Path) -> Option<String> {
    let name = output.file_name()?.to_str()?;
    let stem = name.strip_suffix(".duckdb_extension").unwrap_or_else(|| {
        Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name)
    });
    (!stem.is_empty()).then(|| stem.to_owned())
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
    let found = entry_point_names(data);
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
             \"undefined symbol\" unless the file is named `{}.duckdb_extension`",
            output.display(),
            found[0]
        )
    })
}

/// Writes `bytes` to `path` by writing a sibling file and renaming it over
/// `path`.
///
/// `fs::write` truncates and rewrites the file in place. A process that has
/// the previous build loaded has that file mapped, so rewriting it under the
/// process crashes it on the next call into the extension (checked: a host
/// that loaded the extension, then had it re-stamped, died with `SIGSEGV`),
/// and a write that fails halfway leaves a truncated library. A rename
/// replaces the directory entry and leaves the old file intact for whoever
/// has it open.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let mut temp_name = name.to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp: PathBuf = path.with_file_name(temp_name);
    fs::write(&temp, bytes).map_err(|e| format!("failed to write {}: {e}", temp.display()))?;
    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!(
            "failed to replace {} (is it loaded by a running process, on a system that \
             forbids replacing a mapped file?): {e}",
            path.display()
        )
    })
}
