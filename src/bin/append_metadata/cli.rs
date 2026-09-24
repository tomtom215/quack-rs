// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Command-line parsing and validation (std-only, no clap).

use std::path::PathBuf;

use crate::footer::VALID_ABI_TYPES;

#[derive(Debug)]
pub struct Args {
    pub input: PathBuf,
    pub output: PathBuf,
    pub abi_type: String,
    pub extension_version: String,
    pub duckdb_version: String,
    pub platform: String,
    pub dump: bool,
    /// Replace a footer the input already carries instead of refusing.
    pub replace: bool,
    /// Precede the footer with the WebAssembly custom-section header.
    pub wasm: bool,
    /// Non-fatal notes for stderr.
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum Command {
    Help,
    Run(Args),
}

pub fn help(prog: &str) -> String {
    let host = host_platform().unwrap_or("(none: --platform is required on this host)");
    format!(
        "Usage: {prog} <input> <output> [OPTIONS]

Append a DuckDB extension metadata footer to a compiled shared library.

Arguments:
  <input>   Input .so / .dylib / .dll / .wasm file
  <output>  Output .duckdb_extension file

Options (each value can also be given as --option=VALUE):
  --abi-type <TYPE>            C_STRUCT | CPP | C_STRUCT_UNSTABLE  [default: C_STRUCT]
  --extension-version <VER>    Your extension's version (e.g. v0.1.0)  [default: v0.1.0]
  --duckdb-version <VER>       C_STRUCT: the C extension API version, at most {api}
                               (NOT a DuckDB release -- see LESSONS.md P2).
                               CPP / C_STRUCT_UNSTABLE: the exact DuckDB release the
                               binary is for (e.g. v1.5.5, or a dev build's commit hash).
                               [default: {api}]
  --platform <PLATFORM>        DuckDB platform the binary was built for, e.g. linux_amd64,
                               linux_arm64, osx_arm64, windows_amd64, wasm_eh. DuckDB
                               refuses a file whose platform differs from its own, so a
                               group name (linux, osx, wasm, windows) is an error.
                               [default: this host, {host}]
  --wasm                       Precede the footer with the 22-byte WebAssembly custom
                               section header extension-ci-tools writes, so a .wasm
                               module stays valid. Required with a wasm_* platform (it
                               is an error without it); a warning with any other.
  --replace                    The input already ends in a footer: replace it instead of
                               refusing (without this, re-stamping is an error, because
                               appending a second footer leaves the first inside the file).
  --dump                       Print metadata fields after writing
  -h, --help                   Print this help message",
        api = quack_rs::DUCKDB_API_VERSION,
    )
}

/// Parses `raw` (without the program name).
///
/// # Errors
///
/// A message for a missing, unknown, repeated or invalid argument.
pub fn parse(raw: &[String]) -> Result<Command, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut abi_type: Option<&str> = None;
    let mut extension_version: Option<&str> = None;
    let mut duckdb_version: Option<&str> = None;
    let mut platform: Option<&str> = None;
    let (mut dump, mut replace, mut wasm) = (false, false, false);

    let mut iter = raw.iter().map(String::as_str);
    while let Some(raw_arg) = iter.next() {
        // `--flag=value` is the same as `--flag value`.
        let (arg, inline) = match raw_arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (raw_arg, None),
        };
        if inline.is_some() && matches!(arg, "--help" | "--dump" | "--replace" | "--wasm") {
            return Err(format!("{arg} takes no value"));
        }
        let slot = match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "--dump" => {
                dump = true;
                continue;
            }
            "--replace" => {
                replace = true;
                continue;
            }
            "--wasm" => {
                wasm = true;
                continue;
            }
            "--abi-type" => &mut abi_type,
            "--extension-version" => &mut extension_version,
            "--duckdb-version" => &mut duckdb_version,
            "--platform" => &mut platform,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown flag: {flag}"))
            }
            value => {
                positional.push(value);
                continue;
            }
        };
        // A following flag is not a value: `--platform --dump` used to stamp
        // the platform "--dump". An inline `--flag=value` is taken as given.
        let value = match inline {
            Some("") => return Err(format!("{arg} requires a value")),
            Some(v) => v,
            None => match iter.next() {
                Some(v) if !v.starts_with('-') => v,
                Some(v) => return Err(format!("{arg} requires a value, got the flag {v:?}")),
                None => return Err(format!("{arg} requires a value")),
            },
        };
        if slot.replace(value).is_some() {
            return Err(format!("{arg} given more than once"));
        }
    }

    let (input, output) = match positional.as_slice() {
        [input, output] => (*input, *output),
        [] | [_] => return Err("expected positional arguments: <input> <output>".to_string()),
        [_, _, extra @ ..] => {
            return Err(format!(
            "expected exactly two positional arguments (<input> <output>), got {} more: {extra:?}",
            extra.len()
        ))
        }
    };

    let abi_type = abi_type.unwrap_or("C_STRUCT");
    if !VALID_ABI_TYPES.contains(&abi_type) {
        return Err(format!(
            "--abi-type must be one of {VALID_ABI_TYPES:?}, got {abi_type:?}"
        ));
    }

    let extension_version = extension_version.unwrap_or("v0.1.0");
    if extension_version.is_empty() || extension_version.contains(char::is_whitespace) {
        return Err(format!(
            "--extension-version must be non-empty with no whitespace, got {extension_version:?}"
        ));
    }

    let mut warnings = Vec::new();
    let duckdb_version = duckdb_version.unwrap_or(quack_rs::DUCKDB_API_VERSION);
    check_duckdb_version(abi_type, duckdb_version, &mut warnings)?;

    let platform = resolve_platform(platform, &mut warnings)?;
    check_wasm(platform, wasm, &mut warnings)?;

    Ok(Command::Run(Args {
        input: PathBuf::from(input),
        output: PathBuf::from(output),
        abi_type: abi_type.to_string(),
        extension_version: extension_version.to_string(),
        duckdb_version: duckdb_version.to_string(),
        platform: platform.to_string(),
        dump,
        replace,
        wasm,
        warnings,
    }))
}

