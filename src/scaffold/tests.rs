// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;

fn valid_config() -> ScaffoldConfig {
    ScaffoldConfig {
        name: "my_analytics".to_string(),
        description: "Fast analytics functions".to_string(),
        version: "0.1.0".to_string(),
        license: "MIT".to_string(),
        maintainer: "Jane Doe".to_string(),
        github_repo: "janedoe/duckdb-my-analytics".to_string(),
        excluded_platforms: vec![],
        ..ScaffoldConfig::default()
    }
}

#[test]
fn generates_all_required_files() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"Cargo.toml"));
    assert!(paths.contains(&"Makefile"));
    assert!(paths.contains(&"extension_config.cmake"));
    assert!(paths.contains(&"src/lib.rs"));
    assert!(paths.contains(&"src/wasm_lib.rs"));
    assert!(paths.contains(&"description.yml"));
    assert!(paths.contains(&"test/sql/my_analytics.test"));
    assert!(paths.contains(&".github/workflows/extension-ci.yml"));
    assert!(paths.contains(&".gitmodules"));
    assert!(paths.contains(&".gitignore"));
    assert!(paths.contains(&".cargo/config.toml"));
}

#[test]
fn cargo_toml_has_correct_name() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
    assert!(cargo.content.contains("name = \"my_analytics\""));
}

#[test]
fn cargo_toml_has_cdylib() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
    assert!(cargo.content.contains("crate-type = [\"cdylib\"]"));
}

#[test]
fn cargo_toml_has_release_profile() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
    // `unwind`, not `abort` — otherwise quack-rs's `catch_unwind`-based panic
    // handling in callbacks and the entry point is inert in release builds.
    assert!(cargo.content.contains("panic = \"unwind\""));
    assert!(!cargo.content.contains("panic = \"abort\""));
    assert!(cargo.content.contains("lto = true"));
    assert!(cargo.content.contains("strip = true"));
}

#[test]
fn makefile_has_extension_name() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = files.iter().find(|f| f.path == "Makefile").unwrap();
    // base.Makefile reads EXTENSION_NAME, not EXT_NAME.
    assert!(makefile.content.contains("EXTENSION_NAME=my_analytics"));
}

#[test]
fn makefile_includes_rust_build_rules() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = files.iter().find(|f| f.path == "Makefile").unwrap();
    assert!(makefile.content.contains("rust.Makefile"));
}

#[test]
fn lib_rs_has_entry_point() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let lib = files.iter().find(|f| f.path == "src/lib.rs").unwrap();
    assert!(lib.content.contains("entry_point!"));
}

#[test]
fn lib_rs_uses_quack_rs_api() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let lib = files.iter().find(|f| f.path == "src/lib.rs").unwrap();
    assert!(lib.content.contains("quack_rs::prelude"));
    // Must not use the duckdb crate VTab API
    assert!(!lib.content.contains("use duckdb::"));
    // Must not contain .expect() or .unwrap() (no panics in FFI paths)
    assert!(!lib.content.contains(".expect("));
    assert!(!lib.content.contains(".unwrap()"));
}

#[test]
fn lib_rs_no_cpp_glue() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let lib = files.iter().find(|f| f.path == "src/lib.rs").unwrap();
    // Must not contain any C++ references
    assert!(!lib.content.contains("CMake"));
}

#[test]
fn description_yml_has_fields() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let desc = files.iter().find(|f| f.path == "description.yml").unwrap();
    assert!(desc.content.contains("name: my_analytics"));
    assert!(desc.content.contains("license: MIT"));
    assert!(desc.content.contains("janedoe/duckdb-my-analytics"));
}

#[test]
fn description_yml_uses_rust_language() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let desc = files.iter().find(|f| f.path == "description.yml").unwrap();
    assert!(desc.content.contains("language: Rust"));
    assert!(desc.content.contains("build: cargo"));
    assert!(desc.content.contains("requires_toolchains: rust;python3"));
}

#[test]
fn invalid_name_rejected() {
    let mut config = valid_config();
    config.name = "Invalid Name".to_string();
    assert!(generate_scaffold(&config).is_err());
}

#[test]
fn invalid_license_rejected() {
    let mut config = valid_config();
    config.license = "FAKE-LICENSE".to_string();
    assert!(generate_scaffold(&config).is_err());
}

