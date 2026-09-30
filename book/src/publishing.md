# Community Extensions

DuckDB's community extensions repository lets anyone publish a loadable
extension that DuckDB users install with `INSTALL … FROM community`. This page
covers scaffolding, `description.yml`, naming, versioning, platforms, build
settings and submission for a community extension built with quack-rs.

---

## Prerequisites

- A working extension that passes local E2E tests
- A GitHub repository (the community build runs from it)
- All functions tested with SQLLogicTest format
- A globally unique extension name

---

## Scaffolding a new project

`quack_rs::scaffold::generate_scaffold` generates the project files in one call.
It returns paths relative to the project root and writes nothing itself, so join
them under the project directory — never write them
relative to the current directory, which would overwrite its `Cargo.toml` and
`src/lib.rs`:

```rust,no_run
use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};
use std::path::Path;

let config = ScaffoldConfig {
    name: "my_extension".to_string(),
    description: "Does something useful".to_string(),
    version: "0.1.0".to_string(),
    license: "MIT".to_string(),
    maintainer: "Your Name".to_string(),
    github_repo: "yourorg/duckdb-my-extension".to_string(),
    excluded_platforms: vec![],
    // `target_duckdb_version`, `use_unstable_c_api` and `git_ref` default to the
    // stable-ABI settings; see `concepts/abi.md` for when to change them.
    ..ScaffoldConfig::default()
};

let files = generate_scaffold(&config).expect("scaffold failed");
let root = Path::new(&config.name); // ./my_extension/
for file in &files {
    let path = root.join(&file.path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &file.content).unwrap();
}
```

In a new repository, add the build tooling submodule once with
`git submodule add https://github.com/duckdb/extension-ci-tools.git extension-ci-tools`
(`git submodule update --init` does nothing until then — Pitfall P4).

This generates:

```text
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

The file the scaffold generates, with the fields a community submission uses:

```yaml
extension:
  name: my_extension
  description: One-line description of what your extension does
  version: 0.1.0
  language: Rust
  build: cargo
  license: MIT
  requires_toolchains: rust;python3
  # excluded_platforms: "wasm_mvp;wasm_eh;wasm_threads"   # optional
  maintainers:
    - Your Name

repo:
  github: yourorg/duckdb-my-extension
  # Must be a commit hash, not a branch: the community repository builds
  # exactly this revision and signs the result, so a moving reference would
  # make the build unreproducible. DuckDB's docs: "Provide the hash of the
  # latest commit on the branch targeting stable as `ref`".
  ref: 0a11ddc058beb2d480ccbfa83e16a68400c5d076
  # ref_next: <hash>   # optional: a revision compatible with DuckDB main,
  #                    # used while a new DuckDB release is being prepared.

docs:
  hello_world: |
    SELECT my_extension_hello('world');
  extended_description: |
    A longer description, rendered on the community-extensions site.
```

Use `quack_rs::validate` to pre-validate fields before submission:

```rust
use quack_rs::validate::{
    validate_extension_name,
    validate_extension_version,
    validate_spdx_license,
    validate_excluded_platforms_str,
};

# fn main() -> Result<(), quack_rs::error::ExtensionError> {
validate_extension_name("my_extension")?;
validate_extension_version("0.1.0")?;
validate_spdx_license("MIT")?;
validate_excluded_platforms_str("wasm_mvp;wasm_eh")?;
# Ok(())
# }
```

---

## Naming rules

Extension names must satisfy **all** of the following:

- Match `^[a-z][a-z0-9_]*$` (lowercase, digits, underscores — no hyphens, because DuckDB
  looks up the entry point as `<name>_init_c_api`)
- Not exceed 64 characters
- Be **globally unique** across the entire DuckDB community extensions ecosystem

Check existing names at [community-extensions.duckdb.org](https://community-extensions.duckdb.org/)
before choosing. Use vendor-prefixed names to avoid collisions:

```text
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
| 7–40 lowercase hex chars (a git hash) | `690bfc5` | Unstable — no guarantees |
| `0.y.z` | `0.1.0` | Pre-release — working toward stability |
| `x.y.z` (x > 0) | `1.0.0` | Stable — full semver guarantees |

`validate_extension_version` is deliberately permissive: it accepts these three
formats and anything else made of `[A-Za-z0-9._+-]`, such as the date-based build ids
some published extensions use. `classify_extension_version` accepts only the three
formats above and returns the stability tier:

```rust
use quack_rs::validate::semver::{classify_extension_version, ExtensionStability};

# fn main() -> Result<(), quack_rs::error::ExtensionError> {
// Returns the tier and the version string it classified.
let (stability, _version) = classify_extension_version("0.1.0")?;
match stability {
    ExtensionStability::Unstable => println!("git hash"),
    ExtensionStability::PreRelease => println!("0.y.z"),
    ExtensionStability::Stable => println!("x.y.z, x>0"),
}
# Ok(())
# }
```

---

## Platform targets

Community extensions are built for:

| Platform | Description | Opt-in? |
|----------|-------------|---------|
| `linux_amd64` | Linux x86_64 (glibc) | |
| `linux_amd64_musl` | Linux x86_64 (musl) | yes |
| `linux_arm64` | Linux AArch64 (glibc) | |
| `linux_arm64_musl` | Linux AArch64 (musl) | yes |
| `osx_amd64` | macOS x86_64 | |
| `osx_arm64` | macOS Apple Silicon | |
| `windows_amd64` | Windows x86_64 | |
| `windows_amd64_mingw` | Windows x86_64 (MinGW) | |
| `windows_arm64` | Windows AArch64 | yes |
| `wasm_mvp` | WebAssembly (MVP) | |
| `wasm_eh` | WebAssembly (exception handling) | |
| `wasm_threads` | WebAssembly (threads) | |

