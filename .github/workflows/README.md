<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215) -->

# CI/CD Workflows

This directory contains all GitHub Actions workflows for the quack-rs project.

## Workflow overview

| Workflow | File | Trigger | Purpose |
|----------|------|---------|---------|
| **CI** | `ci.yml` | Push to `main`/`claude/**`, PRs to `main`, weekly schedule, manual, called by `release.yml` | Every quality gate (table below), summarised by the `ci-ok` job |
| **Release** | `release.yml` | Semver tags (`vX.Y.Z`, and `vX.Y.Z-pre` prereleases) | Full release pipeline: validate the tag and the release metadata, run all of `ci.yml`, package with SLSA attestation, publish to crates.io (approval required; the `.crate` is re-packaged and compared with the attested hash before the upload), check crates.io serves the attested `.crate`, GitHub release, versioned-book deploy |
| **Documentation** | `docs.yml` | Push to `main`, manual, dispatched by `release.yml` after a release | Build the versioned book (latest release at `/`, each release line at `/vX.Y/`, `main` at `/dev/`) and deploy to GitHub Pages |
| **Coverage** | `coverage.yml` | Push to `main`, PRs to `main` | Generate LCOV coverage report and upload to Codecov |
| **Mutation Testing** | `mutants.yml` | Manual dispatch, PRs to `main` | Verify tests detect code changes via cargo-mutants |
| **Benchmarks** | `benchmarks.yml` | Push to `main` (changes under `benches/` or `src/`, or to `Cargo.toml` / `Cargo.lock`), manual | Run criterion benchmarks and archive reports |

## Quality gates (enforced by CI)

Every job in `ci.yml` must be green before merging a PR, except the four marked
informational. This table is generated from `ci.yml`; the hand-written list it
replaced had drifted to fewer than half the jobs.

| Job ID | Name | |
|---|---|---|
| `check` | Check | blocking |
| `test` | Test (${{ matrix.os }}) | blocking |
| `test-bundled` | Test bundled-test (${{ matrix.os }}) | blocking |
| `test-bundled-prebuilt` | Test bundled-test-prebuilt (prebuilt libduckdb) | blocking |
| `test-older-engines` | Test against DuckDB ${{ matrix.engine }} (${{ matrix.features }}) | blocking |
| `test-duckdb-2-0` | Test against DuckDB v2.0.0 | blocking |
| `test-bundled-duckdb-2-0` | Test bundled-test on the DuckDB 2.0 bindings (informational) | informational |
| `test-duckdb-1-5` | Test duckdb-1-5 / duckdb-1-5-3 / duckdb-1-5-4 features | blocking |
| `wasm` | WASM (wasm32-unknown-emscripten) | blocking |
| `clippy` | Clippy | blocking |
| `clippy-beta` | Clippy (beta, informational) | informational |
| `latest-stable` | Latest stable Rust (informational) | informational |
| `fmt` | Format | blocking |
| `doc` | Documentation | blocking |
| `msrv` | MSRV (1.86.0) | blocking |
| `dependency-floor` | Dependency floor (duckdb / libduckdb-sys 1.4.4) | blocking |
| `bench-compile` | Benchmark (compile check) | blocking |
| `example-check` | Example (hello-ext · ${{ matrix.os }}) | blocking |
| `scaffold-compile` | Scaffold (compile check) | blocking |
| `symbol-check` | Symbol check (hello-ext · ${{ matrix.os }}) | blocking |
| `abi-table` | ABI layout table (vs upstream DuckDB headers), and every C API slot wrapped or listed | blocking |
| `platform-table` | Platform list (vs upstream distribution matrix) | blocking |
| `spdx-list` | SPDX shortlist (vs official registry) | blocking |
| `autoload-entries` | Catalog autoload names (vs every supported DuckDB) | blocking |
| `book` | Book builds (mdBook) | blocking |
| `msrv-vs-duckdb-ci` | MSRV vs DuckDB's extension CI | blocking |
| `extension-load` | Extension load test (DuckDB ${{ matrix.duckdb }}) | blocking |
| `scaffold-e2e` | Scaffold end-to-end (build, stamp, load, query) | blocking |
| `abi-guard` | ABI guard refuses a build whose declared DuckDB version contradicts its bindings | blocking |
| `abi-guard-layout` | ABI guard refuses a real layout mismatch (and accepts the matching DuckDB) | blocking |
| `publish-dry-run` | Publish dry-run | blocking |
| `security` | Security (cargo-deny) | blocking |
| `osv-scan` | Security (OSV / GHSA) | blocking |
| `hygiene` | Hygiene (typos, taplo, machete) | blocking |
| `nightly` | Nightly (informational) | informational |
| `miri` | Miri (undefined behaviour) | blocking |
| `leak-check` | LeakSanitizer (RAII wrappers vs a real DuckDB) | blocking |
| `asan` | AddressSanitizer | blocking |
| `semver` | Public API (semver-checks) | blocking |
| `fuzz` | Fuzz (smoke) | blocking |
| `workflow-lint` | Workflow lint (actionlint) | blocking |
| `ci-ok` | CI passed | blocking (aggregate) |

