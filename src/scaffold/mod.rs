//! Project scaffolding for `DuckDB` Rust extensions.
//!
//! Generates the complete file set needed to build and submit a `DuckDB` extension
//! to the community extensions repository — **without any C++ glue**.
//!
//! # Background
//!
//! The `DuckDB` C Extension API (used by the official
//! [`extension-template-rs`](https://github.com/duckdb/extension-template-rs))
//! allows pure-Rust extensions that need only:
//!
//! - `Cargo.toml` (cdylib, pinned `duckdb` + `libduckdb-sys` deps)
//! - `Makefile` (delegates to `cargo build` + metadata scripts from `extension-ci-tools`)
//! - `src/lib.rs` (entry point + function registration)
//! - `extension-ci-tools/` (git submodule for CI/CD)
//! - `description.yml` (extension metadata for community submission)
//!
//! This module generates all of these from a [`ScaffoldConfig`].
//!
//! # Example
//!
//! ```rust
//! use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};
//!
//! let config = ScaffoldConfig {
//!     name: "my_analytics".to_string(),
//!     description: "Fast analytics functions for DuckDB".to_string(),
//!     version: "0.1.0".to_string(),
//!     license: "MIT".to_string(),
//!     maintainer: "Jane Doe".to_string(),
//!     github_repo: "janedoe/duckdb-my-analytics".to_string(),
//!     excluded_platforms: vec![],
//! };
//!
//! let files = generate_scaffold(&config).unwrap();
//! assert!(files.iter().any(|f| f.path == "Cargo.toml"));
//! assert!(files.iter().any(|f| f.path == "Makefile"));
//! assert!(files.iter().any(|f| f.path == "src/lib.rs"));
//! assert!(files.iter().any(|f| f.path == "description.yml"));
//! ```

use crate::error::ExtensionError;
use crate::validate::{validate_extension_name, validate_spdx_license};

/// Configuration for generating a new extension project.
#[derive(Debug, Clone)]
pub struct ScaffoldConfig {
    /// Extension name (must pass [`validate_extension_name`]).
    pub name: String,
    /// One-line description of the extension.
    pub description: String,
    /// Initial version (semver, e.g., `"0.1.0"`).
    pub version: String,
    /// SPDX license identifier (must pass [`validate_spdx_license`]).
    pub license: String,
    /// Primary maintainer name.
    pub maintainer: String,
    /// GitHub repository path (e.g., `"myorg/duckdb-my-ext"`).
    pub github_repo: String,
    /// Platforms to exclude from CI builds (e.g., `["wasm_mvp", "wasm_eh"]`).
    pub excluded_platforms: Vec<String>,
}

/// A generated file with its relative path and content.
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    /// Relative path from the project root (e.g., `"src/lib.rs"`).
    pub path: String,
    /// File content as a string.
    pub content: String,
}

/// Generates the complete set of project files for a new `DuckDB` Rust extension.
///
/// Validates the configuration and returns a list of [`GeneratedFile`]s that can be
/// written to disk. Does NOT write files — callers decide how to persist them.
///
/// # Errors
///
/// Returns [`ExtensionError`] if the extension name, license, or version is invalid.
pub fn generate_scaffold(config: &ScaffoldConfig) -> Result<Vec<GeneratedFile>, ExtensionError> {
    validate_extension_name(&config.name)?;
    crate::validate::validate_extension_version(&config.version)?;
    validate_spdx_license(&config.license)?;

    for platform in &config.excluded_platforms {
        crate::validate::validate_platform(platform)?;
    }

    let files = vec![
        GeneratedFile {
            path: "Cargo.toml".to_string(),
            content: generate_cargo_toml(config),
        },
        GeneratedFile {
            path: "Makefile".to_string(),
            content: generate_makefile(config),
        },
        GeneratedFile {
            path: "src/lib.rs".to_string(),
            content: generate_lib_rs(config),
        },
        GeneratedFile {
            path: "description.yml".to_string(),
            content: generate_description_yml(config),
        },
        GeneratedFile {
            path: ".gitmodules".to_string(),
            content: generate_gitmodules(),
        },
        GeneratedFile {
            path: ".gitignore".to_string(),
            content: generate_gitignore(),
        },
    ];

    Ok(files)
}

fn generate_cargo_toml(config: &ScaffoldConfig) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "2021"

[lib]
name = "{name}"
crate-type = ["cdylib"]

# WASM support: staticlib target for emcc compilation.
# See extension-ci-tools for details.
[[example]]
name = "{name}"
crate-type = ["staticlib"]
path = "src/lib.rs"

[dependencies]
quack-rs = {{ version = "0.1.0" }}
duckdb = {{ version = "=1.4.4", features = ["loadable-extension"] }}
libduckdb-sys = {{ version = "=1.4.4", features = ["loadable-extension"] }}

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
"#,
        name = config.name,
        version = config.version,
    )
}

