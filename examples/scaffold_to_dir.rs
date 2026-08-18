// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Writes a scaffolded extension project to a directory.
//!
//! `generate_scaffold` returns file contents rather than touching the disk, so
//! this example is the thin "actually write them" step. CI uses it to take the
//! scaffold all the way to a `.duckdb_extension` that a real `DuckDB` loads —
//! see the `scaffold-e2e` job.
//!
//! ```console
//! cargo run --example scaffold_to_dir -- ./my_ext
//! cargo run --example scaffold_to_dir -- ./my_ext --unstable v1.5.5
//! ```

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(out_dir) = args.next() else {
        eprintln!("usage: scaffold_to_dir <output-dir> [--unstable <duckdb-version>]");
        return ExitCode::FAILURE;
    };

    let mut config = ScaffoldConfig {
        name: "my_analytics".to_string(),
        description: "Fast analytics functions".to_string(),
        version: "0.1.0".to_string(),
        license: "MIT".to_string(),
        maintainer: "Jane Doe".to_string(),
        github_repo: "janedoe/duckdb-my-analytics".to_string(),
        excluded_platforms: vec![],
        ..ScaffoldConfig::default()
    };

    if args.next().as_deref() == Some("--unstable") {
        let Some(version) = args.next() else {
            eprintln!("--unstable requires a DuckDB version, e.g. --unstable v1.5.5");
            return ExitCode::FAILURE;
        };
        config.use_unstable_c_api = true;
        config.target_duckdb_version = version;
    }

    let files = match generate_scaffold(&config) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("scaffold failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    for file in files {
        let path = Path::new(&out_dir).join(&file.path);
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                eprintln!("failed to create {}: {error}", parent.display());
                return ExitCode::FAILURE;
            }
        }
        if let Err(error) = fs::write(&path, &file.content) {
            eprintln!("failed to write {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {}", path.display());
    }

    ExitCode::SUCCESS
}
