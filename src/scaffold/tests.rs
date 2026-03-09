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
    assert!(cargo.content.contains("panic = \"abort\""));
    assert!(cargo.content.contains("lto = true"));
    assert!(cargo.content.contains("strip = true"));
}

#[test]
fn makefile_has_extension_name() {
    let files = generate_scaffold(&valid_config()).unwrap();
    let makefile = files.iter().find(|f| f.path == "Makefile").unwrap();
    assert!(makefile.content.contains("EXT_NAME=my_analytics"));
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
    config.version = "not-a-version".to_string();
    assert!(generate_scaffold(&config).is_err());
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
