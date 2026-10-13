#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
"""Check that ci.yml's `ci-ok` job waits on every blocking job.

Branch protection should require one check, `CI passed` (the `ci-ok` job),
rather than the 37-odd job and matrix names, which drift out of the required
list whenever a job is added or renamed. That only works if `ci-ok` itself
`needs:` every blocking job; this script fails when one is missing, or when an
informational job (`continue-on-error: true`) is listed and could block.

Usage:
    scripts/check-ci-aggregate.py          # exit 1 on a mismatch
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

CI = Path(__file__).resolve().parent.parent / ".github" / "workflows" / "ci.yml"
AGGREGATE = "ci-ok"


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
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            current = jobs.setdefault(m.group(1), {"needs": []})
            in_needs = False
            continue
        if current is None:
            continue
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


def main() -> int:
    jobs = parse_jobs(CI.read_text(encoding="utf-8"))
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
    for job in sorted(needs & informational):
        print(f"error: informational job `{job}` is in `{AGGREGATE}`.needs")
        failed = True
    for job in sorted(needs - set(jobs)):
        print(f"error: `{AGGREGATE}`.needs names `{job}`, which is not a job")
        failed = True
    if not failed:
        print(f"`{AGGREGATE}` waits on all {len(blocking)} blocking jobs")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
