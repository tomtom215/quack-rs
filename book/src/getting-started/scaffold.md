# Project Scaffold

`quack_rs::scaffold::generate_scaffold` generates every file of a new DuckDB community
extension project in Rust — `Cargo.toml`, `Makefile`, CI workflow, `description.yml`, an
example function and its tests — from a single function call.

---

## What it generates

```text
my_extension/
├── Cargo.toml                          # cdylib + rlib crate, MSRV, dependencies, release profile
├── Makefile                            # delegates to cargo + extension-ci-tools
├── src/
│   ├── lib.rs                          # entry point, example function, unit test
│   └── wasm_lib.rs                     # WASM staticlib shim
├── description.yml                     # community extension metadata
├── test/
│   └── sql/
│       └── my_extension.test           # SQLLogicTest for the example function
├── .github/
│   └── workflows/
│       ├── extension-ci.yml            # fast native CI: lint, tests, MSRV build
│       └── MainDistributionPipeline.yml  # every community platform, via extension-ci-tools
├── .gitmodules                         # extension-ci-tools submodule (pinned branch)
├── .gitignore
└── .cargo/
    └── config.toml                     # Windows: static CRT; musl: dynamic CRT (a cdylib needs it)
```

---

## Usage

`generate_scaffold` returns paths relative to the project root (`Cargo.toml`,
`src/lib.rs`, ...) and writes nothing itself. Join them under the directory the
project should live in — writing them relative to the current directory would
overwrite whatever `Cargo.toml` and `src/lib.rs` are already there.

```rust,no_run
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
        // `excluded_platforms` defaults to DEFAULT_EXCLUDED_PLATFORMS (see
        // "Excluded platforms" below). `target_duckdb_version`,
        // `use_unstable_c_api` and `git_ref` default to the stable-ABI
        // settings; see `concepts/abi.md`.
        ..ScaffoldConfig::default()
    };

    let files = generate_scaffold(&config).expect("scaffold generation failed");

    // Everything goes under ./my_extension/, never into the current directory.
    let root = Path::new(&config.name);
    for file in &files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, &file.content).unwrap();
        println!("created {}", path.display());
    }
}
```

The repository's `scaffold_to_dir` example does the same from the command
line:

```bash
cargo run --example scaffold_to_dir -- my_extension            # writes ./my_extension
# An extension using the duckdb-1-5* features: C_STRUCT, C API v1.5.6
cargo run --example scaffold_to_dir -- my_extension ../my_extension --c-api v1.5.6
```

`--c-api v1.5.6` sets `target_duckdb_version` to `v1.5.6`: the binary loads into
DuckDB 1.5.6 and 2.0.0. `--unstable <release>` (`use_unstable_c_api`) pins the
binary to one DuckDB 1.x release instead, such as `--unstable v1.5.5`; DuckDB 2.0
cannot load such a binary, and the scaffold refuses a 2.x release there.

---

## ScaffoldConfig fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Extension name — must match `[lib] name` in Cargo.toml and `description.yml` |
| `description` | `String` | Description for `description.yml` and the `//!` docs of `src/lib.rs`. Quoted and escaped, so `:`, `#` and quotes are fine; must not be empty, padded with whitespace, or contain control characters other than newline and tab |
| `version` | `String` | The extension's version, also the Cargo package version — so it must be semver without a leading `v` (`validate_semver`): `0.1.0`, not `v0.1.0`, `1.0` or a date stamp |
| `license` | `String` | SPDX license identifier or expression (e.g., `"MIT"`, `"MIT OR Apache-2.0"`) |
| `maintainer` | `String` | Your name or org, listed in `description.yml` — one non-empty line |
| `github_repo` | `String` | `"owner/repo"`, in the characters GitHub allows |
| `excluded_platforms` | `Vec<String>` | Platforms the community build skips, written to `description.yml` and the distribution workflow alike. Defaults to `DEFAULT_EXCLUDED_PLATFORMS` (Wasm and MinGW) |
| `git_ref` | `String` | `repo.ref` — **a commit hash** (or tag), not a branch. Defaults to `REF_PLACEHOLDER` so it cannot be submitted unset |
| `target_duckdb_version` | `String` | Written as `TARGET_DUCKDB_VERSION` in the Makefile. Without `use_unstable_c_api`, the C API version: `DUCKDB_API_VERSION` (`v1.2.0`, the default) or `DUCKDB_API_VERSION_1_5_6` (`v1.5.6`) for an extension that enables `duckdb-1-5` / `duckdb-1-5-3` / `duckdb-1-5-4`; `v1.5.6` pins `libduckdb-sys` to `~1.10506.0` and loads into DuckDB 1.5.6 and 2.0.0. With `use_unstable_c_api`, an exact DuckDB 1.x release such as `v1.5.5` |
| `use_unstable_c_api` | `bool` | Stamps `C_STRUCT_UNSTABLE`, pinning the binary to the one DuckDB 1.x release in `target_duckdb_version`. DuckDB 2.0 cannot load such a binary, and 2.x releases are refused. Defaults to `false` |

