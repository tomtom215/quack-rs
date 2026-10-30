#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
"""Check that ci.yml's `ci-ok` job waits on every blocking job.

Branch protection should require one check, `CI passed` (the `ci-ok` job),
rather than the 37-odd job and matrix names, which drift out of the required
list whenever a job is added or renamed. That only works if `ci-ok` itself
`needs:` every blocking job; this script fails when one is missing, or when an
informational job (`continue-on-error: true`) is listed and could block. It
also checks the two properties that make the aggregate safe: it must run
`if: always()` (otherwise a failed dependency makes it *skipped*, and a skipped
required check counts as passing), and its step must compare every result to
`success`.

More holes are closed, each of which let a blocking job stop blocking without
this check noticing:

* the set of informational jobs is fixed in INFORMATIONAL below. A job that gains
  `continue-on-error: true` and drops out of `needs:` is no longer silently
  demoted: the script fails until INFORMATIONAL names it, so it takes a
  deliberate edit here, visible in review;
* a step-level `continue-on-error: true` in a blocking job, or in `ci-ok` itself
  (the job stays green while the step fails), must be listed in
  INFORMATIONAL_STEPS;
* a job id written with a trailing comment or a YAML anchor (`  fuzz: # smoke`)
  is still a job. It used to be invisible to the parser;
* the safety patterns are matched against `ci-ok` with its comments removed, so
  a pattern kept in a comment after the code that enforced it went does not
  count;
* the quality-gate table in .github/workflows/README.md must list exactly
  ci.yml's jobs, each with the blocking / informational kind ci.yml gives it.

Usage:
    scripts/check-ci-aggregate.py          # exit 1 on a mismatch
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

CI = Path(__file__).resolve().parent.parent / ".github" / "workflows" / "ci.yml"
README = CI.parent / "README.md"
AGGREGATE = "ci-ok"

# The jobs that are allowed to be informational (`continue-on-error: true`, and
# outside `ci-ok`'s needs). Adding one is a decision; make it here.
INFORMATIONAL = {"clippy-beta", "latest-stable", "nightly", "test-bundled-duckdb-2-0"}

# (job, step name) pairs allowed to carry a step-level `continue-on-error`.
INFORMATIONAL_STEPS = {
    ("semver", "List public API changes since origin/main (informational)"),
}

JOB_ID = re.compile(r"^  ([A-Za-z0-9_-]+):[ \t]*(?:[&#].*)?$")
# A step's own keys: on its `- ` line, or indented eight spaces under it.
STEP_CONTINUE_ON_ERROR = re.compile(r"^(?:      - |        )continue-on-error:\s*(?!false\b)\S")
STEP_NAME = re.compile(r"^(?:      - |        )name:\s*(.+?)\s*$")
# A row of the README's quality-gate table: | `job-id` | Name | blocking... / informational |
README_ROW = re.compile(r"^\| `([A-Za-z0-9_-]+)` \|.*\| (blocking[^|]*?|informational) \|\s*$", re.MULTILINE)


def parse_jobs(text: str) -> dict[str, dict]:
    """The `jobs:` map of a workflow, reduced to what this check reads.

    Standard library only (no PyYAML on every runner): ci.yml indents job ids
    by two spaces under `jobs:` and job keys by four, which is all this needs.
    """
    jobs: dict[str, dict] = {}
    current: dict | None = None
    in_jobs = in_needs = False
    for line in text.splitlines():
        if line.startswith("jobs:"):
            in_jobs = True
            continue
        if not in_jobs:
            continue
        if line and not line.startswith(" ") and not line.startswith("#"):
            break
        m = JOB_ID.match(line)
        if m:
            current = jobs.setdefault(m.group(1), {"needs": [], "lines": []})
            in_needs = False
            continue
        if current is None:
            continue
        current["lines"].append(line)
        if re.match(r"^    continue-on-error:\s*true\b", line):
            current["continue-on-error"] = True
        m = re.match(r"^    needs:\s*(.*)$", line)
        if m:
            rest = m.group(1).strip()
            if rest.startswith("["):
                current["needs"] = [s.strip() for s in rest.strip("[]").split(",") if s.strip()]
            elif rest:
                current["needs"] = [rest]
            in_needs = not rest
            continue
        if in_needs:
            m = re.match(r"^      - ([A-Za-z0-9_-]+)\s*$", line)
            if m:
                current["needs"].append(m.group(1))
            elif line.strip() and not line.lstrip().startswith("#"):
                in_needs = False
    return jobs


def aggregate_block(text: str) -> str:
    """The text of the `ci-ok` job: from its id to the next job id or EOF."""
    start = re.search(rf"^  {AGGREGATE}:[ \t]*(?:[&#].*)?$", text, re.MULTILINE)
    if start is None:
        return ""
    rest = text[start.end() :]
    nxt = re.search(r"^  [A-Za-z0-9_-]+:[ \t]*(?:[&#].*)?$", rest, re.MULTILINE)
    block = rest[: nxt.start()] if nxt else rest
    # Comments do not count: a pattern kept in a comment while the code that
    # enforced it is gone must not satisfy the check below.
    return "\n".join(re.sub(r"(^|\s)#.*$", "", line) for line in block.splitlines())


def safety_errors(block: str) -> list[str]:
    """What is wrong with the aggregate job's own safety properties."""
    errors = []
    if not re.search(r"^    if:\s*always\(\)\s*$", block, re.MULTILINE):
        errors.append(
            f"`{AGGREGATE}` must have `if: always()`: without it a failed dependency "
            "skips the job, and a skipped required check counts as passing"
        )
    if not re.search(r'\[\s*"result"\s*\]\s*!=\s*"success"', block):
        errors.append(
            f"`{AGGREGATE}`'s step must fail on any result other than "
            '`"success"` (skipped and cancelled included)'
        )
    return errors