#[test]
fn invalid_version_rejected() {
    let mut config = valid_config();
    // Whitespace and path separators are rejected; an unusual-but-harmless
    // version like "not-a-version" or "2025120401" is not, because DuckDB's
    // community-extension docs specify no version format and real published
    // extensions use date-based build ids.
    config.version = "1.0.0 beta".to_string();
    assert!(generate_scaffold(&config).is_err());
    config.version = "../etc/passwd".to_string();
    assert!(generate_scaffold(&config).is_err());
    config.version = "2025120401".to_string();
    assert!(
        generate_scaffold(&config).is_ok(),
        "a date-based build id is used by 11 of 43 published extensions"
    );
}

#[test]
fn invalid_platform_rejected() {
    let mut config = valid_config();
    config.excluded_platforms = vec!["invalid_platform".to_string()];
    assert!(generate_scaffold(&config).is_err());
}

#[test]
fn excluded_platforms_in_description() {
    let mut config = valid_config();
    config.excluded_platforms = vec!["wasm_mvp".to_string(), "wasm_eh".to_string()];
    let files = generate_scaffold(&config).unwrap();
    let desc = files.iter().find(|f| f.path == "description.yml").unwrap();
    // Platforms are semicolon-separated per DuckDB convention
    assert!(desc
        .content
        .contains("excluded_platforms: \"wasm_mvp;wasm_eh\""));
}

#[test]
fn gitignore_has_target_and_duckdb_patterns() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let gi = files.iter().find(|f| f.path == ".gitignore").unwrap();
    assert!(gi.content.contains("/target"));
    assert!(gi.content.contains("*.duckdb"));
    assert!(gi.content.contains("build/"));
}

#[test]
fn gitmodules_references_ci_tools() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let gitmod = files.iter().find(|f| f.path == ".gitmodules").unwrap();
    assert!(gitmod
        .content
        .contains("https://github.com/duckdb/extension-ci-tools"));
}

#[test]
fn unstable_version_accepted() {
    let mut config = valid_config();
    config.version = "690bfc5".to_string();
    assert!(generate_scaffold(&config).is_ok());
}

#[test]
fn wasm_staticlib_example_present() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
    assert!(cargo.content.contains("staticlib"));
    assert!(cargo.content.contains("wasm_lib.rs"));
}

#[test]
fn cargo_config_has_windows_crt_static() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cfg = files
        .iter()
        .find(|f| f.path == ".cargo/config.toml")
        .unwrap();
    assert!(cfg.content.contains("crt-static"));
    assert!(cfg.content.contains("x86_64-pc-windows-msvc"));
    assert!(cfg.content.contains("aarch64-pc-windows-msvc"));
}

#[test]
fn wasm_lib_shim_exists() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let wasm = files.iter().find(|f| f.path == "src/wasm_lib.rs").unwrap();
    // A bare `mod lib;` in `src/wasm_lib.rs` resolves to `src/wasm_lib/lib.rs`,
    // which does not exist — the path attribute is required.
    assert!(wasm.content.contains("#[path = \"lib.rs\"]"));
    assert!(wasm.content.contains("mod lib"));
}

// --- extension_config.cmake ---

#[test]
fn extension_config_cmake_exists() {
    let files = generate_scaffold(&valid_config()).unwrap();
    assert!(files.iter().any(|f| f.path == "extension_config.cmake"));
}

#[test]
fn extension_config_cmake_references_name() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cmake = files
        .iter()
        .find(|f| f.path == "extension_config.cmake")
        .unwrap();
    assert!(cmake.content.contains("duckdb_extension_load(my_analytics"));
}

#[test]
fn extension_config_cmake_references_github_repo() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cmake = files
        .iter()
        .find(|f| f.path == "extension_config.cmake")
        .unwrap();
    assert!(cmake.content.contains("janedoe/duckdb-my-analytics"));
}

#[test]
fn extension_config_cmake_has_load_tests() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cmake = files
        .iter()
        .find(|f| f.path == "extension_config.cmake")
        .unwrap();
    assert!(cmake.content.contains("LOAD_TESTS"));
}

// --- SQLLogicTest ---

#[test]
fn sqllogictest_file_exists() {
    let files = generate_scaffold(&valid_config()).unwrap();
    assert!(files.iter().any(|f| f.path == "test/sql/my_analytics.test"));
}

#[test]
fn sqllogictest_has_require_directive() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let test = files
        .iter()
        .find(|f| f.path == "test/sql/my_analytics.test")
        .unwrap();
    assert!(test.content.contains("require my_analytics"));
}

