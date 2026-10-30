// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! What [`generate_scaffold`][super::generate_scaffold] checks before it
//! writes anything, and the warnings it gives about values it accepts.

use crate::error::ExtensionError;
use crate::validate::platform::{
    platforms_in_group, DUCKDB_CI_PLATFORMS, DUCKDB_OPT_IN_PLATFORMS, DUCKDB_PLATFORM_GROUPS,
};
use crate::validate::spdx::check_spdx_expression;
use crate::validate::{validate_community_extension_name, validate_semver};

use super::{pins_bindings, ScaffoldConfig};

/// Checks every field of `config`, refusing what would make the generated
/// project wrong and adding to `warnings` what is merely unusual.
pub(super) fn check_config(
    config: &ScaffoldConfig,
    warnings: &mut Vec<String>,
) -> Result<(), ExtensionError> {
    validate_community_extension_name(&config.name)?;
    // Semver, not the looser `validate_extension_version` that `description.yml`
    // alone would allow: the version is also `[package] version` in Cargo.toml.
    validate_semver(&config.version).map_err(|e| {
        ExtensionError::new(format!(
            "version {:?} is not usable as the Cargo package version: {e}",
            config.version
        ))
    })?;
    validate_cargo_version(&config.version)?;
    if let Some(unlisted) = check_spdx_expression(&config.license)? {
        warnings.push(unlisted);
    }
    validate_free_text("description", &config.description, true)?;
    validate_free_text("maintainer", &config.maintainer, false)?;
    validate_github_repo(&config.github_repo)?;
    validate_git_ref(&config.git_ref)?;
    check_excluded_platforms(&config.excluded_platforms, warnings)?;
    validate_target_duckdb_version(config)?;
    warn_about_the_community_build_of_an_unstable_binary(config, warnings);
    Ok(())
}

/// Checks that `version` is a version Cargo accepts as `[package] version`.
///
/// `SemVer` places no bound on a numeric component, so `validate_semver`
/// accepts `18446744073709551616.0.0`; Cargo parses components as `u64` and
/// refuses it ("value of major version number exceeds `u64::MAX`"), which would
/// leave the generated project unbuildable.
fn validate_cargo_version(version: &str) -> Result<(), ExtensionError> {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    for (label, part) in ["major", "minor", "patch"].iter().zip(core.split('.')) {
        if part.parse::<u64>().is_err() {
            return Err(ExtensionError::new(format!(
                "version {version:?}: the {label} component {part:?} exceeds u64::MAX, which \
                 Cargo refuses as a package version"
            )));
        }
    }
    Ok(())
}

/// Checks `excluded_platforms` as the community build applies it.
///
/// The build excludes a platform only when its exact name is listed (the Go
/// `extbuild matrix` on `v2.0-cyanoptera`, `modify_distribution_matrix.py`
/// for the deploy step), and the generated distribution workflow passes the
/// same list as `exclude_archs`. So a name the build does not build excludes
/// nothing, and is refused here: a group such as `wasm` (named with the
/// platforms it stands for), and the legacy names `parse_description_yml`
/// accepts in published files (`linux_amd64_gcc4`, `windows_amd64_rtools`,
/// `windows_arm64_mingw`). An opt-in platform is never built unless an
/// extension asks for it, so excluding one is a warning, as is excluding
/// everything that would be built.
fn check_excluded_platforms(
    platforms: &[String],
    warnings: &mut Vec<String>,
) -> Result<(), ExtensionError> {
    let names: Vec<&str> = platforms.iter().map(String::as_str).collect();
    if names.iter().any(|p| p.is_empty()) {
        return Err(ExtensionError::new(
            "excluded_platforms contains an empty name",
        ));
    }
    crate::validate::validate_excluded_platforms(&names)?;
    for &platform in &names {
        if DUCKDB_PLATFORM_GROUPS.contains(&platform) {
            return Err(ExtensionError::new(format!(
                "excluded_platforms: '{platform}' is a platform group, not a platform. The \
                 community build compares exact platform names, so it would exclude nothing; \
                 list the platforms instead: {}",
                platforms_in_group(platform).join(", ")
            )));
        }
        if !DUCKDB_CI_PLATFORMS.contains(&platform) {
            return Err(ExtensionError::new(format!(
                "excluded_platforms: '{platform}' is not a platform the community build \
                 builds, so excluding it has no effect; remove it (the platforms are: {})",
                DUCKDB_CI_PLATFORMS.join(", ")
            )));
        }
        if DUCKDB_OPT_IN_PLATFORMS.contains(&platform) {
            warnings.push(format!(
                "excluded_platforms: '{platform}' is opt-in: the community build only builds \
                 it for an extension that lists it in opt_in_platforms, so excluding it does \
                 nothing"
            ));
        }
    }
    let built = DUCKDB_CI_PLATFORMS
        .iter()
        .filter(|p| !DUCKDB_OPT_IN_PLATFORMS.contains(p))
        .all(|p| names.contains(p));
    if built {
        warnings.push(
            "excluded_platforms excludes every platform the community build builds by \
             default: nothing would be built or published"
                .to_string(),
        );
    }
    Ok(())
}