fn generate_makefile(config: &ScaffoldConfig) -> String {
    // Matches the structure from duckdb/extension-template-rs
    format!(
        r"# DuckDB Rust Extension Makefile
# Delegates to cargo for building and extension-ci-tools for metadata.

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

# Extension configuration
EXT_NAME={name}
EXT_CONFIG=$(PROJ_DIR)extension_config.cmake

# DuckDB C API version (NOT the DuckDB release version)
# See: https://github.com/tomtom215/quack-rs/blob/main/LESSONS.md (Pitfall P8)
USE_UNSTABLE_C_API=1
DUCKDB_PLATFORM_VERSION=v1.4.4

# Include extension-ci-tools build rules
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile
",
        name = config.name,
    )
}

fn generate_lib_rs(config: &ScaffoldConfig) -> String {
    format!(
        r#"//! {description}
//!
//! A DuckDB extension built with [quack-rs](https://github.com/tomtom215/quack-rs).

use duckdb::{{
    core::{{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId}},
    duckdb_entrypoint_c_api,
    vtab::{{BindInfo, InitInfo, TableFunctionInfo, VTab}},
    Connection, Result,
}};
use std::{{
    error::Error,
    ffi::CString,
    sync::atomic::{{AtomicBool, Ordering}},
}};

/// Extension name — must match `[lib] name` in Cargo.toml and `description.yml`.
const EXTENSION_NAME: &str = env!("CARGO_PKG_NAME");

// ---------------------------------------------------------------------------
// Example: a simple table function. Replace with your own functions.
// ---------------------------------------------------------------------------

#[repr(C)]
struct HelloBindData {{
    name: String,
}}

#[repr(C)]
struct HelloInitData {{
    done: AtomicBool,
}}

struct HelloVTab;

impl VTab for HelloVTab {{
    type InitData = HelloInitData;
    type BindData = HelloBindData;

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn Error>> {{
        bind.add_result_column("column0", LogicalTypeHandle::from(LogicalTypeId::Varchar));
        let name = bind.get_parameter(0).to_string();
        Ok(HelloBindData {{ name }})
    }}

    fn init(_: &InitInfo) -> Result<Self::InitData, Box<dyn Error>> {{
        Ok(HelloInitData {{
            done: AtomicBool::new(false),
        }})
    }}

    fn func(
        func: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn Error>> {{
        let init_data = func.get_init_data();
        let bind_data = func.get_bind_data();
        if init_data.done.swap(true, Ordering::Relaxed) {{
            output.set_len(0);
        }} else {{
            let vector = output.flat_vector(0);
            let result = CString::new(format!("Hello from {name}! {{}}", bind_data.name))?;
            vector.insert(0, result);
            output.set_len(1);
        }}
        Ok(())
    }}

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {{
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }}
}}

// ---------------------------------------------------------------------------
// Entry point — the C Extension API handles everything, no C++ glue needed.
// ---------------------------------------------------------------------------

#[duckdb_entrypoint_c_api()]
pub unsafe fn extension_entrypoint(con: Connection) -> Result<(), Box<dyn Error>> {{
    con.register_table_function::<HelloVTab>(EXTENSION_NAME)
        .expect("Failed to register table function");
    Ok(())
}}
"#,
        description = config.description,
        name = config.name,
    )
}

fn generate_description_yml(config: &ScaffoldConfig) -> String {
    let mut yml = format!(
        r"extension:
  name: {name}
  description: {description}
  version: {version}
  language: C
  build: cargo
  license: {license}
  maintainers:
    - {maintainer}

repo:
  github: {github_repo}
  ref: main
",
        name = config.name,
        description = config.description,
        version = config.version,
        license = config.license,
        maintainer = config.maintainer,
        github_repo = config.github_repo,
    );

    if !config.excluded_platforms.is_empty() {
        use std::fmt::Write;
        yml.push_str("\nextension:\n  excluded_platforms:\n");
        for platform in &config.excluded_platforms {
            let _ = writeln!(yml, "    - {platform}");
        }
    }

    yml
}

fn generate_gitmodules() -> String {
    "[submodule \"extension-ci-tools\"]\n\tpath = extension-ci-tools\n\turl = https://github.com/duckdb/extension-ci-tools\n".to_string()
}

fn generate_gitignore() -> String {
    "/target\n*.duckdb\n*.wal\nbuild/\n.env\n__pycache__/\n".to_string()
}

#[cfg(test)]
mod tests {
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
        assert!(paths.contains(&"src/lib.rs"));
        assert!(paths.contains(&"description.yml"));
        assert!(paths.contains(&".gitmodules"));
        assert!(paths.contains(&".gitignore"));
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
        assert!(lib.content.contains("duckdb_entrypoint_c_api"));
    }

    #[test]
    fn lib_rs_no_cpp_glue() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let lib = files.iter().find(|f| f.path == "src/lib.rs").unwrap();
        // Must not contain any C++ references
        assert!(!lib.content.contains("cpp"));
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
    fn description_yml_uses_c_language() {
        // C Extension API extensions use language: C, not C++
        let files = generate_scaffold(&valid_config()).unwrap();
        let desc = files.iter().find(|f| f.path == "description.yml").unwrap();
        assert!(desc.content.contains("language: C"));
        assert!(!desc.content.contains("language: C++"));
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
        assert!(desc.content.contains("wasm_mvp"));
        assert!(desc.content.contains("wasm_eh"));
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
    }
}