**Branch protection should require exactly one check: `CI passed`** (the
`ci-ok` job). It `needs:` every blocking job and runs with `if: always()`, so
it fails when any of them fails or is cancelled.
`scripts/check-ci-aggregate.py` (run by the `doc` job) fails when a blocking
job is missing from its `needs`, so a new job cannot drop out of the required
set unnoticed; it also fails when a job turns informational without being
named in its `INFORMATIONAL` set, when a blocking job (or `ci-ok` itself) has a
step-level `continue-on-error`, and when the table above disagrees with
`ci.yml`. `release.yml` runs this whole workflow as its CI gate
(`workflow_call`), so a release passes the same checks.

Regenerate after adding or renaming a job:

```bash
python3 - <<'EOF'
import yaml, pathlib
d = yaml.safe_load(pathlib.Path(".github/workflows/ci.yml").read_text())
for k, j in d["jobs"].items():
    kind = "informational" if j.get("continue-on-error") else "blocking"
    print(f'| `{k}` | {j.get("name", k)} | {kind} |')
EOF
```

Coverage (`coverage.yml`), mutation testing (`mutants.yml`), docs (`docs.yml`),
benchmarks (`benchmarks.yml`) and release (`release.yml`) run in their own
workflows.

## Pitfall: an invalid workflow file fails silently

GitHub evaluates `${` + `{ ... }}` expressions **anywhere in the file**,
including inside a shell comment in a `run:` block. An empty one is a syntax
error that invalidates the entire workflow:

```
Invalid workflow file: .github/workflows/mutants.yml
(Line: 203, Col: 14): An expression was expected
```

This does not fail loudly. GitHub records a run with **zero jobs** and the
workflow stops running — `mutants.yml` silently stopped executing on pull
requests this way, so the mutation gate was not running at all while every
other check stayed green.

Neither `yaml.safe_load` nor a JSON-Schema validator catches it, because both
treat the `run:` block as an opaque string. `scripts/check-workflow-expressions.py`
runs in the `doc` job and does.

`actionlint` also catches it and catches much more besides; the `workflow-lint`
job runs it from the release tarball, pinned by version and sha256 the way
`osv-scanner` is (the release publishes `actionlint_<version>_checksums.txt`).

## Pinning, and the checks that hold it

Everything CI downloads is pinned, and a script in the `doc` job keeps each
kind of pin consistent:

| What | Pinned by | Checked by |
|---|---|---|
| Actions | full commit SHA, version in the trailing comment (`# v2.9.3`, or `# master` for `dtolnay/rust-toolchain`, which has no release tags) | `scripts/check-action-pins.py`: one commit per action across all workflows, and the same commits in the workflows the scaffold generates (`src/scaffold/templates_ci.rs`), which Dependabot does not update |
| Rust | an exact release in `rust-toolchain.toml`, repeated in every `toolchain:` input | `scripts/check-toolchain-pin.py`: `stable` / `beta` only in informational jobs; MSRV and nightly where they are the point |
| Release binaries (osv-scanner, actionlint, typos, taplo, cargo-machete) | version and SHA-256 in the job | the job itself (`sha256sum -c`) |
| Cargo-installed tools (cargo-deny, cargo-semver-checks, cargo-fuzz, mdBook, ...) | `cargo install --locked --version` | — |

The five jobs that compare a checked-in table with upstream (`abi-table`, which
has two such steps, `platform-table`, `spdx-list`, `autoload-entries`,
`msrv-vs-duckdb-ci`) run each script through `scripts/upstream-guard.sh`:
exit 1 (drift) fails; exit 2
("could not check": upstream unreachable or unparseable) is a warning on a
pull request or a schedule run and a failure on a release tag, where nothing
may be skipped; a missing script fails. `upstream-guard.sh --self-test` runs in
the `doc` job.

## Adding a new workflow

1. Create a new `.yml` file in this directory.
2. Pin all third-party actions to their full commit SHA (not a tag), at the
   commit the other workflows use (`scripts/check-action-pins.py`).
3. Install Rust with `toolchain:` set to the release `rust-toolchain.toml`
   pins (`scripts/check-toolchain-pin.py`).
4. Add SPDX license header.
5. Update this README.