/// A `C_STRUCT_UNSTABLE` binary loads into one `DuckDB` 1.x release only, and
/// the community build builds `repo.ref` for the newest release (v2.0.0),
/// `repo.variegata` for v1.5.6 and `repo.andium` for v1.4.5
/// (`scripts/build.py`). So such a project cannot be submitted as generated:
/// say what it needs.
fn warn_about_the_community_build_of_an_unstable_binary(
    config: &ScaffoldConfig,
    warnings: &mut Vec<String>,
) {
    if !config.use_unstable_c_api {
        return;
    }
    let target = config.target_duckdb_version.as_str();
    let key = match target {
        "v1.5.6" => Some("variegata"),
        "v1.4.5" => Some("andium"),
        _ => None,
    };
    let latest = super::DISTRIBUTION_DUCKDB_VERSION;
    warnings.push(key.map_or_else(
        || {
            format!(
                "use_unstable_c_api: the binary loads into DuckDB {target} only, and the \
                 community build builds no extension for {target} (it builds repo.ref for \
                 {latest}, repo.variegata for v1.5.6 and repo.andium for v1.4.5); such a \
                 project can be distributed only outside the community repository"
            )
        },
        |key| {
            format!(
                "use_unstable_c_api: the binary loads into DuckDB {target} only, but the \
                 community build builds repo.ref for DuckDB {latest}, which refuses to load \
                 it. Submit this revision as repo.{key} (built for {target}), and point \
                 repo.ref at a revision stamped C_STRUCT (C API v1.2.0 or v1.5.6) that DuckDB \
                 2.0 loads"
            )
        },
    ));
}

/// Checks that `target_duckdb_version` is a `vX.Y.Z` string and is consistent
/// with `use_unstable_c_api`.
///
/// The two settings are coupled: `extension-ci-tools` feeds
/// `TARGET_DUCKDB_VERSION` to `append_extension_metadata.py -dv`, where it means
/// the C API version for a `C_STRUCT` build and an exact `DuckDB` release for a
/// `C_STRUCT_UNSTABLE` one. Getting this wrong yields a binary `DuckDB` silently
/// refuses to load ("built specifically for `DuckDB` version …").
fn validate_target_duckdb_version(config: &ScaffoldConfig) -> Result<(), ExtensionError> {
    let version = config.target_duckdb_version.as_str();

    let numeric = version.strip_prefix('v').ok_or_else(|| {
        ExtensionError::new(format!(
            "target_duckdb_version must start with 'v' (e.g. \"v1.5.5\"), got {version:?}"
        ))
    })?;
    let parts: Vec<&str> = numeric.split('.').collect();
    let well_formed = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !well_formed {
        return Err(ExtensionError::new(format!(
            "target_duckdb_version must be 'vMAJOR.MINOR.PATCH' (e.g. \"v1.5.5\"), got {version:?}"
        )));
    }

    let stable_api = crate::DUCKDB_API_VERSION;
    let api_1_5_6 = crate::DUCKDB_API_VERSION_1_5_6;
    if !config.use_unstable_c_api && version != stable_api && version != api_1_5_6 {
        return Err(ExtensionError::new(format!(
            "with use_unstable_c_api = false the extension is stamped C_STRUCT, so \
             target_duckdb_version is the C extension API version and must be {stable_api:?} \
             (stable API only) or {api_1_5_6:?} (quack-rs's duckdb-1-5 features; loads into \
             DuckDB v1.5.6 and v2.0.0); got {version:?}. Set use_unstable_c_api = true to pin \
             the binary to DuckDB {version} instead."
        )));
    }

    if config.use_unstable_c_api && parts[0].parse::<u64>().is_ok_and(|major| major >= 2) {
        return Err(ExtensionError::new(format!(
            "with use_unstable_c_api = true the extension is stamped C_STRUCT_UNSTABLE, and \
             DuckDB {version} loads such a binary only through the C API v2 entry point \
             (<name>_init_c_api_v2), which quack-rs does not generate. Set \
             use_unstable_c_api = false and target_duckdb_version = {api_1_5_6:?}: DuckDB \
             v1.5.6 and v2.0.0 load that, duckdb-1-5 features included."
        )));
    }

    if config.use_unstable_c_api && version == crate::DUCKDB_API_VERSION {
        return Err(ExtensionError::new(format!(
            "with use_unstable_c_api = true the extension is stamped C_STRUCT_UNSTABLE and \
             DuckDB requires target_duckdb_version to be the exact DuckDB release it will be \
             loaded into (e.g. \"v1.5.5\"); {:?} is the C extension API version, not a \
             DuckDB release to pin to.",
            crate::DUCKDB_API_VERSION
        )));
    }

    if pins_bindings(config) {
        libduckdb_sys_requirement(version)?;
    }

    Ok(())
}

