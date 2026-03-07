# Project Scaffold

`quack_rs::scaffold::generate_scaffold` generates a complete, submission-ready DuckDB
community extension project from a single function call. No manual file creation, no
copy-pasting templates.

---

## What it generates

```
my_extension/
├── Cargo.toml                          # cdylib crate, pinned deps, release profile
├── Makefile                            # delegates to cargo + extension-ci-tools
├── extension_config.cmake              # required by extension-ci-tools
├── src/
│   ├── lib.rs                          # entry point template
│   └── wasm_lib.rs                     # WASM staticlib shim
├── description.yml                     # community extension metadata
├── test/
│   └── sql/
│       └── my_extension.test           # SQLLogicTest skeleton
├── .github/
│   └── workflows/
│       └── extension-ci.yml            # cross-platform CI workflow
├── .gitmodules                         # extension-ci-tools submodule
├── .gitignore
└── .cargo/
    └── config.toml                     # Windows CRT static linking
```

---

## Usage

```rust
use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};
use std::path::Path;

fn main() {
    let config = ScaffoldConfig {
        name: "my_extension".to_string(),
        description: "My DuckDB extension".to_string(),
        version: "0.1.0".to_string(),
        license: "MIT".to_string(),
        maintainer: "Your Name".to_string(),
        github_repo: "yourorg/duckdb-my-extension".to_string(),
        excluded_platforms: vec![],
    };

    let files = generate_scaffold(&config).expect("scaffold generation failed");

    for file in &files {
        let path = Path::new(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, &file.content).unwrap();
        println!("created {}", file.path);
    }
}
```

---

## ScaffoldConfig fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Extension name — must match `[lib] name` in Cargo.toml and `description.yml` |
| `description` | `String` | One-line description for `description.yml` |
| `version` | `String` | Semver or git hash — validated by `validate_extension_version` |
| `license` | `String` | SPDX license identifier (e.g., `"MIT"`, `"Apache-2.0"`) |
| `maintainer` | `String` | Your name or org, listed in `description.yml` |
| `github_repo` | `String` | `"owner/repo"` format |
| `excluded_platforms` | `Vec<String>` | Platforms to skip (e.g., `["wasm_mvp", "wasm_eh"]`) |

---

## Name validation

Extension names must satisfy all of:
- Match `^[a-z][a-z0-9_-]*$`
- Not exceed 64 characters
- Be globally unique on [community-extensions.duckdb.org](https://community-extensions.duckdb.org/)

Use vendor-prefixed names to avoid collisions: `myorg_analytics`, not `analytics`.

The scaffold generator validates the name before generating any files and returns an error
if it violates the rules.

---

## After scaffolding

```bash
cd my_extension
git init
git submodule add https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
git submodule update --init --recursive
make configure
make release
```

Then add your function logic in `src/lib.rs`, write your SQLLogicTests in
`test/sql/my_extension.test`, and push to GitHub — CI runs automatically.

---

## Excluded platforms

Some extensions cannot be built for all platforms (e.g., extensions that depend on
platform-specific system libraries, or WASM environments that lack threading).

```rust
ScaffoldConfig {
    excluded_platforms: vec![
        "wasm_mvp".to_string(),
        "wasm_eh".to_string(),
        "wasm_threads".to_string(),
    ],
    // ...
}
```

Validate individual platform names with `quack_rs::validate::validate_platform`, or a
semicolon-delimited string (as used in `description.yml`) with
`quack_rs::validate::validate_excluded_platforms_str`.
