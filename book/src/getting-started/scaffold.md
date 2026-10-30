# Project Scaffold

`quack_rs::scaffold::generate_scaffold` generates every file of a new DuckDB community
extension project in Rust — `Cargo.toml`, `Makefile`, CI workflows, `description.yml`, an
example function and its tests, a README, the license text and a `cargo-deny` configuration
— from a single function call, and the `quack_scaffold` command does the same from the
command line.

---

## What it generates

```text
my_extension/
├── Cargo.toml                          # cdylib + rlib crate, MSRV, dependencies, release profile
├── Makefile                            # delegates to cargo + extension-ci-tools
├── README.md                           # build, test and try instructions, and what to submit
├── LICENSE                             # the license text (see "License files" below)
├── deny.toml                           # cargo-deny configuration
├── src/
│   ├── lib.rs                          # entry point, an example scalar function, unit test
│   └── wasm_lib.rs                     # WASM staticlib shim
├── description.yml                     # community extension metadata
├── docs/
│   └── function_descriptions.csv       # the function descriptions the community site shows
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

`generate_scaffold_with_warnings` returns a `Scaffold`: the same files, plus warnings about
values that were accepted but deserve a second look (a license outside quack-rs's shortlist,
an excluded platform that is opt-in, every platform excluded, an unstable-ABI project the
community build cannot ship as generated). `generate_scaffold` is the same call without the
warnings.

### The `quack_scaffold` command

```bash
cargo install quack-rs --bin quack_scaffold     # once (0.20.0 or later)
quack_scaffold my_extension \
    --description "What the extension does" \
    --maintainer "Your Name" \
    --github-repo yourorg/duckdb-my-extension   # writes ./my_extension

# An extension using the duckdb-1-5* features: C_STRUCT, C API v1.5.6
quack_scaffold my_extension --description "..." --maintainer "..." \
    --github-repo yourorg/duckdb-my-extension --target-duckdb-version v1.5.6
```

Every `ScaffoldConfig` field is a flag; `--description`, `--maintainer` and `--github-repo`
are required, and each value can also be written `--flag=VALUE`:

| Flag | Field | Default |
|------|-------|---------|
| `--version` | `version` | `0.1.0` |
| `--license` | `license` | `MIT` |
| `--excluded-platforms` | `excluded_platforms`, separated by `;` or `,` (an empty value excludes none) | Wasm and MinGW |
| `--target-duckdb-version` | `target_duckdb_version`: the C API version (`v1.2.0`, or `v1.5.6` for the `duckdb-1-5*` features), or with `--unstable-c-api` the exact DuckDB 1.x release | `v1.2.0` |
| `--unstable-c-api` | `use_unstable_c_api` | off |
| `--git-ref` | `git_ref` | the placeholder |
| `--dir` | where to write the project | `./<name>` |

The command refuses to overwrite a file it generates unless `--force` is given, and writes
nothing when it refuses; files it does not generate are left alone, and it never writes
through a symbolic link inside the project. Warnings go to stderr. When it is done it prints
the next steps: the `git submodule add` command for extension-ci-tools, the build and test
commands, and the exact `append_metadata` invocation for stamping a build without `make`.
(Older releases ship no such command: use the program above in a project that already
depends on `quack-rs`.)

`--target-duckdb-version v1.5.6` sets `target_duckdb_version` to the C API version `v1.5.6`:
the binary loads into DuckDB 1.5.6 and 2.0.0, and the generated `Cargo.toml` pins the
bindings to `~1.10506.0` and enables the `duckdb-1-5*` feature it describes in a comment.
`--unstable-c-api` with `--target-duckdb-version v1.5.5` pins the binary to one DuckDB 1.x
release instead; DuckDB 2.0 cannot load such a binary, and the scaffold refuses a 2.x release
there.

---

## ScaffoldConfig fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Extension name — must match `[lib] name` in Cargo.toml and `description.yml` |
| `description` | `String` | Description for `description.yml` and the `//!` docs of `src/lib.rs`. Quoted and escaped, so `:`, `#` and quotes are fine; must not be empty, padded with whitespace, or contain control characters other than newline and tab |
| `version` | `String` | The extension's version, also the Cargo package version — so it must be semver without a leading `v` (`validate_semver`): `0.1.0`, not `v0.1.0`, `1.0` or a date stamp. A component above `u64::MAX` is refused |
| `license` | `String` | SPDX license identifier or expression (e.g., `"MIT"`, `"MIT OR Apache-2.0"`). An identifier outside quack-rs's shortlist (`BUSL-1.1`, `CC0-1.0`) is accepted with a warning; the wrong case of a listed one (`mit`), or a character SPDX does not allow, is an error. Line breaks and runs of whitespace are normalised to single spaces |
| `maintainer` | `String` | Your name or org, listed in `description.yml` — one non-empty line |
| `github_repo` | `String` | `"owner/repo"`, in the characters GitHub allows |
| `excluded_platforms` | `Vec<String>` | Platforms the community build skips, written to `description.yml` and the distribution workflow alike. Defaults to `DEFAULT_EXCLUDED_PLATFORMS` (Wasm and MinGW). An entry that excludes nothing is refused: the community build compares exact names, so a platform group such as `wasm` (the error lists its platforms), a retired platform or a name it does not build is an error |
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

