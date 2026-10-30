// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Corpus regression: fixtures shaped like real published `description.yml`
//! files that the parser used to reject or misread.
//!
//! Each fixture in `fixtures/` reproduces the structure of one or more files
//! in `duckdb/community-extensions` (names and values are made up; the shapes
//! are not). Against all 346 published files the parser previously rejected
//! 29; it now accepts all 346 and agrees with `yaml.safe_load` on every field
//! it reads. The expected values below are what `yaml.safe_load` returns for
//! each fixture.

use super::*;

fn parse(fixture: &str) -> DescriptionYml {
    parse_description_yml(fixture).unwrap_or_else(|e| panic!("{e}"))
}

fn has_warning(desc: &DescriptionYml, needle: &str) -> bool {
    desc.warnings.iter().any(|w| w.contains(needle))
}

/// Shaped like `mssql` / `mssql_ducklake`: every value quoted, the British
/// `licence:` key, a block-sequence `excluded_platforms`, comments between
/// keys and prose in `docs:` that looks like metadata.
#[test]
fn quoted_values_licence_spelling_and_list_exclusions() {
    let desc = parse(include_str!("fixtures/list_exclusions_and_licence.yml"));
    assert_eq!(desc.name, "sql_bridge");
    assert_eq!(desc.version.as_deref(), Some("0.2.5"));
    assert_eq!(desc.language, "C++");
    assert_eq!(desc.license, "MIT");
    assert_eq!(desc.maintainers, vec!["Jane Doe", "bob # not a comment"]);
    assert_eq!(desc.excluded_platforms, "osx_amd64;windows_arm64;wasm_mvp");
    assert_eq!(desc.github, "example-org/sql-bridge-extension");
    assert_eq!(desc.git_ref, "d35b617c16587e02b4d6e62057ead8f84c1568bf");
    assert!(has_warning(&desc, "licence"), "{:?}", desc.warnings);
    assert!(
        has_warning(&desc, "excluded_platforms is a YAML list"),
        "{:?}",
        desc.warnings
    );
    assert_eq!(desc.warnings.len(), 2, "{:?}", desc.warnings);
}

/// Shaped like `anofox_tabular` and the other 11 files with no `version:`, and
/// the 10 declaring `BSL 1.1` (not an SPDX identifier; `BUSL-1.1` is).
#[test]
fn missing_version_and_a_non_spdx_license() {
    let desc = parse(include_str!("fixtures/no_version_bsl_license.yml"));
    assert_eq!(desc.version, None);
    assert_eq!(desc.license, "BSL 1.1");
    assert_eq!(
        desc.excluded_platforms,
        "windows_amd64_rtools;windows_amd64_mingw;wasm_mvp;wasm_eh;wasm_threads;linux_arm64;\
         linux_amd64_musl;"
    );
    assert!(has_warning(&desc, "'BSL"), "{:?}", desc.warnings);
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(validate_rust_extension(&desc).is_ok());
}

/// Shaped like `yardstick` (`linux_amd64_gcc4`), `duckdb_opendalfs`
/// (`windows_arm64_mingw`) and `gaggle` (`MIT OR Apache-2.0`).
#[test]
fn spdx_expression_and_retired_or_extra_platforms() {
    let desc = parse(include_str!(
        "fixtures/spdx_expression_retired_platforms.yml"
    ));
    assert_eq!(desc.license, "MIT OR Apache-2.0");
    assert!(desc
        .excluded_platforms
        .contains("linux_amd64_gcc4;windows_amd64_rtools;windows_amd64_mingw;windows_arm64_mingw"));
    assert_eq!(
        desc.git_ref_next,
        "0123456789abcdef0123456789abcdef01234567"
    );
    // The expression is fine; excluding a retired platform is merely pointless.
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(
        has_warning(&desc, "linux_amd64_gcc4"),
        "{:?}",
        desc.warnings
    );
}

/// Shaped like `bitfilters` and `shellfs`: `docs:` first, keys in alphabetical
/// order, maintainers indented level with their key, a single-quoted
/// date version, and a double-quoted description continued with `\` escaped
/// line breaks.
#[test]
fn docs_first_compact_sequence_and_escaped_line_breaks() {
    let desc = parse(include_str!("fixtures/docs_first_escaped_newline.yml"));
    assert_eq!(desc.name, "filters");
    assert_eq!(desc.version.as_deref(), Some("2025120401"));
    assert_eq!(
        desc.description,
        "Provides probabilistic data structures -including quotient and XOR filters - for fast \
         approximate set membership testing with no \"false\" negatives."
    );
    assert_eq!(desc.maintainers, vec!["rustyconover"]);
    assert_eq!(desc.requires_toolchains, "rust;python3");
    assert!(desc.warnings.is_empty(), "{:?}", desc.warnings);
}

