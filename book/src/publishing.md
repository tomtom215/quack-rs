# Community Extensions

DuckDB's community extension ecosystem allows anyone to publish a loadable
extension that DuckDB users can install with a single SQL command. This page
covers everything you need to submit and maintain a community extension built
with quack-rs.

---

## Prerequisites

- A working extension that passes local E2E tests
- A GitHub repository (the community build runs from it)
- All functions tested with SQLLogicTest format
- A globally unique extension name

---

## Scaffolding a new project

`quack_rs::scaffold::generate_scaffold` generates all required files from a
single function call:

```rust
use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};

let config = ScaffoldConfig {
    name: "my_extension".to_string(),
    description: "Does something useful".to_string(),
    version: "0.1.0".to_string(),
    license: "MIT".to_string(),
    maintainer: "Your Name".to_string(),
    github_repo: "yourorg/duckdb-my-extension".to_string(),
    excluded_platforms: vec![],
};

let files = generate_scaffold(&config).expect("scaffold failed");
for file in &files {
    std::fs::create_dir_all(std::path::Path::new(&file.path).parent().unwrap()).unwrap();
    std::fs::write(&file.path, &file.content).unwrap();
}
```

This generates:

```
my_extension/
├── Cargo.toml
├── Makefile
├── extension_config.cmake
├── src/lib.rs
├── src/wasm_lib.rs
├── description.yml
├── test/sql/my_extension.test
├── .github/workflows/extension-ci.yml
├── .gitmodules
├── .gitignore
└── .cargo/config.toml
```

---

## `description.yml`

Required fields for community submission:

```yaml
extension:
  name: my_extension
  description: One-line description of what your extension does
  version: 0.1.0
  language: Rust
  build: cargo
  license: MIT
  requires_toolchains: rust;python3
  excluded_platforms: ""   # or "wasm_mvp;wasm_eh;wasm_threads"
  maintainers:
    - Your Name

repo:
  github: yourorg/duckdb-my-extension
  ref: main
```

Use `quack_rs::validate` to pre-validate fields before submission:

```rust
use quack_rs::validate::{
    validate_extension_name,
    validate_extension_version,
    validate_spdx_license,
    validate_excluded_platforms_str,
};

validate_extension_name("my_extension")?;
validate_extension_version("0.1.0")?;
validate_spdx_license("MIT")?;
validate_excluded_platforms_str("wasm_mvp;wasm_eh")?;
```

---

## Naming rules

Extension names must satisfy **all** of the following:

- Match `^[a-z][a-z0-9_-]*$` (lowercase, digits, hyphens, underscores)
- Not exceed 64 characters
- Be **globally unique** across the entire DuckDB community extensions ecosystem

