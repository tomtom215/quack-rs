// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Tests pinning what the generated files must contain for the project to
//! build, load and stay ABI-safe. Split from `tests.rs` to keep both files
//! within the project's size budget.

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

fn unstable_config(target: &str) -> ScaffoldConfig {
    ScaffoldConfig {
        use_unstable_c_api: true,
        target_duckdb_version: target.to_string(),
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

/// A hyphenated name used to generate `entry_point!(my-ext_init_c_api, …)` and
/// `[lib] name = "my-ext"` — a project that does not compile, and a symbol
/// `DuckDB` (which derives it from the file name) could never find anyway.
#[test]
fn hyphenated_extension_name_is_rejected() {
    let config = ScaffoldConfig {
        name: "my-ext".to_string(),
        ..valid_config()
    };
    let err = generate_scaffold(&config).unwrap_err();
    assert!(err.as_str().contains("'-'"), "{err}");
}

/// `docs.hello_world` is rendered on the community-extensions site as the
/// example to copy; it used to call `<name>_version()`, which the scaffold
/// never registers.
#[test]
fn description_hello_world_calls_a_function_the_scaffold_registers() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let yml = file(&files, "description.yml");
    let lib = file(&files, "src/lib.rs");
    assert!(yml.contains("SELECT my_analytics_hello('world');"), "{yml}");
    assert!(lib.contains("\"my_analytics_hello\""), "{lib}");
    assert!(!yml.contains("_version()"), "{yml}");
}

/// A `C_STRUCT_UNSTABLE` build must compile against exactly the release it
/// declares. With `libduckdb-sys` left at `>=1.4.4, <2`, bumping
/// `TARGET_DUCKDB_VERSION` without touching the lock file made the extension
/// declare (via `QUACK_RS_TARGET_DUCKDB_VERSION`) a release whose headers it
/// was not compiled against — and when quack-rs has no layout entry for that
/// release, `abi::decide` trusts the declaration and reports `Compatible`.
#[test]
fn unstable_scaffold_pins_libduckdb_sys_to_the_target_release() {
    let cases = [
        ("v1.5.5", "~1.10505.0"),
        ("v1.5.0", "~1.10500.0"),
        ("v1.5.12", "~1.10512.0"),
        ("v1.4.4", "=1.4.4"),
    ];
    for (target, requirement) in cases {
        let files = generate_scaffold(&unstable_config(target)).unwrap();
        let cargo = file(&files, "Cargo.toml");
        let expected = format!(
            "libduckdb-sys = {{ version = \"{requirement}\", features = [\"loadable-extension\"] }}"
        );
        assert!(cargo.contains(&expected), "{target}: {cargo}");
        assert!(!cargo.contains(">=1.4.4, <2"), "{target}: {cargo}");
    }
}

#[test]
fn stable_scaffold_leaves_libduckdb_sys_open() {
    // C_STRUCT uses only the frozen stable prefix; any 1.x binding works.
    let files = generate_scaffold(&valid_config()).unwrap();
    assert!(file(&files, "Cargo.toml").contains(
        "libduckdb-sys = { version = \">=1.4.4, <2\", features = [\"loadable-extension\"] }"
    ));
    let makefile = file(&files, "Makefile");
    assert!(!makefile.contains("check_duckdb_pin"), "{makefile}");
    assert!(!makefile.contains("DUCKDB_TEST_VERSION"), "{makefile}");
}

#[test]
fn unstable_makefile_tests_against_and_verifies_the_target_release() {
    let files = generate_scaffold(&unstable_config("v1.5.5")).unwrap();
    let makefile = file(&files, "Makefile");
    // extension-ci-tools pip-installs `duckdb==$(DUCKDB_TEST_VERSION)`, and
    // defaults to the latest release when it is unset — which a binary pinned
    // to one release cannot load.
    assert!(
        makefile.contains("DUCKDB_TEST_VERSION=$(patsubst v%,%,$(TARGET_DUCKDB_VERSION))"),
        "{makefile}"
    );
    // The build refuses to run when the resolved bindings disagree with the
    // declared release.
    assert!(makefile.contains("check_duckdb_pin:"), "{makefile}");
    assert!(
        makefile.contains("release: check_duckdb_pin build_extension_library_release"),
        "{makefile}"
    );
    assert!(
        makefile.contains("debug: check_duckdb_pin build_extension_library_debug"),
        "{makefile}"
    );
    assert!(
        makefile.contains("cargo tree -e normal -i libduckdb-sys --depth 0 --prefix none"),
        "{makefile}"
    );
}

#[test]
fn unstable_scaffold_rejects_a_release_libduckdb_sys_cannot_encode() {
    // libduckdb-sys encodes DuckDB 1.Y.Z as crate version 1.(10000 + 100·Y + Z);
    // there is no published scheme for another major, nor for a minor or
    // patch of 100 or more, so no pin can be derived.
    for target in ["v2.0.0", "v1.100.0", "v1.5.100", "v0.10.3"] {
        let err = generate_scaffold(&unstable_config(target)).unwrap_err();
        assert!(err.as_str().contains("libduckdb-sys"), "{target}: {err}");
    }
}

/// Regression: the generated project's `cargo test` ran zero tests, and its
/// SQLLogicTest file held nothing but `require` and commented-out examples, so
/// both CI steps passed whatever the extension did.
#[test]
fn generated_lib_rs_carries_a_real_unit_test() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let lib = file(&files, "src/lib.rs");
    assert!(lib.contains("#[cfg(test)]"), "{lib}");
    assert!(lib.contains("#[test]"), "{lib}");
    // The test exercises the function `register` actually registers.
    assert!(lib.contains("hello_macro()"), "{lib}");
    assert!(
        lib.contains(
            r#"CREATE OR REPLACE MACRO "my_analytics_hello"("name") AS (concat('Hello from my_analytics! ', name))"#
        ),
        "{lib}"
    );
}

#[test]
fn generated_sqllogictest_queries_the_hello_function_with_its_expected_output() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let test = file(&files, "test/sql/my_analytics.test");
    assert!(
        test.contains(
            "\nquery T\nSELECT my_analytics_hello('world');\n----\nHello from my_analytics! world\n"
        ),
        "{test}"
    );
    // Every uncommented statement is something the scaffold registers.
    for line in test.lines().filter(|l| l.starts_with("SELECT")) {
        assert!(line.contains("my_analytics_hello("), "{line}");
    }
}

/// Regression (P4): a freshly generated project has a `.gitmodules` but no
/// submodule gitlink, so `git submodule update --init` is a silent no-op and
/// `make` then failed on a bare "No such file or directory" for the include.
/// The Makefile must name the command that actually fixes it.
#[test]
fn makefile_explains_a_missing_extension_ci_tools_checkout() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = file(&files, "Makefile");
    let check = makefile
        .find("ifeq ($(wildcard extension-ci-tools/makefiles/c_api_extensions/base.Makefile),)")
        .expect("a check for the submodule");
    let include = makefile
        .find("include extension-ci-tools/")
        .expect("the include");
    assert!(check < include, "{makefile}");
    assert!(
        makefile.contains(
            "git submodule add https://github.com/duckdb/extension-ci-tools.git extension-ci-tools"
        ),
        "{makefile}"
    );
}
