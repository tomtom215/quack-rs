// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Tests of what the scaffold accepts, warns about and refuses, and of the
//! files a community submission needs from it: the example function and its
//! `SQLLogicTest`, `README.md`, `docs/function_descriptions.csv`, the license
//! text and `deny.toml`. Split from `tests.rs` to keep both files within the
//! project's size budget.

use super::*;

fn valid_config() -> ScaffoldConfig {
    ScaffoldConfig {
        name: "my_analytics".to_string(),
        description: "Fast analytics functions".to_string(),
        maintainer: "Jane Doe".to_string(),
        github_repo: "janedoe/duckdb-my-analytics".to_string(),
        ..ScaffoldConfig::default()
    }
}

fn with_license(license: &str) -> ScaffoldConfig {
    ScaffoldConfig {
        license: license.to_string(),
        ..valid_config()
    }
}

fn with_excluded(platforms: &[&str]) -> ScaffoldConfig {
    ScaffoldConfig {
        excluded_platforms: platforms.iter().map(|p| (*p).to_string()).collect(),
        ..valid_config()
    }
}

fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} not generated"))
        .content
}

fn has_file(files: &[GeneratedFile], path: &str) -> bool {
    files.iter().any(|f| f.path == path)
}

// ---------------------------------------------------------------------------
// License (CONFORM-14, HARDEN-5)
// ---------------------------------------------------------------------------

/// The scaffold refused every SPDX identifier outside quack-rs's ~40-entry
/// shortlist, so `BUSL-1.1` or `CC0-1.0` could not be scaffolded at all.
#[test]
fn an_spdx_identifier_outside_the_shortlist_is_accepted() {
    for license in [
        "BUSL-1.1",
        "CC0-1.0",
        "CC-BY-SA-4.0",
        "MIT OR BUSL-1.1",
        "LicenseRef-Acme-1.0",
    ] {
        let files = generate_scaffold(&with_license(license)).expect(license);
        let cargo = file(&files, "Cargo.toml");
        assert!(
            cargo.contains(&format!("license = \"{license}\"")),
            "{license}: {cargo}"
        );
    }
}

#[test]
fn an_unlisted_identifier_is_a_warning_and_a_listed_one_is_not() {
    let scaffold = generate_scaffold_with_warnings(&with_license("BUSL-1.1")).unwrap();
    assert_eq!(scaffold.warnings.len(), 1, "{:?}", scaffold.warnings);
    assert!(
        scaffold.warnings[0].contains("'BUSL-1.1'"),
        "{:?}",
        scaffold.warnings
    );
    for license in ["MIT", "MIT OR Apache-2.0", "Apache-2.0 WITH LLVM-exception"] {
        let scaffold = generate_scaffold_with_warnings(&with_license(license)).unwrap();
        assert!(
            scaffold.warnings.is_empty(),
            "{license}: {:?}",
            scaffold.warnings
        );
    }
}

/// Accepting unlisted identifiers must not let through what is not SPDX at
/// all, or what could not be written into TOML and YAML as it is.
#[test]
fn a_malformed_or_miscased_license_is_refused() {
    for license in [
        "MIT OR",
        "(MIT",
        "MIT Apache-2.0",
        "MIT AND AND Apache-2.0",
        "MIT/Apache-2.0",
        "MIT\"",
        "Ünïcode-1.0",
        "mit",
        "apache-2.0",
    ] {
        assert!(
            generate_scaffold(&with_license(license)).is_err(),
            "{license:?} accepted"
        );
    }
}

/// HARDEN-5: `MIT\nOR Apache-2.0` passed the SPDX check, which splits on any
/// whitespace, and the line break reached `license = "…"` in `Cargo.toml`,
/// which a TOML basic string cannot hold.
#[test]
fn a_license_with_line_breaks_is_written_on_one_line() {
    // The whitespace list is harden's (patch 03): what `tomllib` refused, and
    // what parses but is not a license.
    for license in [
        "MIT\nOR Apache-2.0",
        "MIT\r\nOR\tApache-2.0",
        "MIT  OR Apache-2.0",
        "MIT OR\u{b}Apache-2.0",
        "MIT\u{c}OR Apache-2.0",
        "MIT\u{a0}OR\u{a0}Apache-2.0",
        "MIT\u{2028}OR\u{2028}Apache-2.0",
        "MIT\u{85}OR Apache-2.0",
    ] {
        let files = generate_scaffold(&with_license(license)).expect(license);
        let cargo = file(&files, "Cargo.toml");
        assert!(
            cargo
                .lines()
                .any(|line| line == "license = \"MIT OR Apache-2.0\""),
            "{license:?}: {cargo}"
        );
        assert!(
            file(&files, "description.yml").contains("license: \"MIT OR Apache-2.0\""),
            "{license:?}"
        );
        assert!(file(&files, "README.md").contains("`MIT OR Apache-2.0`"));
    }
}

