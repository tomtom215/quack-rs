#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Downloads and extracts a DuckDB release's linux-amd64 CLI or shared library.
#
#   scripts/fetch-duckdb.sh <vX.Y.Z | latest> <cli | lib> <dest-dir>
#
# DuckDB renamed its release assets in v2.0.0, so the name depends on the
# release:
#
#   1.x:   duckdb_cli-linux-amd64.zip       libduckdb-linux-amd64.zip
#   2.0+:  duckdb-cli-linux-amd64.tar.gz    duckdb-shared-libs-linux-amd64.tar.gz
#
# The 1.x names 404 for v2.0.0, and `releases/latest/download/<1.x name>` has
# 404ed since 2.0.0 became the latest release. `latest` is therefore resolved
# to its tag first, and the tag is printed, so a job's log says which DuckDB
# it tested.
#
# The 2.0 shared-libs archive holds libduckdb.so and the C headers only (no
# duckdb.hpp, no static library); quack-rs's test shim builds from
# duckdb_extension.h, which both archive formats include from 1.5 on.
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <vX.Y.Z|latest> <cli|lib> <dest-dir>" >&2
  exit 2
fi
tag=$1 kind=$2 dest=$3
requested=$tag
repo=https://github.com/duckdb/duckdb
sums="$(cd "$(dirname "$0")" && pwd)/duckdb-sha256.txt"

if [ "$tag" = "latest" ]; then
  # `releases/latest/download/<asset>` redirects to
  # `releases/download/vX.Y.Z/<asset>` when the latest release has that asset.
  # Read the tag from the redirect rather than from the `releases/latest` page,
  # which some proxies refuse. Try the 2.x name first, then the 1.x one.
  tag=
  for probe in duckdb-cli-linux-amd64.tar.gz duckdb_cli-linux-amd64.zip; do
    url=$(curl -sS --retry 5 --retry-connrefused --connect-timeout 20 --max-time 120 -o /dev/null -w '%{redirect_url}' "$repo/releases/latest/download/$probe")
    if [[ "$url" =~ /releases/download/(v[0-9]+\.[0-9]+\.[0-9]+)/ ]]; then
      tag=${BASH_REMATCH[1]}
      break
    fi
  done
  if [ -z "$tag" ]; then
    echo "error: could not resolve the latest DuckDB release" >&2
    exit 1
  fi
fi
if ! [[ "$tag" =~ ^v([0-9]+)\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: '$tag' is not a release tag (vX.Y.Z)" >&2
  exit 2
fi
major=${BASH_REMATCH[1]}

case "$kind:$major" in
  cli:1) asset=duckdb_cli-linux-amd64.zip ;;
  lib:1) asset=libduckdb-linux-amd64.zip ;;
  cli:*) asset=duckdb-cli-linux-amd64.tar.gz ;;
  lib:*) asset=duckdb-shared-libs-linux-amd64.tar.gz ;;
  *) echo "error: kind must be cli or lib, got '$kind'" >&2; exit 2 ;;
esac

mkdir -p "$dest"
archive="$dest/$asset"
# `--retry` backs off and retries a transient failure: a timeout, a refused
# connection (`--retry-connrefused`), or HTTP 408, 429, 500, 502, 503 or 504.
# The scheduled run of 2026-11-09 lost four jobs to a 504 from this URL; a 404
# (a wrong asset name) is not transient and still fails at once. The time limits
# turn a stalled transfer into a retry instead of a hang until the job's timeout.
curl -fsSL --retry 5 --retry-connrefused --connect-timeout 20 --max-time 600 \
  "$repo/releases/download/$tag/$asset" -o "$archive"

# Verify against scripts/duckdb-sha256.txt before anything is unpacked. ci.yml
# is also release.yml's gate, so a replaced library or CLI must not be able to
# make that gate pass or fail falsely. A pinned tag without a line fails: add
# one (the table says how). `latest` is the canary that a new DuckDB release
# exists, so an unlisted latest tag is reported and used unverified.
actual=$(sha256sum "$archive" | cut -d' ' -f1)
expected=$(awk -v t="$tag" -v a="$asset" '$1 == t && $2 == a { print $3 }' "$sums")
if [ -n "$expected" ]; then
  if [ "$actual" != "$expected" ]; then
    rm -f "$archive"
    echo "error: DuckDB $tag $asset has sha256 $actual, but $sums lists $expected" >&2
    exit 1
  fi
elif [ "$requested" = "latest" ]; then
  echo "warning: DuckDB $tag (latest) is not in $sums, so $asset is unverified (sha256 $actual)" >&2
else
  rm -f "$archive"
  echo "error: DuckDB $tag $asset has no sha256 in $sums; add one (its header says how)" >&2
  exit 1
fi
case "$asset" in
  *.zip) unzip -o -q "$archive" -d "$dest" ;;
  *.tar.gz) tar -xzf "$archive" -C "$dest" ;;
esac
rm -f "$archive"
if [ "$kind" = "cli" ]; then
  chmod +x "$dest/duckdb"
fi
echo "fetched DuckDB $tag $kind ($asset, sha256 ${actual}) into $dest" >&2
echo "$tag"