/// Checks a free-text field (`description`, `maintainer`) before it is written
/// into `description.yml` and, for the description, a `//!` doc comment.
///
/// Punctuation that means something to YAML (`: `, ` #`, quotes, backslashes)
/// is fine — the templates quote and escape it. What is refused is what no
/// escaping makes right: an empty value; leading or trailing whitespace, which
/// `description.yml` readers trim; control characters (a bare carriage return
/// is a hard error in a Rust doc comment); Unicode bidirectional controls,
/// which rustc denies in comments (`text_direction_codepoint_in_comment`); and,
/// in a single-line field, any line break.
fn validate_free_text(field: &str, text: &str, multi_line: bool) -> Result<(), ExtensionError> {
    if text.trim().is_empty() {
        return Err(ExtensionError::new(format!("{field} must not be empty")));
    }
    if text.trim() != text {
        return Err(ExtensionError::new(format!(
            "{field} must not start or end with whitespace: {text:?}"
        )));
    }
    let line_break = |c: char| matches!(c, '\n' | '\u{2028}' | '\u{2029}');
    if let Some(bad) = text.chars().find(|&c| {
        (c.is_control() && c != '\t' && !(multi_line && c == '\n'))
            || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            || (!multi_line && line_break(c))
    }) {
        return Err(ExtensionError::new(format!(
            "{field} contains the character {bad:?}, which cannot be written into the \
             generated files{}",
            if multi_line {
                ""
            } else {
                "; it must be a single line"
            }
        )));
    }
    Ok(())
}

/// `owner/repo`, in the characters GitHub allows: the value is written into
/// `description.yml`.
fn validate_github_repo(repo: &str) -> Result<(), ExtensionError> {
    if crate::validate::is_github_repo(repo) {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "github_repo must be 'owner/repo' (letters, digits, '-', and '_' / '.' in the \
             repository name), got {repo:?}"
        )))
    }
}

/// A commit hash or tag — anything else is not a revision `repo.ref` can name.
fn validate_git_ref(git_ref: &str) -> Result<(), ExtensionError> {
    let valid = !git_ref.is_empty()
        && !git_ref.starts_with('-')
        && git_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    if valid {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "git_ref must be a commit hash or tag (letters, digits, '-', '_', '.', '/'), \
             got {git_ref:?}"
        )))
    }
}

/// The `libduckdb-sys` version requirement that compiles against exactly the
/// `DuckDB` release `target` (`vX.Y.Z`), for a build that [pins its
/// bindings](pins_bindings). Only 1.x targets reach it: a 2.x one is refused
/// before (`C_STRUCT_UNSTABLE` cannot load into 2.x, and C API v1.5.6 maps to
/// the v1.5.6 bindings).
///
/// `libduckdb-sys` numbers its releases after the `DuckDB` release whose
/// headers it ships. From `DuckDB` 1.5.0 the scheme is
/// `1.(10000·major + 100·minor + patch).N` — 1.10505.0 is `DuckDB` v1.5.5,
/// 1.20000.0 is v2.0.0 — where `N`
/// is the bindings crate's own patch release, so `~1.10505.0` admits binding
/// fixes but never another `DuckDB` release. On the 1.4 line the crate version
/// *is* the `DuckDB` version, so the pin is exact. This is the inverse of the
/// mapping in `scripts/duckdb-version-from-lock.sh`.
///
/// # Errors
///
/// A release neither scheme can express, or one older than the 1.4.4 floor
/// quack-rs itself requires.
pub(super) fn libduckdb_sys_requirement(target: &str) -> Result<String, ExtensionError> {
    let numbers: Vec<u32> = target
        .trim_start_matches('v')
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| ExtensionError::new(format!("malformed DuckDB version {target:?}")))?;
    match numbers.as_slice() {
        [1, minor @ 5..=99, patch @ 0..=99] => Ok(format!("~1.{}.0", 10_000 + minor * 100 + patch)),
        [1, 4, patch] if *patch >= 4 => Ok(format!("=1.4.{patch}")),
        _ => Err(ExtensionError::new(format!(
            "cannot pin libduckdb-sys to DuckDB {target}: libduckdb-sys encodes DuckDB \
             1.Y.Z (Y >= 5) as 1.(10000 + 100*Y + Z) and ships 1.4.x as-is, and quack-rs \
             needs at least 1.4.4; no libduckdb-sys release is known to match {target}"
        ))),
    }
}