def step_blocks(lines: list[str]) -> list[list[str]]:
    """A job's lines split into its steps (each starts with `      - `)."""
    blocks: list[list[str]] = []
    for line in lines:
        if line.startswith("      - "):
            blocks.append([])
        if blocks:
            blocks[-1].append(line)
    return blocks


def step_errors(job: str, lines: list[str]) -> list[str]:
    """Steps of `job` that can fail without failing it."""
    errors = []
    for block in step_blocks(lines):
        if not any(STEP_CONTINUE_ON_ERROR.match(line) for line in block):
            continue
        name = next(
            (m.group(1).strip().strip("\"'") for line in block if (m := STEP_NAME.match(line))),
            "(unnamed)",
        )
        if (job, name) not in INFORMATIONAL_STEPS:
            errors.append(
                f"step `{name}` of blocking job `{job}` has `continue-on-error`: the job "
                "stays green when it fails (list it in INFORMATIONAL_STEPS if that is intended)"
            )
    return errors


def readme_errors(jobs: dict[str, dict]) -> list[str]:
    """The README's quality-gate table must list exactly ci.yml's jobs, with the
    blocking / informational split ci.yml declares (it is documented as
    generated, and nothing regenerated it)."""
    rows = {m.group(1): m.group(2) for m in README_ROW.finditer(README.read_text(encoding="utf-8"))}
    errors = []
    for job in sorted(set(jobs) - set(rows)):
        errors.append(f"{README.name}: job `{job}` is missing from the quality-gate table")
    for job in sorted(set(rows) - set(jobs)):
        errors.append(f"{README.name}: the table lists `{job}`, which is not a job in {CI.name}")
    for job in sorted(set(rows) & set(jobs)):
        kind = "informational" if jobs[job].get("continue-on-error") else "blocking"
        if not rows[job].startswith(kind):
            errors.append(f"{README.name}: `{job}` is `{rows[job]}` in the table, `{kind}` in {CI.name}")
    return errors


def main() -> int:
    text = CI.read_text(encoding="utf-8")
    jobs = parse_jobs(text)
    if AGGREGATE not in jobs:
        print(f"error: {CI.name} has no `{AGGREGATE}` job")
        return 1
    needs = set(jobs[AGGREGATE]["needs"])
    blocking = {k for k, j in jobs.items() if k != AGGREGATE and not j.get("continue-on-error")}
    informational = {k for k, j in jobs.items() if j.get("continue-on-error")}
    failed = False
    for job in sorted(blocking - needs):
        print(f"error: blocking job `{job}` is not in `{AGGREGATE}`.needs")
        failed = True
    for job in sorted(informational - INFORMATIONAL):
        print(f"error: job `{job}` has `continue-on-error: true` but is not in INFORMATIONAL; "
              "a blocking job must not be demoted by an edit to ci.yml alone")
        failed = True
    for job in sorted(INFORMATIONAL - informational):
        print(f"error: INFORMATIONAL names `{job}`, which is not (or no longer) `continue-on-error`")
        failed = True
    # `ci-ok` itself included: a `continue-on-error` on its own step would make
    # the one required check pass whatever its dependencies did.
    for job in sorted(set(jobs) - informational):
        for message in step_errors(job, jobs[job]["lines"]):
            print(f"error: {message}")
            failed = True
    for job in sorted(needs & informational):
        print(f"error: informational job `{job}` is in `{AGGREGATE}`.needs")
        failed = True
    for job in sorted(needs - set(jobs)):
        print(f"error: `{AGGREGATE}`.needs names `{job}`, which is not a job")
        failed = True
    for message in readme_errors(jobs):
        print(f"error: {message}")
        failed = True
    for message in safety_errors(aggregate_block(text)):
        print(f"error: {message}")
        failed = True
    if not failed:
        print(f"`{AGGREGATE}` waits on all {len(blocking)} blocking jobs")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
