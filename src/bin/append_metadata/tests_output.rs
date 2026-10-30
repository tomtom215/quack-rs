// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Reading the input and writing the output (LASTDIFF-6, HARDEN-3, HARDEN-4,
//! FFI-11), and the pointers to the book (DOCS-8). Split from `tests.rs` to
//! keep both files within the project's size budget.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;

/// A fresh directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("quack_output_{}_{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("tmp"))
        })
        .collect()
}

/// HARDEN-3: the temporary file was `<output>.<pid>.tmp`, opened with
/// create-and-truncate, so a symbolic link planted at that name made the tool
/// overwrite the link's target. The name is now random and the file created
/// exclusively.
#[cfg(unix)]
#[test]
fn a_link_planted_at_the_old_temporary_name_is_not_written_through() {
    let dir = scratch("planted");
    let victim = dir.join("victim.txt");
    fs::write(&victim, b"precious").unwrap();
    let out = dir.join("ext.duckdb_extension");
    let planted = dir.join(format!("ext.duckdb_extension.{}.tmp", std::process::id()));
    std::os::unix::fs::symlink(&victim, &planted).unwrap();

    output::write_atomically(&out, b"new build").unwrap();

    assert_eq!(fs::read(&victim).unwrap(), b"precious");
    assert_eq!(fs::read(&out).unwrap(), b"new build");
    // Only the planted link is left; the tool's own file was renamed.
    assert_eq!(
        temp_files(&dir),
        [planted.file_name().unwrap().to_string_lossy()]
    );
    let _ = fs::remove_dir_all(&dir);
}

/// LASTDIFF-6 (b): the rename replaced a file of mode 750 with one of mode
/// 644; a re-stamp now keeps the mode.
#[cfg(unix)]
#[test]
fn rewriting_keeps_the_outputs_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("mode");
    let out = dir.join("ext.duckdb_extension");
    fs::write(&out, b"old").unwrap();
    fs::set_permissions(&out, fs::Permissions::from_mode(0o750)).unwrap();

    output::write_atomically(&out, b"new").unwrap();

    assert_eq!(fs::read(&out).unwrap(), b"new");
    let mode = fs::metadata(&out).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o750, "{mode:o}");
    let _ = fs::remove_dir_all(&dir);
}

/// LASTDIFF-6 (c): an output that is a symbolic link was replaced by a regular
/// file, leaving the file it pointed to stale. It is now written through, as
/// 0.19.0's `fs::write` did, and the link is kept.
#[cfg(unix)]
#[test]
fn a_symlinked_output_is_written_through_and_kept() {
    let dir = scratch("link");
    let real = dir.join("real.duckdb_extension");
    fs::write(&real, b"old").unwrap();
    let link = dir.join("ext.duckdb_extension");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    output::write_atomically(&link, b"new").unwrap();

    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&real).unwrap(), b"new");
    assert!(temp_files(&dir).is_empty(), "{:?}", temp_files(&dir));

    // A link to nothing is an error that says so, and writes nothing.
    let dangling = dir.join("dangling.duckdb_extension");
    std::os::unix::fs::symlink(dir.join("missing"), &dangling).unwrap();
    let err = output::write_atomically(&dangling, b"new").unwrap_err();
    assert!(err.contains("symbolic link"), "{err}");
    assert!(!dir.join("missing").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// FFI-11 and LASTDIFF-6 (d): the error for a missing directory named the
/// temporary file (`…/x.duckdb_extension.25456.tmp: No such file or
/// directory`), a name the user never gave.
#[test]
fn a_missing_output_directory_is_named_as_such() {
    let dir = scratch("missing");
    let out = dir.join("no").join("such").join("ext.duckdb_extension");
    let err = output::write_atomically(&out, b"new").unwrap_err();
    assert!(err.contains("does not exist"), "{err}");
    assert!(
        err.contains(&dir.join("no").join("such").display().to_string()),
        "{err}"
    );
    assert!(!err.contains(".tmp"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

/// HARDEN-4: `std::env::args()` panics on an argument that is not UTF-8; a
/// Unix file name can be any bytes but NUL. (Harden's test, patch 02.)
#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_accepted_not_a_panic() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let input = OsStr::from_bytes(b"in\xff.so");
    let output = OsStr::from_bytes(b"out\xfe.duckdb_extension");
    let argv = [
        input,
        output,
        OsStr::new("--platform"),
        OsStr::new("linux_amd64"),
    ];
    match cli::parse(&argv) {
        Ok(Command::Run(parsed)) => {
            assert_eq!(parsed.input.as_os_str(), input);
            assert_eq!(parsed.output.as_os_str(), output);
        }
        other => panic!("{other:?}"),
    }
    // A flag value must still be text.
    let argv = [
        input,
        output,
        OsStr::new("--platform"),
        OsStr::from_bytes(b"linux\xff"),
    ];
    let err = match cli::parse(&argv) {
        Err(e) => e,
        other => panic!("{other:?}"),
    };
    assert!(err.contains("UTF-8"), "{err}");
}

/// HARDEN-15's lesson for the input: `fs::read` read a character device until
/// memory ran out. Only a regular file is read.
#[cfg(unix)]
#[test]
fn only_a_regular_file_is_read() {
    let err = output::read_input(Path::new("/dev/zero")).unwrap_err();
    assert!(err.contains("not a regular file"), "{err}");
    let dir = scratch("read");
    let err = output::read_input(&dir).unwrap_err();
    assert!(err.contains("not a regular file"), "{err}");
    let lib = dir.join("lib.so");
    fs::write(&lib, b"\x7fELF library").unwrap();
    assert_eq!(output::read_input(&lib).unwrap(), b"\x7fELF library");
    let _ = fs::remove_dir_all(&dir);
}

/// DOCS-8: the help and the P2 error pointed to `LESSONS.md`, which the
/// published crate does not contain.
#[test]
fn help_and_errors_point_to_the_book() {
    let help = cli::help("append_metadata");
    assert!(help.contains(cli::P2_URL), "{help}");
    let argv = ["in", "out", "--duckdb-version", "v1.5.5"].map(String::from);
    let err = match cli::parse(&argv) {
        Err(e) => e,
        other => panic!("{other:?}"),
    };
    assert!(err.contains(cli::P2_URL), "{err}");
    for text in [&help, &err] {
        assert!(!text.contains("LESSONS"), "{text}");
    }
    assert!(cli::P2_URL.starts_with("https://quack-rs.com/reference/pitfalls.html#p2-"));
}
