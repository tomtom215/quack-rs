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
`src/lib.rs`. From 0.20.0 the `quack_scaffold` command
(`cargo install quack-rs --bin quack_scaffold`) writes the project for you, refuses to
overwrite an existing file without `--force`, and prints the next steps; see
[Project Scaffold](getting-started/scaffold.md). The same call from Rust:

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
`git submodule add -b v2.0-cyanoptera https://github.com/duckdb/extension-ci-tools.git extension-ci-tools`
(the branch for DuckDB 2.0; the scaffold's `Makefile` names the branch it expects)
(`git submodule update --init` does nothing until then — Pitfall P4), and commit
before the first `make configure`; the full sequence is under
[CI workflow](#ci-workflow).

This generates:

```text
my_extension/
├── Cargo.toml
├── Makefile
├── README.md
├── LICENSE
├── deny.toml
├── src/lib.rs
├── src/wasm_lib.rs
├── description.yml
├── docs/function_descriptions.csv
├── test/sql/my_extension.test
├── .github/workflows/extension-ci.yml
├── .github/workflows/MainDistributionPipeline.yml
├── .gitmodules
├── .gitignore
└── .cargo/config.toml
```

---

## `description.yml`

This is the file `generate_scaffold` writes for the configuration above (generated, with
`git_ref` set to a commit hash, `description` to the text shown, and the comments
included). Every text field is quoted because the community build reads the file with
PyYAML (YAML 1.1), where an unquoted `yes` is a boolean and a version such as `1.0` is a
number. An `excluded_platforms` line appears only when `excluded_platforms` is not empty,
and `extended_description` repeats `description` until you replace it.

```yaml
extension:
  name: "my_extension"
  description: "One-line description of what your extension does"
  version: "0.1.0"
  language: Rust
  build: cargo
  license: "MIT"
  requires_toolchains: rust;python3
  maintainers:
    - "Your Name"

repo:
  github: "yourorg/duckdb-my-extension"
  # Must be a commit hash, not a branch.
  ref: "0a11ddc058beb2d480ccbfa83e16a68400c5d076"
  # Optional, each a revision to build instead of `ref` (which is built for
  # the newest release, v2.0.0):
  # variegata: <hash>  # for DuckDB v1.5.6
  # andium: <hash>     # for DuckDB v1.4.5
  # ref_next: <hash>   # for a DuckDB release still in preparation

docs:
  hello_world: |
    SELECT my_extension_hello('world');
  extended_description: "One-line description of what your extension does"
```

`my_extension_hello` is the native scalar function the generated `src/lib.rs` registers, so
the `hello_world` example works as generated.

### `repo.ref` and the other revision keys

DuckDB's
[community extension documentation](https://duckdb.org/community_extensions/development)
asks for a commit: "Provide the hash of the latest commit on the branch targeting
stable as `ref`". The community repository builds exactly that revision and signs
the result, so a branch name, which moves, would make the build unreproducible.
Use a full commit hash. (A few published descriptors pin a tag instead; none pins
a branch.)

The community build also reads three optional keys under `repo`. Its `scripts/build.py`
chooses the revision for the DuckDB version it is building:

| DuckDB version being built | Revision used |
|----------------------------|---------------|
| the latest stable release (v2.0.0 when this was written) | `ref` |
| v1.5.6 (the `v1.5-variegata` line) | `variegata` if set, else `ref` |
| v1.4.5 (the `v1.4-andium` line) | `andium` if set, else `ref` |
| any other version (the development line) | `ref_next` if set, else `ref`; a prerelease validation run can be configured to skip an extension that has no `ref_next` |

An extension that uses only the stable C API (the quack-rs default, stamped `C_STRUCT`
with C API `v1.2.0`) loads into all of those releases from one build, so one `ref`
is enough. An extension whose Makefile pins `TARGET_DUCKDB_VERSION` with
`USE_UNSTABLE_C_API=1` (a `ScaffoldConfig` with `use_unstable_c_api = true`) needs one
revision per line: the build compiles whatever revision it is given with the extension's
own Makefile, so a revision that pins DuckDB v1.4.4 is stamped `C_STRUCT_UNSTABLE` for
v1.4.4 whichever release the community build is running, and every other release refuses
to load it (the conform review built such a project: DuckDB 1.5.6 and 2.0.0 both refused it
with "The file was built specifically for DuckDB version 'v1.4.4'"). Put the revision that targets DuckDB 2.0
under `ref`, the one that targets 1.5.6 under `variegata` and the one that targets 1.4.5
under `andium`. The scaffold writes a comment for each of `variegata`, `andium` and
`ref_next`, and for a `C_STRUCT_UNSTABLE` project a comment that says the project cannot ship
as generated, with a warning from `generate_scaffold_with_warnings` and from `quack_scaffold`;
the wiring is yours to do.

`ScaffoldConfig::git_ref` accepts letters, digits, `-`, `_`, `.` and `/`, and defaults
to the placeholder `REPLACE_WITH_COMMIT_HASH`, which `git` cannot resolve, so an unedited
file cannot be built by accident. `parse_description_yml` (and with it
`validate_description_yml_str`) requires `repo.ref` to be non-empty and **warns** when it
is not 7 to 64 hexadecimal digits: a tag is legal, and a branch name does not fail the
build, it only moves. From 0.20.0 the same check applies to `variegata` and `andium` (an
empty value is an error), and both are returned as `DescriptionYml::git_ref_variegata`
and `git_ref_andium`.

### What the community build reads

`scripts/build.py` in `duckdb/community-extensions` reads `description.yml` like this:

- It starts with `yaml.safe_load` (PyYAML). From 0.20.0 `validate_description_yml_str`
  and `parse_description_yml` read the whole document by PyYAML's rules (a plain scalar
  PyYAML refuses, a tab indent, an unterminated quote or an undefined alias anywhere in the
  file is an error), in time linear in the file size; in a few documented cases they are
  stricter than PyYAML (a repeated or quoted key under `extension:` or `repo:`, a tag, a
  second document). Before 0.20.0 they checked only the fields quack-rs knows. Running
  `python3 -c 'import yaml; yaml.safe_load(open("description.yml"))'` yourself is still
  the cheapest second opinion.
- It requires the directory name `extensions/<dir>/` to equal `extension.name`.
- It reads `extension.name`, `repo.github` and `repo.ref`, and, when present,
  `repo.canonical_name`, `repo.ref_next`, `repo.variegata`, `repo.andium`,
  `extension.excluded_platforms`, `extension.opt_in_platforms`,
  `extension.requires_toolchains`, `extension.custom_toolchain_script`,
  `extension.vcpkg_url`, `extension.vcpkg_commit` and `extension.test_config`.
  **Any other key is ignored without a message**: a misspelt `excluded_platform:` or
  `ref_nxt:` does nothing.
- `requires_toolchains` is a `;`-separated list. The workflow looks for each token
  between semicolons (`;rust;`), so write `rust;python3` with no spaces: in
  `rust; python3` the second token is `" python3"` and is lost. The workflow acts on
  `rust` and `go` (checked) and, per the conform review, on `python3`, `parser_tools`,
  `fortran`, `unixodbc`, `multimedia`, `cuda`, `omp` and `downgraded_aws_cli`; an unknown
  token is ignored too.
- `test_config` is a JSON string of environment variables for the test run, for example
  `{"test_env_variables": {"SKIP_TESTS": "1"}}`. quack-rs neither generates nor validates it.

### Validating the whole file

`validate_description_yml_str` checks a complete `description.yml`, and
`parse_description_yml` returns its fields for inspection. Non-fatal findings,
such as a license that SPDX does not list, are collected in `warnings` instead of
failing. `validate_rust_extension` additionally checks the fields a Rust
extension needs (`language: Rust`, `build: cargo`, `rust` in
`requires_toolchains`):

```rust,no_run
use quack_rs::error::ExtensionError;
use quack_rs::validate::description_yml::{
    parse_description_yml, validate_description_yml_str, validate_rust_extension,
};

fn check_description() -> Result<(), ExtensionError> {
    let text = std::fs::read_to_string("description.yml")?;
    validate_description_yml_str(&text)?;

    let desc = parse_description_yml(&text)?;
    println!("{} {}", desc.name, desc.version.as_deref().unwrap_or("(no version)"));
    for warning in &desc.warnings {
        println!("warning: {warning}");
    }
    validate_rust_extension(&desc)
}
# check_description().unwrap();
```

The rule for each field is in the
[`description_yml` module documentation](https://docs.rs/quack-rs/latest/quack_rs/validate/description_yml/index.html).

The individual validators check one field at a time:

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
before choosing; quack-rs cannot check uniqueness for you (the directory names under
`extensions/` in the community-extensions repository are the authority). Use
vendor-prefixed names to avoid collisions:

```text
myorg_analytics   ✓
analytics         ✗  (likely taken or too generic)
```

Function names are SQL identifiers, checked by `validate_function_name`:
letters, digits and underscores, not starting with a digit. Mixed case is
allowed (DuckDB itself ships `formatReadableSize`), and reserved keywords are
rejected:

```rust
use quack_rs::validate::validate_function_name;

assert!(validate_function_name("word_count").is_ok());
assert!(validate_function_name("WordCount").is_ok());
assert!(validate_function_name("word-count").is_err()); // needs quoting in SQL
assert!(validate_function_name("1word").is_err());      // parsed as a number
assert!(validate_function_name("order").is_err());      // reserved keyword
```

### Names DuckDB reserves

DuckDB rewrites some names before `INSTALL` and `LOAD` look anything up
(`ExtensionHelper::ApplyExtensionAlias`), and it already ships the core extensions. An
extension with one of these names can never be installed by that name from the community
repository. Measured on DuckDB 2.0.0: `INSTALL s3 FROM community;` reports `Failed to download
extension "httpfs" at URL ".../v2.0.0/linux_amd64/httpfs.duckdb_extension.gz" (HTTP 404)`, and
`md` and `sqlite3` fail the same way for `motherduck` and `sqlite_scanner`.

| Kind | Names |
|------|-------|
| Aliases (`name` → what DuckDB looks up) | `http`, `https`, `s3` → `httpfs`; `md` → `motherduck`; `mysql` → `mysql_scanner`; `odbc` → `odbc_scanner`; `postgres` → `postgres_scanner`; `sqlite`, `sqlite3` → `sqlite_scanner`; `uc_catalog` → `unity_catalog` |
| The 34 core extensions (the union of what DuckDB 1.4.4, 1.5.6 and 2.0.0 build in or autoload, and the DuckDB 2.0 documentation's list) | `autocomplete`, `avro`, `aws`, `azure`, `core_functions`, `delta`, `ducklake`, `encodings`, `excel`, `fts`, `httpfs`, `iceberg`, `icu`, `inet`, `jemalloc`, `json`, `lance`, `legacy_parser`, `motherduck`, `mysql_scanner`, `odbc_scanner`, `parquet`, `postgres_scanner`, `quack`, `sirius`, `spatial`, `sqlite_scanner`, `sqlsmith`, `tpcds`, `tpch`, `ui`, `unity_catalog`, `vortex`, `vss` |

The community-extensions FAQ says a community extension whose name matches a core
extension "has to be renamed". Since 0.20.0 `generate_scaffold`, `parse_description_yml`
and `quack_rs::validate::validate_community_extension_name` refuse all 44 names (the ten
aliases and the 34 core names) and say which extension an alias stands for;
`validate_extension_name` stays the syntax check, which accepts DuckDB's own names.
Whether another *community* extension already has your name is still yours to look up
(`extensions/` in `duckdb/community-extensions`).

The list changes with DuckDB releases: `scripts/check-extension-names.py` in the quack-rs
repository compares quack-rs's copy with `src/main/extension/extension_alias.cpp`,
`extension_helper.cpp` and `extension_entries.hpp` in the DuckDB source and with the
core extensions table of the DuckDB documentation. For a name you are about to choose,
read those rather than this page.

### Function descriptions

The community extensions site lists the functions your extension adds, with a description,
a comment and examples for each. The C API v1 that quack-rs uses has no call that sets a
function's description (the C API v2 has `duckdb_v2_function_docs_*`), so by default the
page shows a name and a type and nothing else. To fill the text in, add
`extensions/<name>/docs/function_descriptions.csv` **to the community-extensions
repository**, in the same pull request as `description.yml`; the file is not read from
your extension's repository. Its header is `function,description,comment,example` (some
published files also carry a `function_type` column, which is ignored). If the file
exists, its columns **replace** the generated ones through a left join on the function
name: a function that is missing from the CSV loses any description, it is not merged with
one. About one extension in eight ships the file (52 of 395 descriptors when this was
checked). The scaffold writes `docs/function_descriptions.csv` with a row for its example
function; copy it to `extensions/<name>/docs/` in your pull request to the
community-extensions repository.

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
formats and any other string of up to 64 characters made of `[A-Za-z0-9._+-]`,
such as the date-based build ids some published extensions use. `classify_extension_version` accepts only the three
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

### Which version DuckDB shows

`description.yml`'s `version` is documentation: DuckDB does not read it. The version that
`duckdb_extensions()` reports comes from the metadata footer, and `make configure` writes it
from git: **the tag on `HEAD`, if there is one, otherwise the abbreviated hash of the
commit** (`git tag --points-at HEAD`, then `git log -1 --format=%h` in
`extension-ci-tools`' `configure_helper.py`; it is not `git describe`). So the community
build reports a short hash for your extension unless the commit it builds carries a tag.
Two things break the footer: a tag longer than the footer's fixed 32-byte version field
(the conform review saw DuckDB 2.0.0 refuse such a file with an `INTERNAL Error: Unknown ABI
type for extension`), and several tags on one commit (the tool joins them with newlines;
measured: tags `v0.1.0` and `v0.1.0-rc1` on one commit give the two-line value
"v0.1.0" then "v0.1.0-rc1"). Keep tags short and put one tag on a released commit. The
`Makefile` that the scaffold generates since 0.20.0 sets `EXTENSION_VERSION` itself (the
first tag on `HEAD` if it fits and holds only `A-Za-z0-9._+-`, else the short hash, else the
Cargo version before the first commit). A project that does not set it, such as one from an
older scaffold, gets `extension-ci-tools`' value, so the rules above apply to it.

---

## Platform targets

Community extensions are built for:

| Platform | Description | Opt-in? |
|----------|-------------|---------|
| `linux_amd64` | Linux x86_64 (glibc) | |
| `linux_amd64_musl` | Linux x86_64 (musl) | yes |
| `linux_arm64` | Linux AArch64 (glibc) | |
| `linux_arm64_musl` | Linux AArch64 (musl) | yes |
| `osx_amd64` | macOS x86_64 | yes, in `extension-ci-tools` for DuckDB 2.0; DuckDB 2.1 drops the platform |
| `osx_arm64` | macOS Apple Silicon | |
| `windows_amd64` | Windows x86_64 | |
| `windows_amd64_mingw` | Windows x86_64 (MinGW) | |
| `windows_arm64` | Windows AArch64 | yes |
| `wasm_mvp` | WebAssembly (MVP) | |
| `wasm_eh` | WebAssembly (exception handling) | |
| `wasm_threads` | WebAssembly (threads) | |

An **opt-in** platform is not built unless an extension asks for it, so listing
one in `excluded_platforms` has no effect. `validate::platform::is_opt_in_platform`
reports which these are. To have one built, list it in `extension.opt_in_platforms`
(a `;`-separated string, like `excluded_platforms`).

Both fields are matched against the platform names above **exactly**. A group
name such as `wasm` or `windows` is not a platform. On the 1.x lines
(`v1.5-variegata`, `v1.4-andium`) the matrix comes from
`modify_distribution_matrix.py`, which splits on `;` only and tests
`arch in excluded_arch_values`; on `v2.0-cyanoptera` it comes from the Go program
`extbuild matrix` (which the conform review read to trim spaces and split on `,` too;
not run here). Either way `excluded_platforms: "wasm"` leaves all three `wasm_*`
platforms building. Write `wasm_mvp;wasm_eh;wasm_threads`: separated by `;`, no spaces,
which both accept. Published files do contain group names, so
`parse_description_yml` accepts them, with a warning that spells out the
platforms the group stands for; it also warns about an `opt_in_platforms` entry
that is a group, is not an opt-in platform, or is not a platform at all, all of
which the build ignores without a message.

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
# With resolver 3, Cargo picks dependency versions that build on this. Without
# these two lines, a fresh lock file selects libduckdb-sys 1.20000.x, which needs
# Rust 1.88: the WebAssembly jobs of the 1.x extension-ci-tools lines pin 1.86.0.
rust-version = "1.86"
resolver = "3"

[lib]
name = "my_extension"       # Must match description.yml `name`
crate-type = ["cdylib"]

[dependencies]
quack-rs = "0.20"

[dependencies.libduckdb-sys]
version = ">=1.4.4, <1.20100"
default-features = false
features = ["loadable-extension"]

[profile.release]
panic = "unwind"             # Required — "abort" disables quack-rs's panic guards
opt-level = 3
lto = true
codegen-units = 1
strip = true
```

The scaffold's `Cargo.toml` has these settings and adds `license`, explanatory comments
and a `staticlib` example target for WebAssembly builds. For a project stamped C API `v1.5.6`
it also enables the `duckdb-1-5*` feature on `quack-rs` and pins `libduckdb-sys` to
`~1.10506.0`.

> **[ADR-4](https://github.com/tomtom215/quack-rs/blob/main/docs/architecture.md#adr-4-libduckdb-sys-only-at-run-time-no-duckdb-crate)**
> — Do NOT use the `duckdb` crate's `bundled` feature. A
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

The older advice to set `abort` dates from when a panic escaping an `extern "C"`
function was undefined behavior. It now aborts the process by definition, and
quack-rs catches panics before the boundary anyway.

---

## CI workflow

The scaffold generates two workflows.

`.github/workflows/extension-ci.yml`, the fast check on every push and pull
request to `main`:

1. Runs `cargo fmt --check` and `cargo clippy` on Linux, and `cargo test` on Linux,
   macOS and Windows
2. Builds on the minimum Rust version (1.86.0, which the WebAssembly builds of
   DuckDB 1.x extensions pin)
3. Runs `make configure` and `make release`, which use `extension-ci-tools` to build
   the `.duckdb_extension` file
4. Runs the SQLLogicTests in `test/sql` with `make test`

The shared distribution workflow (`MainDistributionPipeline.yml`) does **not**
run your SQL tests on most platforms. In `extension-ci-tools` at
`v2.0-cyanoptera`, `base.Makefile` sets `SKIP_TESTS` for `linux_amd64`, both musl
platforms and the MinGW builds, `ci_phase.py` skips `linux_arm64` and any macOS
build but native arm64, and the Wasm jobs have no test target. A failure in
`test/sql` that only shows on Linux is therefore not caught by the community
build, which is why the generated `extension-ci.yml` runs `make test` itself and
leaves `DUCKDB_PLATFORM` blank on `ubuntu-latest` (a set value would make the
Makefile skip the tests there too).

`.github/workflows/MainDistributionPipeline.yml` calls `extension-ci-tools`'
`_extension_distribution.yml` — the workflow the community build itself runs —
for every platform the community build targets, minus `excluded_platforms`. Its
`exclude_archs` and `description.yml`'s `excluded_platforms` are generated from
the same setting, so they cannot disagree. The scaffold excludes the
WebAssembly (`wasm_mvp`, `wasm_eh`, `wasm_threads`) and `windows_amd64_mingw`
targets by default; set `excluded_platforms` to opt them back in once your
extension builds there.

**On a pull request the community build runs a reduced matrix.** The default
`reduced_ci_mode: auto` builds only the platforms marked `run_in_reduced_ci_mode` in
`config/distribution_matrix.json` (`linux_amd64`, `windows_amd64` and `wasm_eh` on
`v2.0-cyanoptera`) and builds the rest after the merge, so a failure that is specific
to `osx_arm64` or `linux_arm64` shows up after your pull request is merged. (That
`auto` means "reduced" for a pull request was read in `extbuild` by the conform
review; the matrix flags and the workflow input are from the upstream files.)

**Do not run `strip` on a stamped extension.** DuckDB's documentation warns that stripping
removes or corrupts the metadata footer. Measured: `strip` on a stamped extension exits 0,
and `LOAD` then fails with `The file is not a DuckDB extension. The metadata at the end
of the file is invalid`. Cargo's `strip = true` is safe because it runs when the library
is linked, before the footer is appended.

After scaffolding:

```bash
cd my_extension
git init
# The extension-ci-tools branch the community build uses for your DuckDB release:
# v2.0-cyanoptera for DuckDB 2.0 (there is no v2.0.0 branch).
git submodule add -b v2.0-cyanoptera https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
# Cargo.lock is tracked on purpose: the community build rebuilds your `repo.ref`
# for every DuckDB release, and a missing lock file lets each rebuild re-resolve.
cargo generate-lockfile
# Commit before `make configure`.
git add -A && git commit -m "Initial commit"
make configure
make release
```

**Commit before `make configure`.** `make configure` writes the extension version
into `configure/extension_version.txt` from the tag on `HEAD`, or else the short hash of
the commit (see [Which version DuckDB shows](#which-version-duckdb-shows)). In a repository
with no commit, that command fails and its error text ("error: malformed object name
'HEAD'") is written to the file instead. The text overflows the 32-byte version
field of the metadata footer, and DuckDB then refuses the stamped file with an
error such as "Unknown ABI type for extension". If this has already happened,
commit, delete `configure/` (or run `make clean_all`), and run `make configure`
again.

**Pin `extension-ci-tools` to the community build's version.** The community
repository builds with the `extension-ci-tools` branch for the current DuckDB
release: for DuckDB v2.0.0 its build workflow uses
`_extension_distribution.yml` from the `v2.0-cyanoptera` branch, and `extension-ci-tools`
has no `v2.0.0` branch. (For DuckDB v1.5.6 it used `v1.5-variegata`; a `v1.5.6` branch
exists too, but it is a different commit.) Check out the matching branch so your local
build uses the same scripts. For DuckDB 2.0, `extension-ci-tools` pins Rust 1.97.1
on Windows and for WebAssembly (the `v1.5-variegata` branch pins 1.86.0 for
WebAssembly), installs the current stable on Linux (`rustup` inside the build
image) and uses the runner's Rust on macOS, with emsdk 6.0.9 (was 3.1.71);
quack-rs's MSRV stays 1.86.0. The branch names are listed by
`git ls-remote https://github.com/duckdb/extension-ci-tools`.

**Ignore `configure/`.** It holds the Python virtualenv, `platform.txt` and
`extension_version.txt` that `make configure` generates for this machine; do not
commit it.

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
DuckDB release from 1.4.4 through 2.0.x for its platform. The `libduckdb-sys = ">=1.4.4, <1.20100"`
range above is correct for such an extension.

An extension that enables the `duckdb-1-5*` features should be built against the
v1.5.2 – v1.5.6 bindings (pin `libduckdb-sys` to `~1.10506.0`) and stamped `C_STRUCT`
with C API version `v1.5.6` (`quack_rs::DUCKDB_API_VERSION_1_5_6`; with
`extension-ci-tools`, `TARGET_DUCKDB_VERSION=v1.5.6` and `USE_UNSTABLE_C_API` unset).
It then loads into DuckDB 1.5.6 and 2.0.0, and older releases refuse it at `LOAD`.
Do not stamp it `C_STRUCT_UNSTABLE`: that pins it to one 1.x release, and DuckDB 2.0
cannot load such a binary, because it routes it to the C API v2 entry point
`<name>_init_c_api_v2`, which quack-rs does not generate. See
[ABI Compatibility](concepts/abi.md).

The community build pipeline rebuilds extensions for each DuckDB release.

> **Pitfall P2** — For `C_STRUCT`, the `-dv` flag to `append_extension_metadata.py`
> must be the **C API version** (`v1.2.0`, or `v1.5.6` for a `duckdb-1-5*` build), not the
> DuckDB release version (`v1.4.4`). Use `quack_rs::DUCKDB_API_VERSION` (or
> `quack_rs::DUCKDB_API_VERSION_1_5_6`) to avoid hardcoding this.

---

## Security considerations

The DuckDB team does not audit community extensions for security, so the
responsibility is yours:

- Never let a panic escape an FFI boundary: quack-rs's callbacks catch panics and
  report them as SQL errors, which requires `panic = "unwind"`
- Validate user inputs at system boundaries (extension entry point is the boundary)
- Open every file path that comes from SQL with the `FileSystem` wrapper, not `std::fs`:
  `std::fs` ignores `SET enable_external_access = false` and `allowed_directories`, so a
  function that reads user-supplied paths with it lets a user read what the person running
  DuckDB meant to keep closed ([Virtual File System](duckdb-1-5/file-system.md#security-only-filesystem-honours-duckdbs-permission-settings))
- Do not include secrets, API keys, or credentials in your binary
- Dynamic SQL in SQL macros must not construct queries from unsanitized user data