The scaffold generator checks the first two rules before generating any files, and
refuses the names DuckDB itself reserves (aliases such as `s3` and `md`, and the core
extensions: see [Names DuckDB reserves](../publishing.md#names-duckdb-reserves)). It
returns an error if the name breaks one. Uniqueness among community extensions is yours
to check.

---

## After scaffolding

You need `git` with a user name and email configured (the first commit fails without),
`make`, Python 3 with the `venv` module, and network access to PyPI and GitHub:
`make configure` creates a virtual environment, installs the `duckdb` package from PyPI,
and installs the SQLLogicTest runner from a pinned commit on GitHub.

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

**Leave `CARGO_TARGET_DIR` unset for `make`.** extension-ci-tools' `rust.Makefile` copies
the library from `./target/release/`; with `CARGO_TARGET_DIR` set, Cargo builds elsewhere
and `make release` stops with `FileNotFoundError: './target/release/lib<name>.so'`
(reported by the author-tables review, which saw it with a scaffolded project).

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
version into `configure/extension_version.txt`, which the metadata footer
holds in a 32-byte field. extension-ci-tools writes every tag on `HEAD`, one
per line, or the short commit hash, or, with no commit, git's error message;
several tags, a tag over 32 bytes or that message overflow the field, and DuckDB
then refuses the file with `Unknown ABI type for extension`. The generated
`Makefile` chooses the value itself: the first tag on `HEAD` if it fits and holds
only `A-Za-z0-9._+-`, otherwise the short commit hash, and before the first commit
the Cargo package version (`v0.1.0`). `make EXTENSION_VERSION=v0.1.0 configure`
sets one explicitly. The value is cached, so run `make clean_all configure` after
committing or tagging.

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

## The generated project

- **The example function** is a native scalar function, `my_extension_hello(name)`, written
  with `ScalarFunctionBuilder::map1_str` as a one-line closure: a NULL argument gives NULL
  without calling it, and a panic fails the query instead of aborting. (Before 0.20.0 it was
  a `CREATE MACRO`; a macro is written into the user's database, so `LOAD` failed on a
  read-only one.)
- **The SQLLogicTest** carries the header lines the upstream tests have (`# name:`,
  `# description:`, `# group:`), checks that the function is missing before `require`, and
  reads its NULL from a column, because a literal `NULL` is constant-folded before the
  function is called.
- **`make test` fails when the runner skips a test file or runs none.** The SQLLogicTest
  runner reports a file whose `require` is not met as `SKIPPED` and exits 0, which is what a
  rename that leaves the extension name, the `[lib]` name and the test file out of step looks
  like, so `make test` would pass having tested nothing.
- **`Cargo.toml`** names `libduckdb-sys` as `>=1.4.4, <1.20100` with
  `default-features = false`, and, for a project stamped C API `v1.5.6` (or an unstable
  DuckDB 1.5 target), enables the `duckdb-1-5*` feature its comments describe on `quack-rs`.
- **License files.** `LICENSE` holds the text for MIT, Apache-2.0, BSD-2-Clause,
  BSD-3-Clause, ISC, 0BSD, MIT-0 and Unlicense. An expression gets one `LICENSE-<ID>` per
  license; any other license gets a `LICENSE.TODO` that names the text to add.
- **`deny.toml`** configures `cargo deny check`. It ignores RUSTSEC-2026-0367, -0368 and -0369,
  each with a reason: they concern the `zip` crate before 9, which is only a build-script
  dependency of `libduckdb-sys` up to 1.10506.0, the release Cargo resolves for
  `rust-version = 1.86`. It also allows the licenses of the dependency graph and the
  project's own.
- **`extension-ci.yml`** sets `permissions: contents: read`, checks out with
  `persist-credentials: false`, and pins `Swatinem/rust-cache` and `dtolnay/rust-toolchain`
  to commit hashes.
- **`README.md`** says how to build, test and try the extension, that a stamped extension must
  not be `strip`ped, that pull requests get a reduced CI run, and what to submit.
- **`docs/function_descriptions.csv`** has a row for the example function: the extension page on
  duckdb.org takes function descriptions and examples only from this file, which belongs in
  the community-extensions repository next to `description.yml` when you submit (see
  [Community Extensions](../publishing.md)).

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
