// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Writing the generated files: everything is checked before anything is
//! written, so a refusal leaves the directory as it was.

use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use quack_rs::scaffold::GeneratedFile;

/// How many existing files a refusal names before it says "and N more".
const NAMED: usize = 5;

/// Writes `files` under `dir`, creating directories as needed.
///
/// Refused, with nothing written:
///
/// - `dir` exists and is not a directory;
/// - a file to write, or a directory on its way inside `dir`, is a symbolic
///   link (writing would follow it out of the project), or a file's path is a
///   directory;
/// - without `force`, a file to write already exists.
///
/// # Errors
///
/// The refusal, or the first write that failed, naming its path.
pub fn write_project(dir: &Path, files: &[GeneratedFile], force: bool) -> Result<(), String> {
    match fs::metadata(dir) {
        Ok(meta) if !meta.is_dir() => {
            return Err(format!("{} exists and is not a directory", dir.display()));
        }
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot use {}: {e}", dir.display())),
    }

    let mut existing = Vec::new();
    for file in files {
        let path = dir.join(&file.path);
        check_inside(dir, &file.path)?;
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {
                return Err(format!("{} is a directory", path.display()));
            }
            Ok(_) => existing.push(file.path.as_str()),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot use {}: {e}", path.display())),
        }
    }
    if !force && !existing.is_empty() {
        let mut named = existing[..existing.len().min(NAMED)].join(", ");
        if existing.len() > NAMED {
            let _ = write!(named, " and {} more", existing.len() - NAMED);
        }
        return Err(format!(
            "{} already has {named}; pass --force to overwrite the files the scaffold \
             generates (nothing was written)",
            dir.display()
        ));
    }

    for file in files {
        let path = dir.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(&path, &file.content)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Refuses a symbolic link at `relative` or at any directory on the way to it
/// inside `dir`: `fs::write` would follow it, out of the project.
fn check_inside(dir: &Path, relative: &str) -> Result<(), String> {
    let mut path = PathBuf::from(dir);
    for part in relative.split('/') {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "{} is a symbolic link; refusing to write through it",
                    path.display()
                ));
            }
            Ok(_) => {}
            // Nothing below a missing path exists either.
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("cannot use {}: {e}", path.display())),
        }
    }
    Ok(())
}