/// A WebAssembly module must carry the footer inside a custom section;
/// appended bare, it is not a valid module (`WebAssembly.validate` is false)
/// and DuckDB-Wasm cannot load it. So a `wasm_*` platform needs `--wasm`, and
/// `--wasm` on any other platform is warned about.
fn check_wasm(platform: &str, wasm: bool, warnings: &mut Vec<String>) -> Result<(), String> {
    let wasm_platform = platform.starts_with("wasm");
    if wasm_platform && !wasm {
        return Err(format!(
            "--platform {platform} stamps a WebAssembly module: pass --wasm, or the output is \
             not a valid module"
        ));
    }
    if wasm && !wasm_platform {
        warnings.push(format!(
            "--wasm wraps the footer in a WebAssembly custom section, but --platform \
             {platform} is not a wasm platform"
        ));
    }
    Ok(())
}

/// The `--platform` value, or the host's when it was not given.
fn resolve_platform<'a>(
    platform: Option<&'a str>,
    warnings: &mut Vec<String>,
) -> Result<&'a str, String> {
    let platform = match platform {
        Some(p) => p,
        None => host_platform().ok_or(
            "--platform is required: this host is not a platform DuckDB publishes extensions for",
        )?,
    };
    if platform.is_empty()
        || !platform
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(format!(
            "--platform must be lowercase letters, digits and '_' (e.g. linux_amd64), got {platform:?}"
        ));
    }
    // `linux`, `osx`, `wasm` and `windows` are legal in `excluded_platforms`,
    // where they mean "the whole group", but no DuckDB reports one as its
    // platform, so a file stamped with one never loads.
    if quack_rs::validate::DUCKDB_PLATFORM_GROUPS.contains(&platform) {
        return Err(format!(
            "--platform {platform:?} is a platform group, not a platform: DuckDB refuses a file \
             stamped with it. Name the exact platform, e.g. {platform}_amd64 or wasm_eh"
        ));
    }
    if !quack_rs::validate::DUCKDB_CI_PLATFORMS.contains(&platform) {
        warnings.push(format!(
            "platform {platform:?} is not one DuckDB's community CI builds; DuckDB only loads the \
             file if it reports exactly this platform"
        ));
    }
    Ok(platform)
}

