// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `quack_scaffold`: writes a new `DuckDB` extension project to disk, from
//! [`quack_rs::scaffold::generate_scaffold_with_warnings`], and says what to
//! run next.
//!
//! ```text
//! cargo install quack-rs --bin quack_scaffold
//! quack_scaffold my_ext --description "My functions" --maintainer "Jane Doe" \
//!     --github-repo janedoe/duckdb-my-ext
//! ```

#![allow(missing_docs)]

mod cli;
mod write;

use std::path::Path;
use std::process;

use quack_rs::scaffold::{generate_scaffold_with_warnings, GeneratedFile, ScaffoldConfig};

use cli::Command;

fn run() -> Result<(), String> {
    let raw: Vec<_> = std::env::args_os().collect();
    let prog = raw
        .first()
        .and_then(|p| Path::new(p).file_name())
        .map_or_else(|| "quack_scaffold".into(), |p| p.to_string_lossy());
    let args = match cli::parse(raw.get(1..).unwrap_or_default()) {
        Ok(Command::Help) => {
            println!("{}", cli::help(&prog));
            return Ok(());
        }
        Ok(Command::Run(args)) => args,
        Err(e) => return Err(format!("{e}\n\nRun `{prog} --help` for usage.")),
    };

    let scaffold = generate_scaffold_with_warnings(&args.config).map_err(|e| e.to_string())?;
    for warning in &scaffold.warnings {
        eprintln!("warning: {warning}");
    }
    write::write_project(&args.dir, &scaffold.files, args.force)?;
    println!("{}", next_steps(&args.config, &args.dir, &scaffold.files));
    Ok(())
}

/// What to run after generating: the submodule the `Makefile` needs, the
/// build and test, and how to stamp a build without `make`.
fn next_steps(config: &ScaffoldConfig, dir: &Path, files: &[GeneratedFile]) -> String {
    let name = &config.name;
    // The branch `.gitmodules` tracks, so the command adds the same one.
    let branch = files
        .iter()
        .find(|f| f.path == ".gitmodules")
        .and_then(|f| {
            f.content
                .lines()
                .find_map(|l| l.trim().strip_prefix("branch = "))
        })
        .unwrap_or(quack_rs::scaffold::DISTRIBUTION_CI_TOOLS_REF);
    let abi = if config.use_unstable_c_api {
        "C_STRUCT_UNSTABLE"
    } else {
        "C_STRUCT"
    };
    let library = format!(
        "target/release/{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    format!(
        "Wrote {count} files to {dir}.

Next steps:
  cd {cd}
  git init    # if it is not a git repository yet
  git submodule add -b {branch} https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
  make configure release test    # builds build/release/{name}.duckdb_extension and runs test/sql
  cargo test

`make release` appends the DuckDB metadata footer itself. To stamp a build without make
(append_metadata: `cargo install quack-rs --bin append_metadata`; --platform defaults to
this machine's):
  cargo build --release
  mkdir -p build/release
  append_metadata {library} build/release/{name}.duckdb_extension --abi-type {abi} --extension-version v{version} --duckdb-version {target}

Before submitting to duckdb/community-extensions, commit and set repo.ref in
description.yml to the commit's hash.",
        count = files.len(),
        dir = dir.display(),
        cd = shell_word(&dir.to_string_lossy()),
        version = config.version,
        target = config.target_duckdb_version,
    )
}

/// `word` as one POSIX shell word: as it is when that is safe, else in single
/// quotes.
fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+' | b':')
        });
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests;
