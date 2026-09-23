#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Summarises a cargo-mutants run and decides whether the mutation gate passes.
#
# Both jobs in .github/workflows/mutants.yml call this, so the gate logic lives
# in one place and can be exercised locally against a real `mutants.out/`.
#
# Usage:
#   MUTANTS_EXIT=<cargo-mutants exit code> scripts/mutants-report.sh <full|incremental> [mutants.out]
#
# The gate FAILS when any of these holds:
#
#   - MUTANTS_EXIT is unset/empty (the run step never ran or never reported);
#   - cargo-mutants exited with anything other than 0 (all caught), 2 (some
#     missed) or 3 (some timed out). Those three are the only codes that mean
#     "the mutants were actually tested"; 1 is a usage error, 4 means the test
#     suite already fails in the UNMUTATED tree (so nothing was tested at all),
#     5/6 are `--in-diff` errors and 70 is an internal error. See
#     cargo-mutants' src/exit_code.rs.
#   - mutants were generated but none was tested (caught + missed + timeout +
#     unviable == 0), or -- in `full` mode -- no mutants were generated at all:
#     a full sweep of this crate that finds nothing to mutate is broken, not
#     clean;
#   - any mutant was missed or timed out.
#
# Before this script existed the report steps read only the outcome files and
# ignored the exit code, so a run whose baseline failed wrote four empty files,
# scored "100%" and passed.
#
# Writes a Markdown summary to $GITHUB_STEP_SUMMARY when set, else to stdout.

set -euo pipefail

MODE="${1:-}"
OUT="${2:-mutants.out}"
case "$MODE" in
  full | incremental) ;;
  *)
    echo "usage: MUTANTS_EXIT=<code> $0 <full|incremental> [mutants.out]" >&2
    exit 64
    ;;
esac

SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/stdout}"

# Non-empty lines in a file, 0 if it is missing. `grep -c` prints the count
# (including 0) itself; it merely *exits* 1 on zero matches, so the fallback
# must not print a second number -- `grep -c . f || echo 0` yields "0\n0".
count() {
  local n=0
  if [ -f "$1" ]; then
    n=$(grep -c . "$1" || true)
  fi
  echo "${n:-0}"
}

# Number of mutants cargo-mutants generated (listed in mutants.json), or -1
# when the file is missing/unreadable, which is itself a failure below.
generated() {
  if [ -f "$OUT/mutants.json" ]; then
    jq 'length' "$OUT/mutants.json" 2>/dev/null || echo -1
  else
    echo -1
  fi
}

CAUGHT=$(count "$OUT/caught.txt")
MISSED=$(count "$OUT/missed.txt")
UNVIABLE=$(count "$OUT/unviable.txt")
TIMEOUT=$(count "$OUT/timeout.txt")
GENERATED=$(generated)
TESTED=$((CAUGHT + MISSED + TIMEOUT + UNVIABLE))
SCORED=$((CAUGHT + MISSED + TIMEOUT))
EXIT_CODE="${MUTANTS_EXIT:-}"

FAILURES=()

case "$EXIT_CODE" in
  0 | 2 | 3) ;;
  "") FAILURES+=("cargo-mutants did not report an exit code (did the run step execute?)") ;;
  4) FAILURES+=("cargo-mutants exit 4: the tests FAIL in the unmutated tree, so no mutant was tested") ;;
  *) FAILURES+=("cargo-mutants exited ${EXIT_CODE} (usage/internal error): no trustworthy result") ;;
esac

if [ "$GENERATED" -lt 0 ]; then
  FAILURES+=("$OUT/mutants.json is missing or unreadable: cannot tell what was generated")
elif [ "$GENERATED" -eq 0 ] && [ "$MODE" = full ]; then
  FAILURES+=("a full sweep generated 0 mutants")
elif [ "$GENERATED" -gt 0 ] && [ "$TESTED" -eq 0 ]; then
  FAILURES+=("${GENERATED} mutant(s) were generated but none was tested")
fi

[ "$MISSED" -gt 0 ] && FAILURES+=("${MISSED} mutant(s) survived -- add tests to cover these gaps")
[ "$TIMEOUT" -gt 0 ] && FAILURES+=("${TIMEOUT} mutant(s) timed out -- a hang is not a caught mutant")

# Timeouts count against the score: they are not evidence the tests noticed.
if [ "$SCORED" -gt 0 ]; then
  SCORE="$(((CAUGHT * 100) / SCORED))%"
else
  SCORE="n/a (nothing scored)"
fi

{
  if [ "$MODE" = incremental ]; then
    echo "# Mutation Testing Report (incremental)"
    echo ""
    echo "Only files changed in this PR were mutated."
  else
    echo "# Mutation Testing Report"
  fi
  echo ""
  echo "## Score: ${SCORE}"
  echo ""
  echo "cargo-mutants exit code: \`${EXIT_CODE:-<none>}\`; mutants generated: ${GENERATED}"
  echo ""
  echo "| Outcome | Count | Meaning |"
  echo "|---------|------:|---------|"
  echo "| Caught | ${CAUGHT} | Test suite detected the mutation |"
  echo "| Missed | ${MISSED} | **Test gap** -- a real bug here would go undetected |"
  echo "| Timeout | ${TIMEOUT} | Mutant caused tests to hang (killed after limit) |"
  echo "| Unviable | ${UNVIABLE} | Mutation caused a compile error (not a test gap) |"
  echo ""
  if [ "$MISSED" -gt 0 ]; then
    echo "## Surviving Mutants"
    echo ""
    echo '```'
    cat "$OUT/missed.txt"
    echo '```'
    echo ""
    echo "Reproduce locally: \`cargo install cargo-mutants --locked && cargo mutants --file <path>\`"
    echo ""
  fi
  if [ "$TIMEOUT" -gt 0 ]; then
    echo "## Timed-Out Mutants"
    echo ""
    echo '```'
    cat "$OUT/timeout.txt"
    echo '```'
    echo ""
  fi
  if [ "${#FAILURES[@]}" -gt 0 ]; then
    echo "## Gate: FAILED"
    echo ""
    for f in "${FAILURES[@]}"; do echo "- $f"; done
  elif [ "$GENERATED" -eq 0 ]; then
    echo "No mutants were generated for the changed files (nothing mutable, or all excluded by mutants.toml)."
  else
    echo "All ${TESTED} mutant(s) were caught or unviable. No test gaps found."
  fi
} >>"$SUMMARY"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo " MUTATION SCORE: ${SCORE}   (cargo-mutants exit ${EXIT_CODE:-<none>}, generated ${GENERATED})"
echo " Caught: ${CAUGHT}  Missed: ${MISSED}  Timeout: ${TIMEOUT}  Unviable: ${UNVIABLE}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ "${#FAILURES[@]}" -gt 0 ]; then
  for f in "${FAILURES[@]}"; do echo "::error::$f"; done
  exit 1
fi