#[test]
fn sqllogictest_name_matches_extension() {
    let mut config = valid_config();
    config.name = "custom_ext".to_string();
    let files = generate_scaffold(&config).unwrap();
    let test_path = "test/sql/custom_ext.test";
    assert!(files.iter().any(|f| f.path == test_path));
    let test = files.iter().find(|f| f.path == test_path).unwrap();
    assert!(test.content.contains("require custom_ext"));
}

// --- GitHub Actions CI ---

#[test]
fn extension_ci_yml_exists() {
    let files = generate_scaffold(&valid_config()).unwrap();
    assert!(files
        .iter()
        .any(|f| f.path == ".github/workflows/extension-ci.yml"));
}

#[test]
fn extension_ci_yml_has_linux_matrix() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let ci = files
        .iter()
        .find(|f| f.path == ".github/workflows/extension-ci.yml")
        .unwrap();
    assert!(ci.content.contains("ubuntu-latest"));
    assert!(ci.content.contains("linux_amd64"));
}

#[test]
fn extension_ci_yml_has_macos_matrix() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let ci = files
        .iter()
        .find(|f| f.path == ".github/workflows/extension-ci.yml")
        .unwrap();
    assert!(ci.content.contains("macos-latest"));
    assert!(ci.content.contains("osx_arm64"));
}

#[test]
fn extension_ci_yml_has_windows_matrix() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let ci = files
        .iter()
        .find(|f| f.path == ".github/workflows/extension-ci.yml")
        .unwrap();
    assert!(ci.content.contains("windows-latest"));
    assert!(ci.content.contains("windows_amd64"));
}

#[test]
fn extension_ci_yml_runs_sqllogictest() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let ci = files
        .iter()
        .find(|f| f.path == ".github/workflows/extension-ci.yml")
        .unwrap();
    assert!(ci.content.contains("make test"));
}

#[test]
fn extension_ci_yml_checks_out_submodules() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let ci = files
        .iter()
        .find(|f| f.path == ".github/workflows/extension-ci.yml")
        .unwrap();
    assert!(ci.content.contains("submodules: recursive"));
}

// ── Build-system correctness ────────────────────────────────────────────────
//
// The generated Makefile drives `extension-ci-tools`. Every variable below is
// one `base.Makefile` actually reads; a typo here yields a binary that DuckDB
// silently refuses to load.

#[test]
fn makefile_uses_variables_extension_ci_tools_reads() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = &files.iter().find(|f| f.path == "Makefile").unwrap().content;

    assert!(makefile.contains("EXTENSION_NAME=my_analytics"));
    assert!(makefile.contains("TARGET_DUCKDB_VERSION=v1.2.0"));
    assert!(makefile.contains("USE_UNSTABLE_C_API=0"));
    // DUCKDB_PLATFORM_VERSION is not a variable extension-ci-tools knows about;
    // setting it leaves TARGET_DUCKDB_VERSION at its v0.0.1 default.
    assert!(!makefile.contains("DUCKDB_PLATFORM_VERSION"));
}

#[test]
fn makefile_defines_the_targets_users_invoke() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = &files.iter().find(|f| f.path == "Makefile").unwrap().content;
    for target in [
        "all:",
        "configure:",
        "debug:",
        "release:",
        "test:",
        "clean:",
    ] {
        assert!(makefile.contains(target), "Makefile is missing `{target}`");
    }
}

#[test]
fn unstable_config_pins_the_duckdb_release() {
    let config = ScaffoldConfig {
        use_unstable_c_api: true,
        target_duckdb_version: "v1.5.5".to_string(),
        ..valid_config()
    };
    let files = generate_scaffold(&config).unwrap();
    let makefile = &files.iter().find(|f| f.path == "Makefile").unwrap().content;
    assert!(makefile.contains("USE_UNSTABLE_C_API=1"));
    assert!(makefile.contains("TARGET_DUCKDB_VERSION=v1.5.5"));
    assert!(makefile.contains("C_STRUCT_UNSTABLE"));
    // Declares the build target to quack-rs's ABI check, so the extension keeps
    // loading after a DuckDB release that quack-rs's layout table predates.
    assert!(makefile.contains("export QUACK_RS_TARGET_DUCKDB_VERSION = $(TARGET_DUCKDB_VERSION)"));
}

#[test]
fn stable_abi_makefile_does_not_declare_a_build_target() {
    // With USE_UNSTABLE_C_API=0, TARGET_DUCKDB_VERSION is the *C API* version,
    // not a DuckDB release — declaring it would be a lie about the bindings.
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = &files.iter().find(|f| f.path == "Makefile").unwrap().content;
    assert!(!makefile.contains("QUACK_RS_TARGET_DUCKDB_VERSION"));
}