---

## Name validation

Extension names must satisfy all of:
- Match `^[a-z][a-z0-9_]*$` (no hyphens: the entry point is `<name>_init_c_api`)
- Not exceed 64 characters
- Be globally unique on [community-extensions.duckdb.org](https://community-extensions.duckdb.org/)

Use vendor-prefixed names to avoid collisions: `myorg_analytics`, not `analytics`.

The scaffold generator checks the first two rules before generating any files and returns
an error if the name breaks one. Uniqueness is yours to check.

---

## After scaffolding

```bash
cd my_extension
git init
git submodule add -b v2.0-cyanoptera https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
cargo generate-lockfile
git add -A
git commit -m "Scaffold my_extension"
make configure
make release
make test
```

`cargo generate-lockfile` writes `Cargo.lock`, which `.gitignore` leaves tracked
on purpose. The community build checks out your `repo.ref` and rebuilds that same
commit for every DuckDB release; without a committed lock file, each rebuild
resolves its dependencies afresh. (`make release` would write the file too, but
after `git add -A`, so it would stay untracked.)

`git submodule add` is not optional in a new project: the scaffold writes
`.gitmodules`, but `git submodule update --init` does nothing until the
submodule has been added once (Pitfall P4). The generated `Makefile` stops with
this command if the checkout is missing. `-b v2.0-cyanoptera` checks out the
extension-ci-tools branch the generated `MainDistributionPipeline.yml` builds
with (`DISTRIBUTION_CI_TOOLS_REF`, the branch for `DISTRIBUTION_DUCKDB_VERSION`
`v2.0.0`; extension-ci-tools has no `v2.0.0` branch), or your
`target_duckdb_version` for an unstable-ABI build (`-b v1.5.5`, for example); the
community build uses the branch for the DuckDB release it targets, so keep the
submodule, `.gitmodules` and the workflow on the same one.

**Commit before `make configure`.** `make configure` writes the extension
version into `configure/extension_version.txt` from git — the tag on `HEAD`,
otherwise the short commit hash. With no commit, extension-ci-tools records
git's error message instead, which overflows the 32-byte version field of the
metadata footer, and DuckDB then refuses the file with
`Unknown ABI type for extension`. The generated `Makefile` guards against this
by using the Cargo package version (`v0.1.0`) while there is no commit, and
`make EXTENSION_VERSION=v0.1.0 configure` sets one explicitly. If you configured
before committing, run `make clean_all configure` after the first commit so the
version comes from git.

The project declares `rust-version` (quack-rs's MSRV, 1.86.0; for DuckDB 2.0,
extension-ci-tools pins Rust 1.97.1 on Windows and WebAssembly and uses the current
stable on Linux and macOS) and uses `resolver = "3"`, so
`cargo` resolves dependencies that build on that release; the generated CI's
`msrv` job checks it.

`cargo test` runs the generated unit test and `make test` runs
`test/sql/my_extension.test` against a real DuckDB. Replace the example
function in `src/lib.rs` with your own, give each new function a test in both
places, and push to GitHub — CI runs automatically.

---

## Excluded platforms

The community build compiles every extension for nine platforms with
extension-ci-tools' `_extension_distribution.yml`: `linux_amd64`, `linux_arm64`,
`osx_amd64`, `osx_arm64`, `windows_amd64`, `windows_amd64_mingw`, `wasm_mvp`,
`wasm_eh` and `wasm_threads`. For DuckDB 2.0, `osx_amd64` is opt-in in
extension-ci-tools (built only if an extension asks for it), and DuckDB 2.1 drops
the platform. The generated `MainDistributionPipeline.yml` calls the same
workflow, so a platform that fails there fails in your own CI first.

`excluded_platforms` is written both as `excluded_platforms` in
`description.yml` and as `exclude_archs` in that workflow, so the two always
agree. It defaults to `DEFAULT_EXCLUDED_PLATFORMS` — the three Wasm targets and
`windows_amd64_mingw` — which most `build: cargo` community extensions exclude:
they need the emscripten and MinGW toolchains, which a Rust extension rarely
builds with unchanged. To opt in, pass a shorter list (an empty one builds
everything), or in a generated project remove the platform from both
`description.yml` and `exclude_archs`, and let the distribution workflow prove
it builds:

```rust
use quack_rs::scaffold::ScaffoldConfig;

// Build Wasm too; keep only MinGW excluded.
let config = ScaffoldConfig {
    excluded_platforms: vec!["windows_amd64_mingw".to_string()],
    ..ScaffoldConfig::default()
};
```

Validate individual platform names with `quack_rs::validate::validate_platform`, or a
semicolon-delimited string (as used in `description.yml`) with
`quack_rs::validate::validate_excluded_platforms_str`.
