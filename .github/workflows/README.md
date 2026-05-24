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

All of these must pass before merging any PR:

1. `cargo check --all-targets`
2. `cargo test --all-targets` (Linux, macOS, Windows)
3. `cargo test --all-targets --features bundled-test` (Linux, macOS, Windows)
4. `cargo clippy --all-targets -- -D warnings`
5. `cargo fmt -- --check`
6. `cargo doc --no-deps` (with `-D warnings`)
7. `cargo +1.87.0 check` (MSRV)
8. `cargo bench --no-run` (compile check)
9. Example extension build + clippy + test
10. Scaffold output compilation
11. Entry point symbol verification (Linux/macOS)
12. `cargo publish --dry-run`
13. `cargo deny check` (license/advisory/source)
14. Nightly Rust (informational, non-blocking)

## Adding a new workflow

1. Create a new `.yml` file in this directory.
2. Pin all third-party actions to their full commit SHA (not a tag).
3. Add SPDX license header.
4. Update this README.
