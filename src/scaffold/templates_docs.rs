// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The generated project's `README.md`, `docs/function_descriptions.csv` and
//! `deny.toml`.

use std::fmt::Write as _;

use super::ScaffoldConfig;

/// A stub `README.md`: what the extension is, how to build, test and try it,
/// and what the community-extensions submission needs from this repository.
pub(super) fn generate_readme(config: &ScaffoldConfig) -> String {
    let name = &config.name;
    let license = config
        .license
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let ci_tools_ref = super::extension_ci_tools_ref(config);
    let release_refs = if config.use_unstable_c_api {
        format!(
            "\n- This project is stamped `C_STRUCT_UNSTABLE` for DuckDB {target} and loads into \
             no other release. The community build builds `repo.ref` for DuckDB {latest}, \
             `repo.variegata` for v1.5.6 and `repo.andium` for v1.4.5: submit a revision \
             targeting v1.5.6 or v1.4.5 under that key, and point `repo.ref` at a revision \
             stamped `C_STRUCT`.\n",
            target = config.target_duckdb_version,
            latest = super::DISTRIBUTION_DUCKDB_VERSION,
        )
    } else {
        String::new()
    };
    format!(
        "# {name}\n\
         \n\
         {description}\n\
         \n\
         A DuckDB extension built with [quack-rs](https://github.com/tomtom215/quack-rs).\n\
         \n\
         ## Build and test\n\
         \n\
         You need Rust {msrv} or newer, Python 3 with `venv`, `make` and `git`.\n\
         \n\
         ```sh\n\
         # Once, in a new repository (the Makefile reminds you otherwise):\n\
         git submodule add -b {ci_tools_ref} \\\n    https://github.com/duckdb/extension-ci-tools.git extension-ci-tools\n\
         make configure   # a Python venv with DuckDB and its SQLLogicTest runner\n\
         make release     # build/release/{name}.duckdb_extension\n\
         make test        # test/sql/*.test against it; a skipped test file fails\n\
         cargo test       # the Rust unit tests\n\
         ```\n\
         \n\
         Leave `CARGO_TARGET_DIR` unset for `make`: extension-ci-tools copies the\n\
         library from `./target`.\n\
         \n\
         ## Try it\n\
         \n\
         ```sh\n\
         duckdb -unsigned -c \"LOAD 'build/release/{name}.duckdb_extension'; SELECT {name}_hello('world');\"\n\
         ```\n\
         \n\
         Do not `strip` the `.duckdb_extension` file: DuckDB checks the metadata at its\n\
         end, and stripping removes it. (Cargo's `strip = true` runs before the metadata\n\
         is appended, and is fine.)\n\
         \n\
         ## Publishing to the DuckDB community repository\n\
         \n\
         A pull request to [duckdb/community-extensions](https://github.com/duckdb/community-extensions)\n\
         adds `extensions/{name}/` with two files from this repository:\n\
         \n\
         - `description.yml`, with `repo.ref` set to the hash of the commit to build.\n\
         - `docs/function_descriptions.csv`, as `extensions/{name}/docs/function_descriptions.csv`.\n\
           \x20 The extension's page on duckdb.org lists every function it registers, but takes\n\
           \x20 their descriptions and examples only from this file (the C extension API cannot\n\
           \x20 attach them to a function); a function missing from it is listed without them.\n\
         \n\
         That pull request, like one to this repository, is built in reduced CI mode:\n\
         only `linux_amd64`, `windows_amd64` and `wasm_eh`. macOS, Linux arm64 and the\n\
         other platforms first build after the merge, so run the distribution workflow\n\
         here on `main` (or with `reduced_ci_mode: disabled`) before submitting.\n\
         {release_refs}\
         \n\
         ## License\n\
         \n\
         `{license}`: see the `LICENSE*` file(s).\n",
        description = config.description,
        msrv = env!("CARGO_PKG_RUST_VERSION"),
    )
}

/// `docs/function_descriptions.csv`: what the community documentation site
/// shows for each function (`scripts/generate_md.sh` in
/// `duckdb/community-extensions` joins `duckdb_functions()` with it on
/// `function`), with a row for the example function.
pub(super) fn generate_function_descriptions(config: &ScaffoldConfig) -> String {
    let name = &config.name;
    let row = [
        format!("{name}_hello"),
        "scalar".to_string(),
        format!("Returns 'Hello from {name}! ' followed by the argument, or NULL for NULL."),
        String::new(),
        format!("SELECT {name}_hello('world');"),
    ];
    let row: Vec<String> = row.iter().map(|field| csv_field(field)).collect();
    format!(
        "function,function_type,description,comment,example\n{}\n",
        row.join(",")
    )
}