#[test]
fn stable_abi_rejects_a_duckdb_release_as_target_version() {
    // v1.5.5 as a C_STRUCT `-dv` would claim a C API version DuckDB cannot
    // supply, so the extension would never load.
    let config = ScaffoldConfig {
        use_unstable_c_api: false,
        target_duckdb_version: "v1.5.5".to_string(),
        ..valid_config()
    };
    let err = generate_scaffold(&config).unwrap_err();
    assert!(err.as_str().contains("C_STRUCT"), "{err}");
}

#[test]
fn unstable_abi_rejects_the_c_api_version_as_target_version() {
    // C_STRUCT_UNSTABLE compares `-dv` against the engine's release version, so
    // "v1.2.0" would only ever load on DuckDB v1.2.0.
    let config = ScaffoldConfig {
        use_unstable_c_api: true,
        target_duckdb_version: crate::DUCKDB_API_VERSION.to_string(),
        ..valid_config()
    };
    let err = generate_scaffold(&config).unwrap_err();
    assert!(err.as_str().contains("C_STRUCT_UNSTABLE"), "{err}");
}

#[test]
fn malformed_target_duckdb_versions_are_rejected() {
    for bad in ["1.5.5", "v1.5", "v1.5.5.1", "vX.Y.Z", "", "v1.5.x"] {
        let config = ScaffoldConfig {
            use_unstable_c_api: true,
            target_duckdb_version: bad.to_string(),
            ..valid_config()
        };
        assert!(
            generate_scaffold(&config).is_err(),
            "target_duckdb_version {bad:?} must be rejected"
        );
    }
}

#[test]
fn cargo_toml_tracks_the_current_quack_rs_version() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let cargo = &files
        .iter()
        .find(|f| f.path == "Cargo.toml")
        .unwrap()
        .content;
    let expected = {
        let mut parts = env!("CARGO_PKG_VERSION").split('.');
        format!("{}.{}", parts.next().unwrap(), parts.next().unwrap_or("0"))
    };
    assert!(
        cargo.contains(&format!("quack-rs = {{ version = \"{expected}\" }}")),
        "generated Cargo.toml must depend on quack-rs {expected}, got:\n{cargo}"
    );
}

#[test]
fn ci_workflow_builds_before_testing_and_uses_real_actions() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let ci = &files
        .iter()
        .find(|f| f.path == ".github/workflows/extension-ci.yml")
        .unwrap()
        .content;

    // `make test` depends on `check_configure`, which only passes after
    // `make configure`, and on a built extension.
    let configure = ci
        .find("make configure")
        .expect("workflow must run make configure");
    let release = ci
        .find("make release")
        .expect("workflow must run make release");
    let test = ci.find("make test").expect("workflow must run make test");
    assert!(configure < release && release < test);

    // duckdb/duckdb-build does not exist; the extension is built from source here.
    assert!(!ci.contains("duckdb/duckdb-build"));
    assert!(ci.contains("submodules: recursive"));
}

/// The generated `description.yml` must not pin a branch: `DuckDB`'s
/// documentation says `ref` is "the hash of the latest commit", and 41 of 43
/// published extensions pin a full hash (the other two pin a tag; none uses a
/// branch).
#[test]
fn generated_description_does_not_pin_a_branch() {
    let files = generate_scaffold(&valid_config()).expect("scaffold");
    let yml = &files
        .iter()
        .find(|f| f.path == "description.yml")
        .expect("description.yml")
        .content;
    assert!(
        !yml.contains("ref: main"),
        "a branch ref makes the community build unreproducible:\n{yml}"
    );
    assert!(yml.contains(&format!("ref: {}", crate::scaffold::REF_PLACEHOLDER)));
    assert!(yml.contains("Must be a commit hash"));
    // Every published extension has a docs: section; it is what renders on the
    // community-extensions documentation site.
    assert!(yml.contains("docs:"), "{yml}");
    assert!(yml.contains("hello_world:"), "{yml}");

    // And the whole thing must still parse.
    let desc = crate::validate::description_yml::parse_description_yml(yml)
        .expect("the scaffold must generate a parseable description.yml");
    assert_eq!(desc.git_ref, crate::scaffold::REF_PLACEHOLDER);
    assert!(desc.git_ref_next.is_empty());
}
