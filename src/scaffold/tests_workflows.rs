// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Regression tests for defects a developer-experience run found by following
//! the book with a fresh scaffold: the version footer, the ignore file, the
//! MSRV, the distribution build, the workflow pins, the example's NULL
//! handling, and the free text that reaches `Cargo.toml`, `description.yml`
//! and the crate docs.

use super::*;
use crate::validate::description_yml::parse_description_yml;

fn valid_config() -> ScaffoldConfig {
    ScaffoldConfig {
        name: "my_analytics".to_string(),
        description: "Fast analytics functions".to_string(),
        maintainer: "Jane Doe".to_string(),
        github_repo: "janedoe/duckdb-my-analytics".to_string(),
        ..ScaffoldConfig::default()
    }
}

fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} not generated"))
        .content
}

const DISTRIBUTION: &str = ".github/workflows/MainDistributionPipeline.yml";
const NATIVE_CI: &str = ".github/workflows/extension-ci.yml";

/// H1: in a repository with no commit, extension-ci-tools wrote git's error
/// ("error: malformed object name 'HEAD'", 37 bytes) into
/// `configure/extension_version.txt`; it overflowed the footer's 32-byte field
/// and `DuckDB` refused the file ("Unknown ABI type for extension: 'TRUCT…'").
/// The Makefile must fall back to the Cargo version, via the
/// `EXTENSION_VERSION` variable `base.Makefile` reads, before including it.
#[test]
fn makefile_falls_back_to_the_cargo_version_without_a_commit() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = file(&files, "Makefile");
    let fallback = makefile
        .find("ifeq ($(shell git rev-parse --verify --quiet HEAD 2>/dev/null),)")
        .expect("a check for a commit");
    let assign = makefile
        .find(
            "EXTENSION_VERSION := v$(shell sed -n 's/^version *= *\"\\([^\"]*\\)\".*/\\1/p' \
             Cargo.toml | head -n 1)",
        )
        .expect("a fallback to the Cargo.toml version");
    let include = makefile
        .find("include extension-ci-tools/makefiles/c_api_extensions/base.Makefile")
        .expect("the include");
    assert!(fallback < assign && assign < include, "{makefile}");
    // An explicit EXTENSION_VERSION still wins.
    assert!(
        makefile.contains("ifeq ($(EXTENSION_VERSION),)"),
        "{makefile}"
    );
}

/// H2: `make configure` and `make test` leave `configure/` (a venv),
/// `duckdb_unittest_tempdir/` and `build/` behind, as in
/// `duckdb/extension-template-rs`'s ignore file.
#[test]
fn gitignore_covers_what_the_build_leaves_behind() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let gitignore = file(&files, ".gitignore");
    for entry in [
        "/target",
        "/build/",
        "/configure/",
        "/duckdb_unittest_tempdir/",
        "/test/bin/",
        "venv/",
    ] {
        assert!(
            gitignore.lines().any(|l| l == entry),
            "{entry}: {gitignore}"
        );
    }
    assert!(!gitignore.contains("Cargo.lock"), "{gitignore}");
}

/// H3: a fresh resolution picked zopfli 0.8.4 (rust-version 1.88) through
/// libduckdb-sys → zip, so the project did not build on Rust 1.86. The
/// manifest declares quack-rs's MSRV and resolves with resolver 3, and CI
/// builds on that toolchain.
#[test]
fn cargo_toml_declares_the_msrv_and_resolves_for_it() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cargo = file(&files, "Cargo.toml");
    let msrv = env!("CARGO_PKG_RUST_VERSION");
    assert!(
        cargo.contains(&format!("\nrust-version = \"{msrv}\"\n")),
        "{cargo}"
    );
    assert!(cargo.contains("\nresolver = \"3\"\n"), "{cargo}");
    let package = cargo.split("\n[lib]").next().unwrap_or_default();
    assert!(package.contains("resolver = \"3\""), "must be in [package]");

    let ci = file(&files, NATIVE_CI);
    assert!(ci.contains("\n  msrv:\n"), "{ci}");
    assert!(ci.contains(&format!("toolchain: '{msrv}'")), "{ci}");
}

