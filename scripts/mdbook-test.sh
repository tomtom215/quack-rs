#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Compile-checks the book's Rust code blocks.
#
# `mdbook test` on its own cannot do this here. It only forwards `-L` to
# rustdoc, and `-L` alone does not put a crate in scope in any edition -- an
# `--extern` flag is required, which mdbook 0.4.x has no way to pass. So a
# plain `mdbook test -L target/debug/deps` fails on every block that says
# `use quack_rs::...`, which is nearly all of them.
#
# The fix is a shim: a `rustdoc` earlier on PATH that appends the `--extern`
# flags and execs the real one. Everything else is stock mdbook.
#
# Usage: scripts/mdbook-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
TARGET="${MDBOOK_TEST_TARGET_DIR:-$ROOT/target/booktest}"

# A dedicated target dir keeps exactly one rlib per crate. The main target dir
# accumulates one per feature combination, and rustdoc cannot choose between
# them ("multiple candidates for `quack_rs` found").
# Build with duckdb-1-5-3, matching [package.metadata.docs.rs]. Much of the book
# documents `duckdb-1-5`-gated API (ScalarFunctionBuilder::varargs / volatile /
# init, the whole duckdb-1-5/ chapter set); with default features those blocks
# fail with "no method named ..." that looks like a documentation bug but is not.
echo "==> building quack-rs into $TARGET"
cargo build --quiet --features duckdb-1-5-3 --target-dir "$TARGET"

DEPS="$TARGET/debug/deps"
find_rlib() {
  local n=$1 hit
  hit=$(find "$DEPS" -maxdepth 1 -name "lib$n-*.rlib" | head -1)
  [ -n "$hit" ] || { echo "error: no rlib for '$n' in $DEPS" >&2; exit 1; }
  printf '%s' "$hit"
}
QUACK=$(find_rlib quack_rs)
SYS=$(find_rlib libduckdb_sys)

SHIM=$(mktemp -d)
trap 'rm -rf "$SHIM"' EXIT
cat > "$SHIM/rustdoc" <<SHIMEOF
#!/usr/bin/env bash
exec "$(command -v rustdoc)" \\
  --extern quack_rs=$QUACK \\
  --extern libduckdb_sys=$SYS \\
  -L $DEPS "\$@"
SHIMEOF
chmod +x "$SHIM/rustdoc"

echo "==> mdbook test"
PATH="$SHIM:$PATH" mdbook test -L "$DEPS"
