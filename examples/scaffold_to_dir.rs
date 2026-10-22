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
//! cargo run --example scaffold_to_dir -- my_ext               # writes ./my_ext
//! cargo run --example scaffold_to_dir -- my_ext /tmp/my_ext
//! cargo run --example scaffold_to_dir -- my_ext --c-api v1.5.6    # duckdb-1-5* features
//! cargo run --example scaffold_to_dir -- my_ext --unstable v1.5.5  # one 1.x release
//! cargo run --example scaffold_to_dir -- --help
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};

const DEFAULT_NAME: &str = "my_extension";

const USAGE: &str = "\
usage: scaffold_to_dir [NAME] [OUTPUT_DIR] [--description TEXT]
                       [--c-api C_API_VERSION | --unstable DUCKDB_VERSION]

  NAME         extension name: lowercase letters, digits and '_', starting with a
               letter (default: my_extension)
  OUTPUT_DIR   where to write the project; must not exist or be empty
               (default: ./NAME)
  --description TEXT
               the description for description.yml and the crate docs
  --c-api C_API_VERSION
               the C extension API version to stamp (C_STRUCT): v1.2.0, the
               default, loads into every DuckDB since v1.2.0; v1.5.6 is what
               quack-rs's duckdb-1-5* features need, and loads into DuckDB
               v1.5.6 and v2.0.0
  --unstable DUCKDB_VERSION
               pin the binary to one DuckDB 1.x release (C_STRUCT_UNSTABLE),
               e.g. v1.5.5; DuckDB 2.0 cannot load such a binary";

struct Args {
    name: String,
    out_dir: PathBuf,
    description: Option<String>,
    unstable: Option<String>,
    c_api: Option<String>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut positional = Vec::new();
    let mut description = None;
    let mut unstable = None;
    let mut c_api = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--description" => {
                description = Some(args.next().ok_or("--description requires a value")?);
            }
            "--c-api" => {
                c_api = Some(
                    args.next()
                        .ok_or("--c-api requires a C API version, e.g. --c-api v1.5.6")?,
                );
            }
            "--unstable" => {
                unstable = Some(
                    args.next()
                        .ok_or("--unstable requires a DuckDB version, e.g. --unstable v1.5.5")?,
                );
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            _ => positional.push(arg),
        }
    }
    if unstable.is_some() && c_api.is_some() {
        return Err("--c-api and --unstable are mutually exclusive".to_string());
    }
    if positional.len() > 2 {
        return Err(format!("unexpected argument {:?}", positional[2]));
    }
    let mut positional = positional.into_iter();
    let name = positional
        .next()
        .unwrap_or_else(|| DEFAULT_NAME.to_string());
    let out_dir = positional
        .next()
        .map_or_else(|| PathBuf::from(&name), PathBuf::from);
    Ok(Some(Args {
        name,
        out_dir,
        description,
        unstable,
        c_api,
    }))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    if fs::read_dir(&args.out_dir).is_ok_and(|mut entries| entries.next().is_some()) {
        eprintln!(
            "error: {} already exists and is not empty; choose another OUTPUT_DIR",
            args.out_dir.display()
        );
        return ExitCode::FAILURE;
    }

    let mut config = ScaffoldConfig {
        name: args.name.clone(),
        description: args
            .description
            .unwrap_or_else(|| format!("The {} DuckDB extension", args.name)),
        maintainer: "Your Name".to_string(),
        github_repo: format!("your-github-user/duckdb-{}", args.name.replace('_', "-")),
        ..ScaffoldConfig::default()
    };
    if let Some(version) = args.unstable {
        config.use_unstable_c_api = true;
        config.target_duckdb_version = version;
    }
    if let Some(version) = args.c_api {
        config.target_duckdb_version = version;
    }

    let files = match generate_scaffold(&config) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    for file in files {
        let path = args.out_dir.join(&file.path);
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
    println!(
        "\nNext: edit `maintainer` and `repo` in {dir}/description.yml, then follow\n\
         \"After scaffolding\" in the quack-rs book (commit before `make configure`).",
        dir = args.out_dir.display()
    );

    ExitCode::SUCCESS
}