/// `field` as a CSV field: quoted, with quotes doubled, when it holds a comma,
/// a quote or a line break.
fn csv_field(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// `deny.toml` for `cargo deny check`.
///
/// The three ignored advisories are for `zip` before 9.0.0, which reaches a
/// new project through `libduckdb-sys` up to 1.10506.0 — the version Cargo
/// picks for `rust-version = 1.86` — as a build dependency of its build
/// script. That script uses it only to unpack a `libduckdb` archive it
/// downloads when `DUCKDB_DOWNLOAD_LIB` is set; nothing of it is linked into
/// the extension.
///
/// The allowed licenses are those `cargo deny list` reports for a generated
/// project locked against quack-rs 0.19.0 with `libduckdb-sys` 1.10506.0 and
/// with 1.20000.1 (the same set), plus each license of the project's own
/// expression.
pub(super) fn generate_deny_toml(config: &ScaffoldConfig) -> String {
    let mut allow: Vec<String> = DEPENDENCY_LICENSES
        .iter()
        .map(|l| (*l).to_string())
        .collect();
    for term in license_terms(&config.license) {
        if !allow.contains(&term) {
            allow.push(term);
        }
    }
    let allow = allow.iter().fold(String::new(), |mut list, license| {
        let _ = writeln!(list, "    \"{license}\",");
        list
    });
    DENY_TOML.replace("@@LICENSES@@", &format!("allow = [\n{allow}]"))
}

/// The licenses of the dependency graph of a generated project.
const DEPENDENCY_LICENSES: [&str; 10] = [
    "0BSD",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-3-Clause",
    "CDLA-Permissive-2.0",
    "ISC",
    "MIT",
    "Unicode-3.0",
    "Unlicense",
    "Zlib",
];

/// The license terms of an SPDX expression the scaffold has checked: each
/// identifier, with its `WITH` exception if it has one.
fn license_terms(license: &str) -> Vec<String> {
    let tokens: Vec<&str> = license
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|t| !t.is_empty())
        .collect();
    let mut terms = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "AND" | "OR" => i += 1,
            id if tokens.get(i + 1) == Some(&"WITH") => {
                if let Some(exception) = tokens.get(i + 2) {
                    terms.push(format!("{id} WITH {exception}"));
                }
                i += 3;
            }
            id => {
                terms.push(id.to_string());
                i += 1;
            }
        }
    }
    terms
}

const DENY_TOML: &str = r#"# cargo-deny configuration (https://embarkstudios.github.io/cargo-deny/),
# written by the quack-rs scaffold: `cargo deny check` checks the dependency
# graph for security advisories, licenses and sources.

[advisories]
ignore = [
    # zip before 9.0.0. It reaches this project only as a build dependency of
    # libduckdb-sys up to 1.10506.0, the bindings Cargo picks for
    # rust-version 1.86; libduckdb-sys's build script uses it only to unpack a
    # libduckdb archive it downloads when DUCKDB_DOWNLOAD_LIB is set, which
    # building this extension does not do. No zip code is linked into the
    # extension. Delete these entries once Cargo.lock has zip 9 or none.
    { id = "RUSTSEC-2026-0367", reason = "zip < 9 in libduckdb-sys's build script only: extraction runs only with DUCKDB_DOWNLOAD_LIB set; not linked into the extension" },
    { id = "RUSTSEC-2026-0368", reason = "zip < 9 in libduckdb-sys's build script only: extraction runs only with DUCKDB_DOWNLOAD_LIB set; not linked into the extension" },
    { id = "RUSTSEC-2026-0369", reason = "zip < 9 in libduckdb-sys's build script only: extraction runs only with DUCKDB_DOWNLOAD_LIB set; not linked into the extension" },
]

[licenses]
# The licenses of quack-rs's and libduckdb-sys's dependencies (build
# dependencies included) when this file was generated, and this project's
# own. Add one when `cargo deny check licenses` rejects a dependency whose
# license you accept.
@@LICENSES@@

[bans]
multiple-versions = "allow"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
"#;