/// `vMAJOR.MINOR.PATCH` → its three numbers.
fn parse_release(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.strip_prefix('v')?.split('.');
    let mut next = || -> Option<u64> {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let parsed = (next()?, next()?, next()?);
    parts.next().is_none().then_some(parsed)
}

/// Checks `--duckdb-version` against what `DuckDB`'s loader does with it.
///
/// For `C_STRUCT`, `DuckDB` parses it as a semver C API version and refuses the
/// file unless the major matches and minor/patch are at most its own
/// (`VersioningUtils::IsSupportedCAPIVersion`); quack-rs extensions request
/// [`quack_rs::DUCKDB_API_VERSION`], so anything above that is an error — in
/// practice, a `DuckDB` release number written where the C API version belongs
/// (LESSONS.md P2). For `CPP` / `C_STRUCT_UNSTABLE` it is compared with the
/// engine's version string: a release tag, or a commit hash for dev builds.
fn check_duckdb_version(
    abi_type: &str,
    version: &str,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let api = quack_rs::DUCKDB_API_VERSION;
    let api_parsed = parse_release(api).ok_or("DUCKDB_API_VERSION is malformed")?;
    if abi_type == "C_STRUCT" {
        let Some(parsed) = parse_release(version) else {
            return Err(format!(
                "--duckdb-version for C_STRUCT must be a C API version like {api}, got {version:?}"
            ));
        };
        if parsed.0 != api_parsed.0 || parsed > api_parsed {
            return Err(format!(
                "--duckdb-version {version} is not a C API version this crate targets: for \
                 C_STRUCT the field is the C extension API version (at most {api}), not a DuckDB \
                 release, and DuckDB refuses to load a file claiming a C API newer than its own \
                 (LESSONS.md P2). Use --duckdb-version {api}, or --abi-type C_STRUCT_UNSTABLE to \
                 pin the binary to DuckDB {version}."
            ));
        }
        return Ok(());
    }
    let is_hash = (7..=40).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    if parse_release(version).is_none() && !is_hash {
        return Err(format!(
            "--duckdb-version for {abi_type} must be the exact DuckDB release (e.g. v1.5.5) or a \
             dev build's commit hash, got {version:?}"
        ));
    }
    if version == api {
        warnings.push(format!(
            "--duckdb-version {api} with {abi_type} pins the binary to DuckDB release {api}; \
             {api} is also the C API version -- make sure a DuckDB release was meant"
        ));
    }
    Ok(())
}

/// The `DuckDB` platform name of the machine this tool was built for, when it
/// is one `DuckDB` publishes extensions for.
///
/// This is the platform the library being stamped was most likely built for
/// too; cross-compiled libraries need an explicit `--platform`.
pub const fn host_platform() -> Option<&'static str> {
    let arm = cfg!(target_arch = "aarch64");
    if !arm && !cfg!(target_arch = "x86_64") {
        return None;
    }
    Some(if cfg!(target_os = "linux") {
        match (arm, cfg!(target_env = "musl")) {
            (false, false) => "linux_amd64",
            (false, true) => "linux_amd64_musl",
            (true, false) => "linux_arm64",
            (true, true) => "linux_arm64_musl",
        }
    } else if cfg!(target_os = "macos") {
        if arm {
            "osx_arm64"
        } else {
            "osx_amd64"
        }
    } else if cfg!(target_os = "windows") {
        match (arm, cfg!(target_env = "gnu")) {
            (false, false) => "windows_amd64",
            (false, true) => "windows_amd64_mingw",
            (true, false) => "windows_arm64",
            (true, true) => "windows_arm64_mingw",
        }
    } else {
        return None;
    })
}
