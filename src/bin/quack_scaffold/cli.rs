// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Command-line parsing (std-only, no clap). Every [`ScaffoldConfig`] field has
//! a flag; the values themselves are checked by `generate_scaffold`, which
//! refuses the same things whether it is called from here or from code.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use quack_rs::scaffold::{ScaffoldConfig, DEFAULT_EXCLUDED_PLATFORMS, REF_PLACEHOLDER};

/// What to generate, and where.
#[derive(Debug)]
pub struct Args {
    /// The project's settings.
    pub config: ScaffoldConfig,
    /// The directory to write into: `--dir`, or `./<name>`.
    pub dir: PathBuf,
    /// Overwrite files the scaffold generates that already exist.
    pub force: bool,
}

/// What the command line asks for.
#[derive(Debug)]
pub enum Command {
    /// `-h` / `--help`.
    Help,
    /// Generate a project.
    Run(Box<Args>),
}

/// The `--help` text.
pub fn help(prog: &str) -> String {
    format!(
        "Usage: {prog} <name> --description <TEXT> --maintainer <NAME> --github-repo <OWNER/REPO> [OPTIONS]

Generate a DuckDB extension project in Rust (C extension API, no C++): Cargo.toml,
src/lib.rs with an example scalar function, a SQLLogicTest, a Makefile driving
extension-ci-tools, description.yml for the community repository, CI workflows,
a README, the license text and deny.toml.

Arguments:
  <name>   Extension name: lowercase ASCII letters, digits and '_', starting with a
           letter. Also the [lib] name and the symbol prefix DuckDB looks for, so it
           cannot be a name DuckDB itself uses (an alias such as s3, or a core
           extension such as json).

Options (each value can also be given as --option=VALUE):
  --description <TEXT>            What the extension does  [required]
  --maintainer <NAME>             Primary maintainer, one line  [required]
  --github-repo <OWNER/REPO>      The extension's repository  [required]
  --version <SEMVER>              Package and extension version  [default: 0.1.0]
  --license <SPDX>                SPDX identifier or expression, e.g. 'MIT OR Apache-2.0'.
                                  The text is written for MIT, Apache-2.0, BSD-2-Clause,
                                  BSD-3-Clause, ISC, 0BSD, MIT-0 and Unlicense; for
                                  anything else, a LICENSE.TODO  [default: MIT]
  --excluded-platforms <LIST>     Platforms the community build skips, separated by ';'
                                  or ',' (an empty value excludes none)
                                  [default: {excluded}]
  --target-duckdb-version <VER>   TARGET_DUCKDB_VERSION. Without --unstable-c-api, the C
                                  extension API version (NOT a DuckDB release): {api} for
                                  the stable API, which every DuckDB since v1.2.0 loads,
                                  or {api156} to use quack-rs's duckdb-1-5 features, which
                                  DuckDB v1.5.6 and v2.0.0 load. With --unstable-c-api,
                                  the exact DuckDB 1.x release the binary is for
                                  (e.g. v1.5.5)  [default: {api}]
  --unstable-c-api                Stamp the binary C_STRUCT_UNSTABLE: it loads into
                                  exactly --target-duckdb-version and no other release
  --git-ref <REV>                 repo.ref in description.yml: the commit hash the
                                  community build builds  [default: {placeholder}]
  --dir <PATH>                    Where to write the project  [default: ./<name>]
  --force                         Overwrite files the scaffold generates that already
                                  exist in --dir (without it, an existing one is an error
                                  and nothing is written). Other files are left alone.
  -h, --help                      Print this help message",
        excluded = DEFAULT_EXCLUDED_PLATFORMS.join(";"),
        api = quack_rs::DUCKDB_API_VERSION,
        api156 = quack_rs::DUCKDB_API_VERSION_1_5_6,
        placeholder = REF_PLACEHOLDER,
    )
}

/// The value slots of [`parse`], one per value-taking flag.
#[derive(Default)]
struct Values {
    description: Option<String>,
    maintainer: Option<String>,
    github_repo: Option<String>,
    version: Option<String>,
    license: Option<String>,
    excluded_platforms: Option<String>,
    target_duckdb_version: Option<String>,
    git_ref: Option<String>,
}