/// The repository needs the license text; `license = "MIT"` is metadata.
#[test]
fn a_known_license_gets_its_text() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let text = file(&files, "LICENSE");
    assert!(
        text.starts_with("MIT License\n\nCopyright (c) Jane Doe\n"),
        "{text}"
    );
    assert!(!has_file(&files, "LICENSE.TODO"));

    for (license, path, first_line) in [
        ("Apache-2.0", "LICENSE", "Apache License"),
        ("BSD-2-Clause", "LICENSE", "Copyright (c) Jane Doe"),
        ("BSD-3-Clause", "LICENSE", "Copyright (c) Jane Doe"),
        ("ISC", "LICENSE", "ISC License"),
        ("0BSD", "LICENSE", "Copyright (C) Jane Doe"),
        ("MIT-0", "LICENSE", "MIT No Attribution"),
        (
            "Unlicense",
            "LICENSE",
            "This is free and unencumbered software",
        ),
    ] {
        let files = generate_scaffold(&with_license(license)).expect(license);
        let text = file(&files, path);
        assert!(
            text.lines().any(|l| l.trim_start().starts_with(first_line)),
            "{license}: {text}"
        );
        assert!(!text.contains("@@"), "{license}: placeholder left");
        assert!(
            !text.contains("<year>") && !text.contains("<owner>"),
            "{license}"
        );
        assert!(text.ends_with('\n') && !text.ends_with("\n\n"), "{license}");
    }
}

#[test]
fn an_expression_gets_one_text_per_license_and_the_rest_a_todo() {
    let files = generate_scaffold(&with_license("MIT OR Apache-2.0")).unwrap();
    assert!(file(&files, "LICENSE-MIT").starts_with("MIT License"));
    assert!(file(&files, "LICENSE-APACHE").contains("Apache License"));
    assert!(!has_file(&files, "LICENSE") && !has_file(&files, "LICENSE.TODO"));

    let files = generate_scaffold(&with_license("MIT OR BUSL-1.1")).unwrap();
    assert!(has_file(&files, "LICENSE-MIT"));
    let todo = file(&files, "LICENSE.TODO");
    assert!(
        todo.contains("`BUSL-1.1`") && !todo.contains("`MIT`"),
        "{todo}"
    );

    // An exception changes the text, so no file claims to be the license.
    let files = generate_scaffold(&with_license("Apache-2.0 WITH LLVM-exception")).unwrap();
    assert!(!has_file(&files, "LICENSE") && !has_file(&files, "LICENSE-APACHE"));
    assert!(file(&files, "LICENSE.TODO").contains("Apache-2.0 WITH LLVM-exception"));
}

// ---------------------------------------------------------------------------
// Version and platforms
// ---------------------------------------------------------------------------

/// `SemVer` places no bound on a component, Cargo reads each as `u64`: the
/// scaffold produced a project Cargo refused to read.
#[test]
fn a_version_cargo_cannot_read_is_refused() {
    for version in [
        "18446744073709551616.0.0",
        "0.18446744073709551616.0",
        "0.1.18446744073709551616-rc.1",
    ] {
        let config = ScaffoldConfig {
            version: version.to_string(),
            ..valid_config()
        };
        let err = generate_scaffold(&config).expect_err(version);
        assert!(err.as_str().contains("u64::MAX"), "{version}: {err}");
    }
    let config = ScaffoldConfig {
        version: "18446744073709551615.0.0".to_string(),
        ..valid_config()
    };
    assert!(generate_scaffold(&config).is_ok());
}

/// The community build compares exact platform names, so a group or a
/// platform it no longer builds excludes nothing: the scaffold wrote it into
/// `description.yml` and `exclude_archs` as if it worked.
#[test]
fn an_exclusion_that_excludes_nothing_is_refused() {
    let err = generate_scaffold(&with_excluded(&["wasm"])).unwrap_err();
    for member in ["wasm_eh", "wasm_mvp", "wasm_threads"] {
        assert!(err.as_str().contains(member), "{err}");
    }
    for platform in [
        "linux",
        "osx",
        "windows",
        "linux_amd64_gcc4",
        "windows_amd64_rtools",
    ] {
        assert!(
            generate_scaffold(&with_excluded(&[platform])).is_err(),
            "{platform} accepted"
        );
    }
    assert!(generate_scaffold(&with_excluded(&[""])).is_err());
}

