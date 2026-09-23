// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Dev tool: run `parse_description_yml` over real `description.yml` files and
//! report what it makes of each.
//!
//! ```console
//! # A checkout of duckdb/community-extensions (extensions/<name>/description.yml):
//! cargo run --example parse_descriptions -- ../community-extensions
//! # Or a flat directory of <name>.yml files:
//! cargo run --example parse_descriptions -- ./descriptors
//! ```
//!
//! Finding no files at all is a failure, not "0 parsed, 0 rejected": that is
//! what pointing it at the wrong directory looks like. The exit status is also
//! non-zero if any file is rejected.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Every descriptor under `dir`, with the extension name its location implies:
/// `<dir>/<name>.yml`, `<dir>/<name>/description.yml`, and the
/// community-extensions layout `<dir>/extensions/<name>/description.yml`.
fn descriptors(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    for root in [dir.to_path_buf(), dir.join("extensions")] {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries {
            let path = entry?.path();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if path.is_dir() {
                let nested = path.join("description.yml");
                if nested.is_file() {
                    found.push((name, nested));
                }
            } else if path.extension().is_some_and(|e| e == "yml") {
                found.push((name, path));
            }
        }
    }
    found.sort();
    Ok(found)
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: parse_descriptions <dir>");
        return ExitCode::FAILURE;
    };
    let entries = match descriptors(Path::new(&dir)) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("cannot read {dir}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if entries.is_empty() {
        eprintln!(
            "no description.yml files found in {dir} (looked for <name>.yml, \
             <name>/description.yml and extensions/<name>/description.yml)"
        );
        return ExitCode::FAILURE;
    }

    let (mut ok, mut err) = (0, 0);
    for (stem, path) in entries {
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                err += 1;
                println!("FAIL {stem:<16} cannot read {}: {e}", path.display());
                continue;
            }
        };
        match quack_rs::validate::description_yml::parse_description_yml(&content) {
            Ok(d) => {
                ok += 1;
                let mismatch = if d.name == stem {
                    ""
                } else {
                    "  <-- NAME MISMATCH"
                };
                println!(
                    "OK   {stem:<16} name={:<16} version={:<10} lang={:<6} build={:<8} lic={}{mismatch}",
                    d.name,
                    d.version.as_deref().unwrap_or("-"),
                    d.language,
                    d.build,
                    d.license
                );
            }
            Err(e) => {
                err += 1;
                println!("FAIL {stem:<16} {e}");
            }
        }
    }
    println!("\n{ok} parsed, {err} rejected");
    if err == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