Check existing names at [community-extensions.duckdb.org](https://community-extensions.duckdb.org/)
before choosing. Use vendor-prefixed names to avoid collisions:

```
myorg_analytics   ✓
analytics         ✗  (likely taken or too generic)
```

> **Pitfall P1** — The `[lib] name` in `Cargo.toml` MUST exactly match the
> extension name. If your crate name is `duckdb-my-ext` (producing
> `libduckdb_my_ext.so`) but `description.yml` says `name: my_ext`, the
> community build fails with `FileNotFoundError`.

---

## Versioning

| Format | Example | Meaning |
|--------|---------|---------|
| 7+ hex chars | `690bfc5` | Unstable — no guarantees |
| `0.y.z` | `0.1.0` | Pre-release — working toward stability |
| `x.y.z` (x > 0) | `1.0.0` | Stable — full semver guarantees |

Use `validate_extension_version` to accept all three formats, and
`classify_extension_version` to determine the stability tier:

```rust
use quack_rs::validate::semver::classify_extension_version;

match classify_extension_version("0.1.0")? {
    ExtensionStability::Unstable => println!("git hash"),
    ExtensionStability::PreRelease => println!("0.y.z"),
    ExtensionStability::Stable => println!("x.y.z, x>0"),
}
```

---

## Platform targets

Community extensions are built for:

| Platform | Description |
|----------|-------------|
| `linux_amd64` | Linux x86_64 |
| `linux_amd64_gcc4` | Linux x86_64 (GCC 4 ABI) |
| `linux_arm64` | Linux AArch64 |
| `osx_amd64` | macOS x86_64 |
| `osx_arm64` | macOS Apple Silicon |
| `windows_amd64` | Windows x86_64 |
| `windows_amd64_mingw` | Windows x86_64 (MinGW) |
| `windows_arm64` | Windows AArch64 |
| `wasm_mvp` | WebAssembly (MVP) |
| `wasm_eh` | WebAssembly (exception handling) |
| `wasm_threads` | WebAssembly (threads) |

If your extension cannot be built for a platform (e.g., it uses a
platform-specific system library), add it to `excluded_platforms`:

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

Validate individual platform names with `validate_platform`:

```rust
use quack_rs::validate::validate_platform;
validate_platform("linux_amd64")?;  // Ok
validate_platform("invalid")?;       // Err
```

---

## `Cargo.toml` requirements

```toml
[package]
name = "my_extension"
version = "0.1.0"
edition = "2021"

[lib]
name = "my_extension"       # Must match description.yml `name`
crate-type = ["cdylib", "rlib"]

[dependencies]
quack-rs = "=0.3.0"          # Pin with = for binary compatibility
libduckdb-sys = { version = "=1.4.4", features = ["loadable-extension"] }

[profile.release]
panic = "abort"              # Required — no stack unwinding in FFI
opt-level = 3
lto = "thin"
strip = "symbols"
```

> **Pitfall ADR-1** — Do NOT use the `duckdb` crate's `bundled` feature. A
> loadable extension must link against the DuckDB that loads it, not bundle
> its own copy. `libduckdb-sys` with `loadable-extension` provides lazy function
> pointers populated by DuckDB at load time.

---

## Release profile check

The `validate_release_profile` validator checks that your release profile is
correctly configured:

```rust
use quack_rs::validate::validate_release_profile;

// Pass all four release profile settings from your Cargo.toml
validate_release_profile("abort", "true", "3", "1")?;   // Ok
validate_release_profile("unwind", "true", "3", "1")?;   // Err — panics across FFI are UB
```

---

## CI workflow

The scaffold generates `.github/workflows/extension-ci.yml` which:

1. Runs on push and pull request
2. Checks, lints, and tests in Rust (all platforms)
3. Calls `extension-ci-tools` to build the `.duckdb_extension` artifact
4. Runs SQLLogicTest integration tests

After scaffolding:

```bash
cd my_extension
git init
git submodule add https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
git submodule update --init --recursive
make configure
make release
```

> **Pitfall P4** — The `extension-ci-tools` submodule must be initialized.
> `make configure` fails if the submodule is missing.

---

## Submitting to the community registry

1. Create a pull request against the
   [community-extensions](https://github.com/duckdb/community-extensions)
   repository
2. Add your `description.yml` under `extensions/my_extension/description.yml`
3. CI runs automatically to verify the build
4. Once approved, users can install your extension:

```sql
INSTALL my_extension FROM community;
LOAD my_extension;
```

---

## Binary compatibility

Extension binaries are tied to a specific DuckDB version. When DuckDB releases
a new version:

- New binaries must be built against that version
- Old binaries will be refused by the new DuckDB runtime
- The community build pipeline re-builds all extensions for each DuckDB release

Pin `libduckdb-sys` with `=` (exact version) to ensure you always build against
the exact version you intend. The `quack_rs::DUCKDB_API_VERSION` constant
(`"v1.2.0"`) is passed to `init_extension` and must match the C API version
of your pinned `libduckdb-sys`.

> **Pitfall P2** — The `-dv` flag to `append_extension_metadata.py` must be the
> **C API version** (`v1.2.0`), not the DuckDB release version (`v1.4.4`).
> Use `quack_rs::DUCKDB_API_VERSION` to avoid hardcoding this.

---

## Security considerations

Community extensions are not vetted for security by the DuckDB team:

- Never panic across FFI boundaries (`panic = "abort"` enforces this)
- Validate user inputs at system boundaries (extension entry point is the boundary)
- Do not include secrets, API keys, or credentials in your binary
- Dynamic SQL in SQL macros must not construct queries from unsanitized user data