#[test]
fn excluding_an_opt_in_platform_or_every_platform_is_a_warning() {
    let scaffold = generate_scaffold_with_warnings(&with_excluded(&["osx_amd64"])).unwrap();
    assert!(
        scaffold.warnings.iter().any(|w| w.contains("opt-in")),
        "{:?}",
        scaffold.warnings
    );

    let all: Vec<&str> = crate::validate::platform::DUCKDB_CI_PLATFORMS
        .iter()
        .copied()
        .filter(|p| !crate::validate::platform::is_opt_in_platform(p))
        .collect();
    let scaffold = generate_scaffold_with_warnings(&with_excluded(&all)).unwrap();
    assert!(
        scaffold
            .warnings
            .iter()
            .any(|w| w.contains("nothing would be built")),
        "{:?}",
        scaffold.warnings
    );

    let scaffold = generate_scaffold_with_warnings(&with_excluded(&[])).unwrap();
    assert!(scaffold.warnings.is_empty(), "{:?}", scaffold.warnings);
}

// ---------------------------------------------------------------------------
// The example function and its SQLLogicTest
// ---------------------------------------------------------------------------

/// The example was a `CREATE MACRO`, which writes into the user's database:
/// `LOAD` then fails on a read-only one.
#[test]
fn the_example_is_a_native_scalar_function() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let lib = file(&files, "src/lib.rs");
    assert!(
        lib.contains("ScalarFunctionBuilder::map1_str(HELLO, |name: &str| greet(name))?"),
        "{lib}"
    );
    assert!(
        lib.contains("const HELLO: &str = \"my_analytics_hello\";"),
        "{lib}"
    );
    assert!(!lib.contains("SqlMacro") && !lib.contains("MACRO"), "{lib}");
    assert!(lib.contains("// SAFETY:"), "{lib}");
}

#[test]
fn the_test_file_carries_the_upstream_header() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let test = file(&files, "test/sql/my_analytics.test");
    assert!(
        test.starts_with(
            "# name: test/sql/my_analytics.test\n\
             # description: test the my_analytics extension\n\
             # group: [my_analytics]\n"
        ),
        "{test}"
    );
}

/// A literal `NULL` argument is constant-folded before the function is
/// called, so only a NULL read from a column tests its NULL handling.
#[test]
fn the_test_file_reads_a_null_from_a_column() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let test = file(&files, "test/sql/my_analytics.test");
    assert!(
        test.contains(
            "query T\n\
             SELECT my_analytics_hello(x) FROM (VALUES ('a'), (NULL), ('b')) t(x) ORDER BY x NULLS LAST;\n\
             ----\n\
             Hello from my_analytics! a\n\
             Hello from my_analytics! b\n\
             NULL\n"
        ),
        "{test}"
    );
}

/// The runner executes a file top to bottom and loads the extension at
/// `require`, so a `statement error` before it shows the function comes from
/// this extension and not from anything already loaded.
#[test]
fn the_test_file_checks_the_function_is_missing_before_require() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let test = file(&files, "test/sql/my_analytics.test");
    let before = test
        .find(
            "statement error\n\
             SELECT my_analytics_hello('world');\n\
             ----\n\
             Catalog Error: Scalar Function with name my_analytics_hello does not exist!\n",
        )
        .expect(test);
    let require = test.find("\nrequire my_analytics\n").expect(test);
    assert!(before < require, "{test}");
}

// ---------------------------------------------------------------------------
// Files for the community submission (CONFORM-15)
// ---------------------------------------------------------------------------

/// The extension's page on duckdb.org takes function descriptions only from
/// `docs/function_descriptions.csv` in the community-extensions repository.
#[test]
fn function_descriptions_list_the_example_function() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let csv = file(&files, "docs/function_descriptions.csv");
    assert_eq!(
        csv,
        "function,function_type,description,comment,example\n\
         my_analytics_hello,scalar,\"Returns 'Hello from my_analytics! ' followed by the \
         argument, or NULL for NULL.\",,SELECT my_analytics_hello('world');\n"
    );
    let readme = file(&files, "README.md");
    assert!(
        readme.contains("extensions/my_analytics/docs/function_descriptions.csv"),
        "{readme}"
    );
}

