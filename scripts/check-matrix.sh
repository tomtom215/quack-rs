#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Runs the feature/target combinations CI runs, locally, in one command.
#
# CI's matrix is wider than the obvious "default + all features": in particular
# it builds with `bundled-test` *without* `duckdb-1-5`, and it compiles the lib
# for `wasm32-unknown-emscripten`. Both caught defects that every other
# combination compiled through:
#
#   - a `#[test]` calling a `duckdb-1-5`-gated method with no `cfg` gate breaks
#     `--features bundled-test` alone and nothing else;
#   - `1 << 37` in a `usize` const is a const-eval overflow only where pointers
#     are 32 bits, i.e. wasm32 — a target DuckDB's own extension CI builds.
#
# Usage:
#   scripts/check-matrix.sh            # compile checks + lints (fast)
#   scripts/check-matrix.sh --tests    # ...and run the test suites (slow, needs disk)
#
# The wasm32 legs are skipped with a notice if the target is not installed:
#   rustup target add wasm32-unknown-emscripten

set -uo pipefail
cd "$(dirname "$0")/.."

# CI sets these globally, which promotes every warning to an error. Without
# them a plain `cargo check` here passes on code CI rejects -- an unnecessary
# `unsafe` block in a test is a warning locally and a build failure there.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:--D warnings}"

run_tests=0
[[ "${1:-}" == "--tests" ]] && run_tests=1

failures=0
run() {
    printf '%-70s ' "$1"
    if output=$(eval "$1" 2>&1); then
        echo 'ok'
    else
        echo 'FAILED'
        printf '%s\n' "$output" | grep -E '^error' -A6 | head -30
        failures=$((failures + 1))
    fi
}

# Keep these lists in step with .github/workflows/ci.yml. CI's highest
# feature level is `duckdb-1-5-4` -- the only one that compiles `src/arrow.rs`
# and the 1.5.4 C API wrappers -- so every level it builds is built here too.
echo '── formatting and lints ─────────────────────────────────────────────────'
run 'cargo fmt --all -- --check'
run 'cargo clippy --all-targets -- -D warnings'
run 'cargo clippy --all-targets --features duckdb-1-5 -- -D warnings'
run 'cargo clippy --all-targets --features duckdb-1-5-3 -- -D warnings'
run 'cargo clippy --all-targets --features duckdb-1-5-4 -- -D warnings'
# `bundled-test` compiles the live-DuckDB tests and the `testing` module,
# which no other combination lints.
run 'cargo clippy --all-targets --features bundled-test -- -D warnings'
run 'cargo clippy --all-targets --features bundled-test,duckdb-1-5-4 -- -D warnings'

echo '── compile checks, every feature combination CI builds ──────────────────'
run 'cargo check --all-targets'
run 'cargo check --all-targets --features duckdb-1-5'
run 'cargo check --all-targets --features duckdb-1-5-3'
run 'cargo check --all-targets --features duckdb-1-5-4'
run 'cargo check --all-targets --features bundled-test'
run 'cargo check --all-targets --features bundled-test,duckdb-1-5-4'
# CI's `bundled-test-prebuilt` job links a downloaded libduckdb. Mirror it
# when one is available (DUCKDB_LIB_DIR), else the bundled check above covers
# the same quack-rs sources.
if [[ -n "${DUCKDB_LIB_DIR:-}" ]]; then
    run 'cargo clippy --all-targets --features bundled-test-prebuilt,duckdb-1-5-4 -- -D warnings'
else
    echo 'bundled-test-prebuilt: DUCKDB_LIB_DIR not set — SKIPPED'
fi

echo '── 32-bit target (const-eval and pointer-width differences) ─────────────'
if rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-emscripten; then
    run 'cargo check --lib --target wasm32-unknown-emscripten'
    run 'cargo check --lib --features duckdb-1-5-4 --target wasm32-unknown-emscripten'
else
    echo 'wasm32-unknown-emscripten not installed — SKIPPED'
    echo '  rustup target add wasm32-unknown-emscripten'
fi

echo '── docs ─────────────────────────────────────────────────────────────────'
run 'cargo doc --no-deps --features duckdb-1-5-4'

if (( run_tests )); then
    echo '── test suites ──────────────────────────────────────────────────────────'
    run 'cargo test --all-targets'
    run 'cargo test --all-targets --features duckdb-1-5'
    run 'cargo test --all-targets --features duckdb-1-5-3'
    run 'cargo test --all-targets --features duckdb-1-5-4'
    run 'cargo test --all-targets --features bundled-test'
    if [[ -n "${DUCKDB_LIB_DIR:-}" ]]; then
        run 'cargo test --all-targets --features bundled-test-prebuilt,duckdb-1-5-4'
    fi
    run 'cargo test --doc --features duckdb-1-5-4'
else
    echo
    echo 'Test suites skipped. Re-run with --tests to include them.'
fi

echo '── repository consistency (offline) ─────────────────────────────────────'
run 'python3 scripts/check-workflow-expressions.py'
run 'python3 scripts/sync-book-changelog.py --check'
run 'python3 scripts/generate-sitemap.py --check'

echo '── upstream drift guards ────────────────────────────────────────────────'
for guard in abi-table platform-table spdx-list msrv-vs-duckdb-ci; do
    printf '%-70s ' "scripts/check-$guard.py"
    if out=$(python3 "scripts/check-$guard.py" 2>&1); then
        echo 'ok'
    elif [[ $? -eq 2 ]]; then
        echo 'SKIPPED (upstream unreachable)'
    else
        echo 'FAILED'
        printf '%s\n' "$out" | tail -12
        failures=$((failures + 1))
    fi
done

echo
if (( failures )); then
    echo "$failures check(s) failed."
    exit 1
fi
echo 'All checks passed.'
