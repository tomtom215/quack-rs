<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215) -->

# CI/CD Workflows

This directory contains all GitHub Actions workflows for the quack-rs project.

## Workflow overview

| Workflow | File | Trigger | Purpose |
|----------|------|---------|---------|
| **CI** | `ci.yml` | Push to `main`/`claude/**`, PRs to `main` | All quality gates: check, test, clippy, fmt, doc, MSRV, bench-compile, example, scaffold, symbol, publish dry-run, security |
| **Release** | `release.yml` | Semver tags (`vX.Y.Z`) | Full release pipeline: validate, CI gate, package with SLSA attestation, GitHub release, crates.io publish |
| **Documentation** | `docs.yml` | Push to `main`, manual | Build mdBook and deploy to GitHub Pages |
| **Coverage** | `coverage.yml` | Push to `main`, PRs to `main` | Generate LCOV coverage report and upload to Codecov |
| **Mutation Testing** | `mutants.yml` | Manual dispatch, PRs to `main` | Verify tests detect code changes via cargo-mutants |
| **Benchmarks** | `benchmarks.yml` | Push to `main` (bench/src changes), manual | Run criterion benchmarks and archive reports |

## Quality gates (enforced by CI)

Every job in `ci.yml` must be green before merging a PR, except the three marked
informational. This table is generated from `ci.yml`; the hand-written list it
replaced had drifted to fewer than half the jobs.

| Job ID | Name | |
|---|---|---|
| `check` | Check | blocking |
| `test` | Test (${{ matrix.os }}) | blocking |
| `test-bundled` | Test bundled-test (${{ matrix.os }}) | blocking |
| `test-bundled-prebuilt` | Test bundled-test-prebuilt (prebuilt libduckdb) | blocking |
| `test-duckdb-1-5` | Test duckdb-1-5 / duckdb-1-5-3 / duckdb-1-5-4 features | blocking |
| `wasm` | WASM (wasm32-unknown-emscripten) | blocking |
| `clippy` | Clippy | blocking |
| `clippy-beta` | Clippy (beta, informational) | informational |
| `fmt` | Format | blocking |
| `doc` | Documentation | blocking |
| `msrv` | MSRV (1.86.0) | blocking |
| `bench-compile` | Benchmark (compile check) | blocking |
| `example-check` | Example (hello-ext · ${{ matrix.os }}) | blocking |
| `scaffold-compile` | Scaffold (compile check) | blocking |
| `symbol-check` | Symbol check (hello-ext · ${{ matrix.os }}) | blocking |
| `abi-table` | ABI layout table (vs upstream DuckDB headers) | blocking |
| `platform-table` | Platform list (vs upstream distribution matrix) | blocking |
| `spdx-list` | SPDX shortlist (vs official registry) | blocking |
| `msrv-vs-duckdb-ci` | MSRV vs DuckDB's extension CI | blocking |
| `extension-load` | Extension load test (DuckDB ${{ matrix.duckdb }}) | blocking |
| `scaffold-e2e` | Scaffold end-to-end (build, stamp, load, query) | blocking |
| `abi-guard` | ABI guard rejects a cross-version unstable build | blocking |
| `publish-dry-run` | Publish dry-run | blocking |
| `security` | Security (cargo-deny) | blocking |
| `osv-scan` | Security (OSV / GHSA) | blocking |
| `nightly` | Nightly (informational) | informational |
| `miri` | Miri (undefined behaviour) | blocking |
| `leak-check` | LeakSanitizer (RAII wrappers vs a real DuckDB) | blocking |
| `asan` | AddressSanitizer (informational) | informational |
| `semver` | Public API (semver-checks) | blocking |
| `fuzz` | Fuzz (smoke) | blocking |

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

`actionlint` also catches it and catches much more besides. It is not wired in
here only because it ships as a Go binary from GitHub releases and cannot be
checksum-pinned the way `osv-scanner` is in `ci.yml`. If a pinned, verifiable
install becomes available, adding it would be a strict improvement.

## Adding a new workflow

1. Create a new `.yml` file in this directory.
2. Pin all third-party actions to their full commit SHA (not a tag).
3. Add SPDX license header.
4. Update this README.