#[test]
fn the_readme_names_the_extension_and_how_to_build_it() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let readme = file(&files, "README.md");
    assert!(
        readme.starts_with("# my_analytics\n\nFast analytics functions\n"),
        "{readme}"
    );
    for needle in [
        "git submodule add -b v2.0-cyanoptera",
        "make configure",
        "make test",
        "SELECT my_analytics_hello('world');",
        "Do not `strip`",
    ] {
        assert!(readme.contains(needle), "{needle}: {readme}");
    }
    assert!(!readme.contains("repo.variegata"), "{readme}");

    let unstable = ScaffoldConfig {
        use_unstable_c_api: true,
        target_duckdb_version: "v1.5.6".to_string(),
        ..valid_config()
    };
    let files = generate_scaffold(&unstable).unwrap();
    assert!(file(&files, "README.md").contains("repo.variegata"));
}

/// `ref` is built for the newest release only; `variegata` and `andium` are
/// how the community build ships an extension for v1.5.6 and v1.4.5.
#[test]
fn description_yml_documents_the_per_release_refs() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let yml = file(&files, "description.yml");
    for key in [
        "# variegata: <hash>",
        "# andium: <hash>",
        "# ref_next: <hash>",
    ] {
        assert!(yml.contains(key), "{key}: {yml}");
    }
    assert!(
        yml.contains("(which is built for\n  # the newest release, v2.0.0)"),
        "{yml}"
    );

    let unstable = ScaffoldConfig {
        use_unstable_c_api: true,
        target_duckdb_version: "v1.5.6".to_string(),
        ..valid_config()
    };
    let scaffold = generate_scaffold_with_warnings(&unstable).unwrap();
    let yml = file(&scaffold.files, "description.yml");
    assert!(yml.contains("stamped C_STRUCT_UNSTABLE"), "{yml}");
    assert!(
        scaffold
            .warnings
            .iter()
            .any(|w| w.contains("repo.variegata")),
        "{:?}",
        scaffold.warnings
    );
}

/// The book is where the pitfalls live for a reader of a generated project:
/// `LESSONS.md` is not in it.
#[test]
fn generated_files_link_the_book_not_lessons_md() {
    for config in [
        valid_config(),
        ScaffoldConfig {
            target_duckdb_version: "v1.5.6".to_string(),
            ..valid_config()
        },
    ] {
        let files = generate_scaffold(&config).unwrap();
        for f in &files {
            assert!(
                !f.content.contains("LESSONS"),
                "{} mentions LESSONS",
                f.path
            );
        }
        assert!(file(&files, "Makefile").contains(
            "https://quack-rs.com/reference/pitfalls.html#p2-metadata-version-is-c-api-version-not-duckdb-version"
        ));
        assert!(file(&files, "Makefile").contains(
            "https://quack-rs.com/reference/pitfalls.html#p4-extension-ci-tools-submodule-must-be-initialized"
        ));
        assert!(file(&files, "test/sql/my_analytics.test").contains(
            "https://quack-rs.com/reference/pitfalls.html#p5-sqllogictest-expected-values-must-match-exactly"
        ));
    }
}

/// DEPS-1: a new project locks `zip` 6 through `libduckdb-sys` 1.10506.0, and
/// `cargo deny check` failed on three advisories that cannot reach a loadable
/// extension. The generated `deny.toml` ignores exactly those, with reasons,
/// and allows the project's own license. (`cargo deny check` on a generated,
/// locked project passes with it and fails on the three without the ignores:
/// see the commit message.)
#[test]
fn deny_toml_ignores_the_zip_advisories_and_allows_the_project_license() {
    let files =
        generate_scaffold(&with_license("BUSL-1.1 OR Apache-2.0 WITH LLVM-exception")).unwrap();
    let deny = file(&files, "deny.toml");
    for id in [
        "RUSTSEC-2026-0367",
        "RUSTSEC-2026-0368",
        "RUSTSEC-2026-0369",
    ] {
        assert!(
            deny.lines()
                .any(|l| l.contains(&format!("{{ id = \"{id}\", reason = \"zip < 9"))),
            "{id}: {deny}"
        );
    }
    assert_eq!(deny.matches("RUSTSEC-").count(), 3, "{deny}");
    assert!(deny.contains("\n    \"BUSL-1.1\",\n"), "{deny}");
    assert_eq!(
        deny.matches("\"Apache-2.0 WITH LLVM-exception\"").count(),
        1,
        "{deny}"
    );
    assert!(!deny.contains("@@"), "{deny}");
    assert!(deny.contains("allow = [\n    \"0BSD\",\n"), "{deny}");
}