/// The per-release revisions: 133 of the 395 published files (community-
/// extensions `845779e`) set `repo.variegata`, 76 `repo.andium`, all as
/// commit hashes. `scripts/build.py` checks out `variegata` for `DuckDB`
/// v1.5.6 and `andium` for v1.4.5 whenever the key is present, so a present
/// but empty one leaves that build with no revision at all.
#[test]
fn per_release_refs_are_read_and_checked() {
    let base = "extension:\n  name: my_ext\n  description: d\n  language: Rust\n  build: cargo\n  \
                license: MIT\n  maintainers:\n    - a\nrepo:\n  github: a/b\n  \
                ref: 0123456789abcdef0123456789abcdef01234567\n";
    let desc = parse(&format!(
        "{base}  variegata: 1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a\n  andium: 2b2b2b2\n"
    ));
    assert_eq!(
        desc.git_ref_variegata,
        "1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a"
    );
    assert_eq!(desc.git_ref_andium, "2b2b2b2");
    assert!(desc.warnings.is_empty(), "{:?}", desc.warnings);

    let desc = parse(base);
    assert_eq!(
        (
            desc.git_ref_variegata.as_str(),
            desc.git_ref_andium.as_str()
        ),
        ("", "")
    );

    for empty in ["  variegata:\n", "  andium: \"\"\n", "  variegata: ~\n"] {
        let err = parse_description_yml(&format!("{base}{empty}")).expect_err(empty);
        assert!(err.as_str().contains("present but empty"), "{empty}: {err}");
    }
    let desc = parse(&format!("{base}  variegata: v1.5-branch\n"));
    assert!(
        has_warning(&desc, "repo.variegata 'v1.5-branch' is not a commit hash"),
        "{:?}",
        desc.warnings
    );
}

/// `repo.github` goes to `actions/checkout` as its `repository`, which takes
/// `owner/repo` and nothing else; a URL or an extra segment used to pass
/// because only a `/` was required. All 395 published values are plain
/// `owner/repo`.
#[test]
fn repo_github_must_be_owner_slash_repo() {
    let yml = |github: &str| {
        format!(
            "extension:\n  name: my_ext\n  description: d\n  language: Rust\n  build: cargo\n  \
             license: MIT\n  maintainers:\n    - a\nrepo:\n  github: {github}\n  ref: 0a1b2c3\n"
        )
    };
    for bad in [
        "https://github.com/me/demo",
        "github.com/me/demo",
        "me/demo/x",
        "/demo",
        "me/",
        "-me/demo",
        "me/..",
        "me/de mo",
    ] {
        let err = parse_description_yml(&yml(bad)).expect_err(bad);
        assert!(err.as_str().contains("owner/repo"), "{bad}: {err}");
    }
    for good in ["me/demo", "My-Org/duckdb.ext_2", "a/b"] {
        assert_eq!(parse(&yml(good)).github, good);
    }
}

/// `extension.name` may not be a name `DuckDB` uses itself. None of the 395
/// published files has one (checked over every name in community-extensions
/// `845779e`; `scripts/check-extension-names.py` repeats the check in CI over
/// duckdb.org's list of published extensions).
#[test]
fn a_reserved_extension_name_is_refused() {
    let yml = |name: &str| {
        format!(
            "extension:\n  name: {name}\n  description: d\n  language: Rust\n  build: cargo\n  \
             license: MIT\n  maintainers:\n    - a\nrepo:\n  github: a/b\n  ref: 0a1b2c3\n"
        )
    };
    for name in ["s3", "sqlite3", "json", "spatial"] {
        let err = parse_description_yml(&yml(name)).expect_err(name);
        assert!(err.as_str().contains("extension.name"), "{name}: {err}");
    }
    assert_eq!(parse(&yml("cache_httpfs")).name, "cache_httpfs");
}

/// `scripts/build.py` reads six `repo:` keys; anything else does nothing, and
/// one published file has `next:` where `ref_next:` was meant.
#[test]
fn a_repo_key_the_build_ignores_is_warned_about() {
    let yml = "extension:\n  name: my_ext\n  description: d\n  language: Rust\n  build: cargo\n  \
               license: MIT\n  maintainers:\n    - a\nrepo:\n  github: a/b\n  ref: 0a1b2c3\n  \
               next: 89abcdef\n  canonical_name: other\n";
    let desc = parse(yml);
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(has_warning(
        &desc,
        "repo.next: the community build does not read this key"
    ));
}

/// Shaped like `cloudfs` / `valhalla_routing` (a block-list
/// `requires_toolchains`) and `level_pivot` (`GPL-3.0`, an SPDX identifier
/// the registry has deprecated), plus a folded description and flow-sequence
/// maintainers.
#[test]
fn folded_description_flow_maintainers_and_toolchain_list() {
    let desc = parse(include_str!(
        "fixtures/folded_description_toolchain_list.yml"
    ));
    assert_eq!(
        desc.description,
        "Query remote file stores using cloud:// URLs, with the same capabilities as s3://.\n\
         Supports Parquet, CSV and JSON."
    );
    assert_eq!(desc.maintainers, vec!["Ann", "Bo, Jr."]);
    assert_eq!(desc.requires_toolchains, "cmake;openssl");
    assert_eq!(desc.git_ref, "v0.1.0");
    assert!(
        has_warning(&desc, "requires_toolchains is a YAML list"),
        "{:?}",
        desc.warnings
    );
    assert!(has_warning(&desc, "'GPL-3.0'"), "{:?}", desc.warnings);
    // It pins a tag, not a commit; the warning says why a tag is not enough
    // (it used to speak of a branch only, CONFORM-15).
    assert!(
        has_warning(&desc, "repo.ref 'v0.1.0' is not a commit hash"),
        "{:?}",
        desc.warnings
    );
    assert!(
        has_warning(&desc, "a branch or tag can be moved"),
        "{:?}",
        desc.warnings
    );
    assert_eq!(desc.warnings.len(), 3, "{:?}", desc.warnings);
}
