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
//! cargo run --example scaffold_to_dir -- my_ext --unstable v1.5.6
//! cargo run --example scaffold_to_dir -- --help
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};

const DEFAULT_NAME: &str = "my_extension";

const USAGE: &str = "\
usage: scaffold_to_dir [NAME] [OUTPUT_DIR] [--description TEXT] [--unstable DUCKDB_VERSION]

  NAME         extension name: lowercase letters, digits and '_', starting with a
               letter (default: my_extension)
  OUTPUT_DIR   where to write the project; must not exist or be empty
               (default: ./NAME)
  --description TEXT
               the description for description.yml and the crate docs
  --unstable DUCKDB_VERSION
               pin the binary to one DuckDB release (C_STRUCT_UNSTABLE), e.g.
               v1.5.6; needed for quack-rs's duckdb-1-5* features";

struct Args {
    name: String,
    out_dir: PathBuf,
    description: Option<String>,
    unstable: Option<String>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut positional = Vec::new();
    let mut description = None;
    let mut unstable = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--description" => {
                description = Some(args.next().ok_or("--description requires a value")?);
            }
            "--unstable" => {
                unstable = Some(
                    args.next()
                        .ok_or("--unstable requires a DuckDB version, e.g. --unstable v1.5.6")?,
                );
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            _ => positional.push(arg),
        }
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
