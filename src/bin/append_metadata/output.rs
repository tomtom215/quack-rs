// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Reading the library and writing the stamped file.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

/// The largest input read: far above any extension (`DuckDB`'s own are tens
/// of MB), and low enough that a wrong path fails instead of exhausting memory.
pub const MAX_INPUT: u64 = 4 << 30;

/// How many temporary names [`write_atomically`] tries before giving up.
const MAX_TEMP_TRIES: u32 = 32;

/// Reads the library at `path`: a regular file of at most [`MAX_INPUT`] bytes.
///
/// `fs::read` read until end of file: a character device such as `/dev/zero`
/// filled memory until the process was killed, and a FIFO with no writer
/// blocked forever in `open`. So the path is checked before it is opened, and
/// the opened file again (it may have been replaced in between); the read
/// stops one byte past the limit whatever the file claims its size is.
///
/// # Errors
///
/// When `path` cannot be read, is not a regular file, or is too large.
pub fn read_input(path: &Path) -> Result<Vec<u8>, String> {
    let fail = |e: std::io::Error| format!("failed to read {}: {e}", path.display());
    let not_a_file = || {
        format!(
            "{} is not a regular file: expected the compiled library",
            path.display()
        )
    };
    if !fs::metadata(path).map_err(fail)?.is_file() {
        return Err(not_a_file());
    }
    let file = fs::File::open(path).map_err(fail)?;
    let meta = file.metadata().map_err(fail)?;
    if !meta.is_file() {
        return Err(not_a_file());
    }
    let too_large = || {
        format!(
            "{} is larger than {} GiB, which no extension library is; check the path",
            path.display(),
            MAX_INPUT >> 30
        )
    };
    if meta.len() > MAX_INPUT {
        return Err(too_large());
    }
    let mut data = Vec::new();
    file.take(MAX_INPUT + 1)
        .read_to_end(&mut data)
        .map_err(fail)?;
    if data.len() as u64 > MAX_INPUT {
        return Err(too_large());
    }
    Ok(data)
}

/// A temporary file, closed and removed when dropped unless it was renamed.
struct Temp {
    path: PathBuf,
    file: Option<fs::File>,
    renamed: bool,
}

impl Drop for Temp {
    fn drop(&mut self) {
        // Closed first: Windows removes an open file only once it is closed.
        drop(self.file.take());
        if !self.renamed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Writes `bytes` to `path` by writing a new sibling file and renaming it
/// over `path`.
///
/// `fs::write` truncates and rewrites the file in place. A process that has
/// the previous build loaded has that file mapped, so rewriting it under the
/// process crashes it on the next call into the extension (checked: a host
/// that loaded the extension, then had it re-stamped, died with `SIGSEGV`),
/// and a write that fails halfway leaves a truncated library. A rename
/// replaces the directory entry and leaves the old file intact for whoever
/// has it open.
///
/// - The sibling is created with `create_new` (`O_CREAT | O_EXCL`), which
///   fails on any existing path, a symbolic link included, instead of
///   following it, under a name with 64 random bits: a link planted at a
///   guessable `<output>.<pid>.tmp` made the tool overwrite its target
///   (HARDEN-3).
/// - It is removed on every failure.
/// - An existing output's permissions are copied to it, so a re-stamp keeps
///   the file's mode. (Its owner and group become the caller's.)
/// - An output that is a symbolic link is written through: the link stays,
///   and the file it points to is replaced, as `fs::write` did.
/// - Errors name `path`, and a missing output directory as such.
///
/// The directory must be writable, which an in-place write did not need.
///
/// # Errors
///
/// What failed, naming `path`.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let target = resolve(path)?;
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    match fs::metadata(dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "cannot write {}: {} is not a directory",
                path.display(),
                dir.display()
            ));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(format!(
                "cannot write {}: the output directory {} does not exist",
                path.display(),
                dir.display()
            ));
        }
        Err(e) => {
            return Err(format!(
                "cannot write {}: {}: {e}",
                path.display(),
                dir.display()
            ))
        }
    }
    let existing = fs::metadata(&target).ok().filter(fs::Metadata::is_file);

    let mut temp = create_temp(&target).map_err(|e| {
        format!(
            "cannot write {}: creating a temporary file in {} failed: {e}",
            path.display(),
            dir.display()
        )
    })?;
    let fail = |e: std::io::Error| format!("failed to write {}: {e}", path.display());
    let file = temp
        .file
        .as_mut()
        .ok_or_else(|| format!("failed to write {}", path.display()))?;
    file.write_all(bytes).map_err(fail)?;
    if let Some(meta) = existing {
        file.set_permissions(meta.permissions()).map_err(fail)?;
    }
    file.sync_all().map_err(fail)?;
    drop(temp.file.take());
    fs::rename(&temp.path, &target).map_err(|e| {
        format!(
            "failed to replace {} (is it loaded by a running process, on a system that \
             forbids replacing a mapped file?): {e}",
            path.display()
        )
    })?;
    temp.renamed = true;
    Ok(())
}

/// `path`, or the file it finally points to if it is a symbolic link.
fn resolve(path: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path).map_err(|e| {
            format!(
                "cannot write {}: it is a symbolic link whose target cannot be resolved: {e}",
                path.display()
            )
        }),
        _ => Ok(path.to_path_buf()),
    }
}

/// Creates a new sibling of `target` named `<file name>.<16 hex digits>.tmp`.
fn create_temp(target: &Path) -> std::io::Result<Temp> {
    let name = target.file_name().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "the output has no file name")
    })?;
    let mut attempt = 0;
    loop {
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(".{:016x}.tmp", random()));
        let path = target.with_file_name(temp_name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                return Ok(Temp {
                    path,
                    file: Some(file),
                    renamed: false,
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists && attempt < MAX_TEMP_TRIES => {
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// 64 unpredictable bits: a hash keyed with the standard library's `HashMap`
/// keys, which come from the operating system's random number generator (no
/// dependency needed).
fn random() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.finish()
}