An **opt-in** platform is not built unless an extension asks for it, so listing
one in `excluded_platforms` has no effect. `validate::platform::is_opt_in_platform`
reports which these are.

`linux_amd64_gcc4` used to appear in this table and no longer exists: DuckDB
retired the legacy CXX ABI target, and `DuckDBPlatform()` now raises a compile
error rather than emitting a `_gcc4` suffix. `validate_platform` rejects it with
that explanation.

This table is derived from `config/distribution_matrix.json` in
[`duckdb/extension-ci-tools`][ci-tools], and `scripts/check-platform-table.py`
fails CI when quack-rs's copy drifts from it.

[ci-tools]: https://github.com/duckdb/extension-ci-tools/blob/main/config/distribution_matrix.json

If your extension cannot be built for a platform (e.g., it uses a
platform-specific system library), add it to `excluded_platforms`:

```rust
use quack_rs::scaffold::ScaffoldConfig;

let config = ScaffoldConfig {
    excluded_platforms: vec![
        "wasm_mvp".to_string(),
        "wasm_eh".to_string(),
        "wasm_threads".to_string(),
    ],
    ..ScaffoldConfig::default()
};
```

Validate individual platform names with `validate_platform`:

```rust
use quack_rs::validate::validate_platform;
assert!(validate_platform("linux_amd64").is_ok());
assert!(validate_platform("invalid").is_err());
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
crate-type = ["cdylib"]

[dependencies]
quack-rs = "0.18"
libduckdb-sys = { version = ">=1.4.4, <2", features = ["loadable-extension"] }

[profile.release]
panic = "unwind"             # Required — "abort" disables quack-rs's panic guards
opt-level = 3
lto = true
codegen-units = 1
strip = true
```

This matches the scaffold's `Cargo.toml` (which also declares a `staticlib` example
target for WebAssembly builds).

> **ADR-4** (in `LESSONS.md`) — Do NOT use the `duckdb` crate's `bundled` feature. A
> loadable extension must call into the DuckDB that loads it, not bundle
> its own copy. `libduckdb-sys` with `loadable-extension` provides function
> pointers that are filled in at load time from the API struct DuckDB passes in.

---

## Release profile check

`validate_release_profile` checks the four release-profile settings. Only
`panic = "unwind"` is required; `lto = true`, `opt-level = 3` and `codegen-units = 1`
are recommended, and the returned `ReleaseProfileCheck` reports each one:

```rust
use quack_rs::validate::validate_release_profile;

// Pass all four release profile settings from your Cargo.toml
assert!(validate_release_profile("unwind", "true", "3", "1").is_ok());
// Err — see below
assert!(validate_release_profile("abort", "true", "3", "1").is_err());
```

`panic` must be `"unwind"`. quack-rs wraps every `extern "C"` entry point in
`catch_unwind` so a panic in your code becomes a DuckDB error rather than a
crash, and `catch_unwind` cannot catch anything under `panic = "abort"`: the
runtime aborts before unwinding starts, killing the user's DuckDB session.

The older advice to set `abort` came from panics escaping an `extern "C"`
boundary once being undefined behavior. They no longer are — Rust defines that
as an abort — and quack-rs catches them before the boundary anyway.

---

## CI workflow

The scaffold generates `.github/workflows/extension-ci.yml`, which:

1. Runs on pushes and pull requests to `main`
2. Runs `cargo fmt --check` and `cargo clippy` on Linux, and `cargo test` on Linux,
   macOS and Windows
3. Runs `make configure` and `make release`, which use `extension-ci-tools` to build
   the `.duckdb_extension` file
4. Runs the SQLLogicTests in `test/sql` with `make test`

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

An extension that uses only the stable C API (the default quack-rs features) is
stamped `C_STRUCT` with C API version `v1.2.0`, and one binary loads into every
DuckDB 1.4.x and 1.5.x release for its platform. The `libduckdb-sys = ">=1.4.4, <2"`
range above is correct for such an extension.

An extension that enables the `duckdb-1-5*` features must be stamped
`C_STRUCT_UNSTABLE` with the exact DuckDB release it was built against, and DuckDB
loads it only into that release. Pin `libduckdb-sys` to that release's bindings
(`~1.10505.0` for DuckDB 1.5.5) and rebuild for each new DuckDB release. See
[ABI Compatibility](concepts/abi.md).

The community build pipeline rebuilds extensions for each DuckDB release.

> **Pitfall P2** — For `C_STRUCT`, the `-dv` flag to `append_extension_metadata.py`
> must be the **C API version** (`v1.2.0`), not the DuckDB release version (`v1.4.4`).
> Use `quack_rs::DUCKDB_API_VERSION` to avoid hardcoding this.

---

## Security considerations

The DuckDB team does not audit community extensions for security, so the
responsibility is yours:

- Never let a panic escape an FFI boundary: quack-rs's callbacks catch panics and
  report them as SQL errors, which requires `panic = "unwind"`
- Validate user inputs at system boundaries (extension entry point is the boundary)
- Do not include secrets, API keys, or credentials in your binary
- Dynamic SQL in SQL macros must not construct queries from unsanitized user data