/// H4: the generated CI built three platforms natively, while the community
/// build uses `_extension_distribution.yml` for every platform. The generated
/// pipeline calls the same workflow at an extension-ci-tools branch that
/// exists, and excludes exactly what `description.yml` excludes.
#[test]
fn distribution_workflow_matches_the_community_build() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let wf = file(&files, DISTRIBUTION);
    // community-extensions' build.yml (2026-10-21): `_extension_distribution.yml@
    // v2.0-cyanoptera` with `duckdb_version: v2.0.0`; extension-ci-tools has no
    // `v2.0.0` branch, so the two are different strings now.
    let r = DISTRIBUTION_CI_TOOLS_REF;
    assert_eq!(r, "v2.0-cyanoptera");
    assert_eq!(DISTRIBUTION_DUCKDB_VERSION, "v2.0.0");
    assert!(
        wf.contains(&format!(
            "uses: duckdb/extension-ci-tools/.github/workflows/_extension_distribution.yml@{r}\n"
        )),
        "{wf}"
    );
    assert!(
        wf.contains(&format!("duckdb_version: {DISTRIBUTION_DUCKDB_VERSION}\n")),
        "{wf}"
    );
    assert!(wf.contains(&format!("ci_tools_version: {r}\n")), "{wf}");
    assert!(wf.contains("extension_name: my_analytics\n"), "{wf}");
    assert!(wf.contains("extra_toolchains: rust;python3\n"), "{wf}");
    assert!(!wf.contains("@@"), "unfilled placeholder: {wf}");

    // The same branch everywhere the project names extension-ci-tools.
    assert!(file(&files, ".gitmodules").contains(&format!("\tbranch = {r}\n")));
    assert!(file(&files, "Makefile").contains(&format!(
        "git submodule add -b {r} https://github.com/duckdb/extension-ci-tools.git"
    )));
}

#[test]
fn exclude_archs_agrees_with_description_yml() {
    let cases: [Vec<String>; 3] = [
        ScaffoldConfig::default().excluded_platforms,
        vec![],
        vec!["linux_arm64".to_string(), "wasm_eh".to_string()],
    ];
    for excluded in cases {
        let config = ScaffoldConfig {
            excluded_platforms: excluded.clone(),
            ..valid_config()
        };
        let files = generate_scaffold(&config).unwrap();
        let desc = parse_description_yml(file(&files, "description.yml")).unwrap();
        let joined = excluded.join(";");
        assert_eq!(desc.excluded_platforms, joined);
        assert!(
            file(&files, DISTRIBUTION).contains(&format!("exclude_archs: '{joined}'\n")),
            "{joined}"
        );
    }
}

/// The default skips the Wasm and `MinGW` builds most cargo extensions exclude.
#[test]
fn default_excludes_wasm_and_mingw() {
    assert_eq!(
        ScaffoldConfig::default().excluded_platforms,
        ["wasm_mvp", "wasm_eh", "wasm_threads", "windows_amd64_mingw"]
    );
    for p in DEFAULT_EXCLUDED_PLATFORMS {
        crate::validate::validate_platform(p).unwrap();
    }
}

/// An unstable-ABI binary loads into exactly one release, so its distribution
/// build uses that release's extension-ci-tools branch.
#[test]
fn unstable_distribution_builds_with_the_target_release() {
    let config = ScaffoldConfig {
        use_unstable_c_api: true,
        target_duckdb_version: "v1.5.5".to_string(),
        ..valid_config()
    };
    let files = generate_scaffold(&config).unwrap();
    let wf = file(&files, DISTRIBUTION);
    assert!(wf.contains("_extension_distribution.yml@v1.5.5\n"), "{wf}");
    assert!(wf.contains("duckdb_version: v1.5.5\n"), "{wf}");
    assert!(file(&files, ".gitmodules").contains("\tbranch = v1.5.5\n"));
}

/// M2: `actions/setup-python` was pinned to v5.4.0, which runs on the
/// deprecated node20.
#[test]
fn workflow_pins_current_action_releases() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let ci = file(&files, NATIVE_CI);
    for pin in [
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
        "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0",
        "Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2",
        "dtolnay/rust-toolchain@e2a55d2ffb04f378e9626c28d38b36d230d1e12f # master 2026-10-08",
    ] {
        assert!(ci.contains(pin), "{pin}");
    }
    assert!(!ci.contains("v5.4.0"), "{ci}");
}

/// Both workflows are YAML a GitHub runner accepts, with the structure the
/// jobs depend on (checked with `PyYAML` when it is installed).
#[test]
#[cfg_attr(miri, ignore = "spawns python3; Miri cannot run a subprocess")]
fn workflows_parse_as_yaml() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let script = "import sys, yaml\n\
                  d = yaml.safe_load(sys.stdin.read())\n\
                  assert isinstance(d['jobs'], dict) and d['jobs'], d\n\
                  assert True in d or 'on' in d, d\n\
                  print(sorted(d['jobs']))\n";
    let files = generate_scaffold(&valid_config()).unwrap();
    for (path, jobs) in [
        (NATIVE_CI, "['build', 'msrv']"),
        (DISTRIBUTION, "['duckdb-stable-build']"),
    ] {
        let Ok(mut child) = Command::new("python3")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        else {
            eprintln!("python3 not found; skipping");
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(file(&files, path).as_bytes()).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("No module named 'yaml'") {
            eprintln!("PyYAML not installed; skipping");
            return;
        }
        assert!(out.status.success(), "{path}: {stderr}");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), jobs, "{path}");
    }
}

