#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Prints the DuckDB release tag (e.g. `v1.5.5`) that the pinned `libduckdb-sys`
# in Cargo.lock corresponds to, so CI never hard-codes it in two places and
# drifts. Usage: `scripts/duckdb-version-from-lock.sh [path/to/Cargo.lock]`
#
# libduckdb-sys encodes the DuckDB version in its middle component from 1.5.0
# onward: 1.10504.0 -> DuckDB 1.5.4, 1.10505.0 -> 1.5.5. Decoding `N`:
#   major = N / 10000, minor = (N / 100) % 100, patch = N % 100
# Releases on the older 1.4.x line (e.g. 1.4.4, 1.4.5) carry the DuckDB version
# directly and are passed through unchanged.
set -euo pipefail

lock="${1:-Cargo.lock}"
ver=$(awk '/^name = "libduckdb-sys"$/ { getline; sub(/^version = "/, ""); sub(/"$/, ""); print; exit }' "$lock")

if [ -z "$ver" ]; then
  echo "libduckdb-sys not found in $lock" >&2
  exit 1
fi

IFS='.' read -r a b _c <<<"$ver"

if [ "$a" = "1" ] && [ "$b" -ge 10000 ]; then
  printf 'v%d.%d.%d\n' "$((b / 10000))" "$(((b / 100) % 100))" "$((b % 100))"
else
  # Old scheme: the crate version IS the DuckDB version.
  printf 'v%s\n' "$ver"
fi
