#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Runs one of the guard scripts that compare a checked-in table with upstream
# (check-abi-table.py, check-platform-table.py, ...), mapping their documented
# exit codes onto a job result:
#
#   0  the table matches upstream            -> pass
#   1  it has drifted                        -> fail
#   2  upstream could not be reached/parsed  -> pass with a warning, UNLESS
#      QUACK_STRICT_UPSTREAM=1 (set by ci.yml on a tag, i.e. for a release),
#      where "could not check" must not read as "checked": fail, and re-run.
#
# It also refuses a guard script that does not exist. Python itself exits 2 for
# "can't open file", which the inline `|| status=$?; [ $status -eq 2 ]` wrapper
# this replaces read as "upstream unreachable": renaming or deleting a guard
# left its job green, saying only "check skipped".
#
# Usage: scripts/upstream-guard.sh scripts/check-abi-table.py [args...]
#        scripts/upstream-guard.sh --self-test   # the mapping above, on stub scripts
set -uo pipefail

if [ $# -lt 1 ]; then
  echo "usage: $0 <guard-script.py> [args...] | --self-test" >&2
  exit 64
fi

# The mapping is what makes these jobs mean anything, so it is tested on every
# run (ci.yml's `doc` job): stubs that exit 0..3, a missing script, strict and not.
if [ "$1" = "--self-test" ]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  for code in 0 1 2 3; do
    printf 'import sys\nsys.exit(%s)\n' "$code" > "$tmp/exit$code.py"
  done
  failed=0
  cases=0
  expect() { # <expected status> <QUACK_STRICT_UPSTREAM value> <script>
    local got
    cases=$((cases + 1))
    # Output discarded: its ::warning:: / ::error:: lines would become annotations.
    QUACK_STRICT_UPSTREAM=$2 bash "${BASH_SOURCE[0]}" "$3" > /dev/null 2>&1
    got=$?
    if [ "$got" -ne "$1" ]; then
      echo "self-test: '$3' with QUACK_STRICT_UPSTREAM='$2' exited $got, expected $1"
      failed=1
    fi
  }
  expect 0 "" "$tmp/exit0.py"
  expect 1 "" "$tmp/exit1.py"
  expect 0 "" "$tmp/exit2.py"
  expect 3 "" "$tmp/exit3.py"
  expect 0 1 "$tmp/exit0.py"
  expect 1 1 "$tmp/exit1.py"
  expect 1 1 "$tmp/exit2.py"
  expect 1 "" "$tmp/does-not-exist.py"
  expect 1 1 "$tmp/does-not-exist.py"
  if [ "$failed" -eq 0 ]; then
    echo "upstream-guard self-test: $cases cases ok"
  fi
  exit "$failed"
fi
script=$1
shift
if [ ! -f "$script" ]; then
  echo "::error::guard script '$script' does not exist (renamed or deleted?); this job would check nothing"
  exit 1
fi

python3 "$script" "$@"
status=$?
if [ "$status" -eq 2 ]; then
  if [ "${QUACK_STRICT_UPSTREAM:-}" = "1" ]; then
    echo "::error::$script could not check upstream (exit 2), and a release must not skip it. Re-run this job when upstream is reachable."
    exit 1
  fi
  echo "::warning::$script: upstream unreachable; check skipped, not failed"
  exit 0
fi
exit "$status"