/// M4: `concat` skips NULLs, so `<name>_hello(NULL)` returned a greeting.
#[test]
fn example_function_propagates_null() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let lib = file(&files, "src/lib.rs");
    assert!(
        lib.contains("\"'Hello from my_analytics! ' || name\""),
        "{lib}"
    );
    assert!(!lib.contains("concat("), "{lib}");
    let test = file(&files, "test/sql/my_analytics.test");
    assert!(
        test.contains("\nquery T\nSELECT my_analytics_hello(NULL);\n----\nNULL\n"),
        "{test}"
    );
}

/// F3: a license expression with a line break passes the SPDX check (which
/// splits on any whitespace) and was written unquoted, ending the YAML scalar.
#[test]
fn license_with_line_breaks_stays_one_yaml_scalar() {
    for license in [
        "MIT\nOR Apache-2.0",
        "MIT\r\nOR Apache-2.0",
        "MIT\u{85}OR Apache-2.0",
        "MIT\u{2028}OR\tApache-2.0",
    ] {
        let config = ScaffoldConfig {
            license: license.to_string(),
            ..valid_config()
        };
        let Ok(files) = generate_scaffold(&config) else {
            continue; // rejected outright is fine too
        };
        let yml = file(&files, "description.yml");
        assert!(yml.contains("  license: \"MIT OR Apache-2.0\"\n"), "{yml}");
        let desc = parse_description_yml(yml).unwrap();
        assert_eq!(desc.license, "MIT OR Apache-2.0");
    }
}

/// F4: description lines that read as Markdown blocks (`- one` then a lazy
/// continuation, a quote, a fence, an indented line) tripped
/// `clippy::doc_lazy_continuation` or became doctests. Leading block markers
/// are escaped and indentation dropped.
#[test]
fn description_lines_cannot_form_markdown_blocks() {
    let config = ScaffoldConfig {
        description: "Functions:\n- one\nand more\n> quote\nlazy\n```rust\n    indented\n\
                      1. first\n12) second\n# heading\n[x]: https://example.com\n\nplain"
            .to_string(),
        ..valid_config()
    };
    let files = generate_scaffold(&config).unwrap();
    let lib = file(&files, "src/lib.rs");
    let expected = "//! Functions:\n//! \\- one\n//! and more\n//! \\> quote\n//! lazy\n\
                    //! \\```rust\n//! indented\n//! 1\\. first\n//! 12\\) second\n\
                    //! \\# heading\n//! \\[x]: https://example.com\n//!\n//! plain\n";
    assert!(lib.starts_with(expected), "{lib}");
    // description.yml keeps the exact text.
    let desc = parse_description_yml(file(&files, "description.yml")).unwrap();
    assert_eq!(desc.description, config.description);
}

/// The `CMake` config pins the same revision as `repo.ref`, not `main`.
#[test]
fn extension_config_cmake_pins_the_configured_ref() {
    let config = ScaffoldConfig {
        git_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        ..valid_config()
    };
    let files = generate_scaffold(&config).unwrap();
    let cmake = file(&files, "extension_config.cmake");
    assert!(
        cmake.contains("\tGIT_TAG 0123456789abcdef0123456789abcdef01234567\n"),
        "{cmake}"
    );
    assert!(!cmake.contains("GIT_TAG main"), "{cmake}");
}

/// Only a run of digits followed by `.` or `)` is an ordered-list marker: a
/// line starting with `.` or `)` itself, or digits followed by anything else,
/// is plain text and stays unescaped.
#[test]
fn only_digits_then_a_list_delimiter_are_escaped() {
    let config = ScaffoldConfig {
        description:
            "Summary\n.dot first\n) paren first\n2026 was a year\n3.5 times faster\n7) seventh"
                .to_string(),
        ..valid_config()
    };
    let files = generate_scaffold(&config).unwrap();
    let lib = file(&files, "src/lib.rs");
    let expected = "//! Summary\n//! .dot first\n//! ) paren first\n//! 2026 was a year\n\
                    //! 3\\.5 times faster\n//! 7\\) seventh\n";
    assert!(lib.starts_with(expected), "{lib}");
}