/// Parses `raw` (without the program name).
///
/// Arguments are `OsString`s so that a non-UTF-8 one is an error message,
/// not a panic; only the `--dir` path may be one (given as `--dir PATH`).
///
/// # Errors
///
/// A message for a missing, unknown, repeated or non-UTF-8 argument.
pub fn parse(raw: &[OsString]) -> Result<Command, String> {
    let mut values = Values::default();
    let mut name: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    let (mut unstable, mut force) = (false, false);

    let mut iter = raw.iter();
    while let Some(raw_arg) = iter.next() {
        let arg = utf8(raw_arg)?;
        // `--flag=value` is the same as `--flag value`.
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (arg, None),
        };
        if inline.is_some() && matches!(flag, "--help" | "--unstable-c-api" | "--force") {
            return Err(format!("{flag} takes no value"));
        }
        let slot = match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "--unstable-c-api" => {
                unstable = true;
                continue;
            }
            "--force" => {
                force = true;
                continue;
            }
            "--dir" => {
                let path = match inline {
                    Some(value) => OsString::from(value),
                    None => iter
                        .next()
                        .filter(|v| !v.to_string_lossy().starts_with('-'))
                        .ok_or("--dir requires a value")?
                        .clone(),
                };
                if path.is_empty() {
                    return Err("--dir requires a value".to_string());
                }
                if dir.replace(PathBuf::from(path)).is_some() {
                    return Err("--dir given more than once".to_string());
                }
                continue;
            }
            "--description" => &mut values.description,
            "--maintainer" => &mut values.maintainer,
            "--github-repo" => &mut values.github_repo,
            "--version" => &mut values.version,
            "--license" => &mut values.license,
            "--excluded-platforms" => &mut values.excluded_platforms,
            "--target-duckdb-version" => &mut values.target_duckdb_version,
            "--git-ref" => &mut values.git_ref,
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("unknown flag: {other}"));
            }
            positional => {
                if let Some(first) = name.replace(positional.to_string()) {
                    return Err(format!(
                        "expected one <name>, got {first:?} and {positional:?}"
                    ));
                }
                continue;
            }
        };
        // A following flag is not a value; `--flag=-value` is taken as given.
        let value = match inline {
            Some(value) => value,
            None => match iter.next().map(|v| utf8(v)).transpose()? {
                Some(v) if !v.starts_with('-') => v,
                Some(v) => return Err(format!("{flag} requires a value, got the flag {v:?}")),
                None => return Err(format!("{flag} requires a value")),
            },
        };
        if slot.replace(value.to_string()).is_some() {
            return Err(format!("{flag} given more than once"));
        }
    }

    let name = name.ok_or("expected the extension <name>")?;
    let required =
        |value: Option<String>, flag: &str| value.ok_or_else(|| format!("{flag} is required"));
    let defaults = ScaffoldConfig::default();
    let config = ScaffoldConfig {
        description: required(values.description, "--description")?,
        maintainer: required(values.maintainer, "--maintainer")?,
        github_repo: required(values.github_repo, "--github-repo")?,
        version: values.version.unwrap_or(defaults.version),
        license: values.license.unwrap_or(defaults.license),
        excluded_platforms: values
            .excluded_platforms
            .map_or(defaults.excluded_platforms, |list| platform_list(&list)),
        target_duckdb_version: values
            .target_duckdb_version
            .unwrap_or(defaults.target_duckdb_version),
        use_unstable_c_api: unstable,
        git_ref: values.git_ref.unwrap_or(defaults.git_ref),
        name: name.clone(),
    };
    Ok(Command::Run(Box::new(Args {
        config,
        dir: dir.unwrap_or_else(|| PathBuf::from(name)),
        force,
    })))
}

/// `wasm_mvp;wasm_eh` or `wasm_mvp, wasm_eh` → the names, without empty ones.
fn platform_list(list: &str) -> Vec<String> {
    list.split([';', ','])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

fn utf8(arg: &OsStr) -> Result<&str, String> {
    arg.to_str().ok_or_else(|| {
        format!(
            "argument {:?} is not valid UTF-8 (only the --dir path may be, given as --dir PATH)",
            arg.to_string_lossy()
        )
    })
}
