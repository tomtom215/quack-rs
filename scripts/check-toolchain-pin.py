#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
"""Check that every workflow builds with the Rust toolchain rust-toolchain.toml pins.

CI used to install `stable` everywhere, so a new Rust release (every six weeks)
changed the compiler, rustfmt and clippy under every job at once, with no change
in this repository: 1.99.0's `clippy::assert_is_empty` turned a release pull
request red (AUDIT.md 12.7), and `RUSTFLAGS: -D warnings` makes any new rustc
warning fatal in all of them. The toolchain is now pinned in one place,
rust-toolchain.toml, and moved by a pull request; the informational
`latest-stable`, `clippy-beta` and `nightly` jobs show what the next releases
will report.

Every toolchain a workflow names is checked, wherever it is named: a
`toolchain:` input, `cargo +X` / `rustc +X` / `rustdoc +X`, `rustup run X`, or a
`RUSTUP_TOOLCHAIN` value. Each must be

* the pin (rust-toolchain.toml's `channel`, which must be an exact `X.Y.Z`);
* Cargo.toml's `rust-version` (the MSRV job);
* `nightly` or a dated `nightly-YYYY-MM-DD` (Miri, the sanitizers and the
  fuzzer need one);
* `stable` or `beta`, only inside an informational job (job-level
  `continue-on-error: true`, which scripts/check-ci-aggregate.py ties to its
  INFORMATIONAL set), where seeing the next release is the point;
* an expression (`${{ ... }}`) or a shell variable (`"+$msrv"`), which cannot
  be checked here, only in a (workflow, job) listed in DYNAMIC below.

Lines that are comments (YAML or shell) are not read.

Usage:
    python3 scripts/check-toolchain-pin.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"
EXACT = re.compile(r"^\d+\.\d+\.\d+$")
NIGHTLY = re.compile(r"^nightly(?:-\d{4}-\d{2}-\d{2})?$")

# Toolchains that only exist at run time, each a deliberate choice:
DYNAMIC = {
    # The rustc the `package` job recorded; the step after it checks they agree.
    ("release.yml", "publish"),
    # The generated project's own `rust-version`, read from its Cargo.toml.
    ("ci.yml", "scaffold-e2e"),
}

JOB_ID = re.compile(r"^  ([A-Za-z0-9_-]+):[ \t]*(?:[&#].*)?$")
REFERENCES = [
    # The `toolchain:` input of dtolnay/rust-toolchain.
    re.compile(r"^\s*(?:-\s+)?toolchain:\s*(?P<value>.+?)\s*(?:\s#.*)?$"),
    # `cargo +X`, `rustc +X`, `rustdoc +X` (also `cargo "+$msrv"`).
    re.compile(r"\b(?:cargo|rustc|rustdoc)\s+[\"']?\+(?P<value>[^\s\"']+)"),
    re.compile(r"\brustup\s+run\s+[\"']?(?P<value>[^\s\"']+)"),
    # An env key, or a shell assignment.
    re.compile(r"\bRUSTUP_TOOLCHAIN(?::\s*|=)[\"']?(?P<value>[^\s\"']+)"),
]


def jobs_of(text: str) -> list[tuple[str, bool, list[tuple[int, str]]]]:
    """(job id, informational, [(line number, line)]) for each job; lines before
    `jobs:` belong to a pseudo-job "(workflow)", which is never informational."""
    jobs: list[tuple[str, bool, list[tuple[int, str]]]] = [("(workflow)", False, [])]
    in_jobs = False
    for number, line in enumerate(text.splitlines(), 1):
        if line.startswith("jobs:"):
            in_jobs = True
            continue
        if in_jobs and line and not line.startswith((" ", "#")):
            in_jobs = False
        m = JOB_ID.match(line) if in_jobs else None
        if m:
            jobs.append((m.group(1), False, []))
            continue
        job, _, lines = jobs[-1]
        if in_jobs and re.match(r"^    continue-on-error:\s*true\b", line):
            jobs[-1] = (job, True, lines)
        lines.append((number, line))
    return jobs


def main() -> int:
    pin = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"]
    msrv = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["rust-version"]
    problems: list[str] = []
    if not EXACT.match(pin):
        problems.append(f"rust-toolchain.toml channel is {pin!r}, not an exact X.Y.Z release")
    counts = {"pin": 0, "msrv": 0, "nightly": 0, "informational": 0, "dynamic": 0}
    for path in sorted(WORKFLOWS.glob("*.yml")):
        for job, informational, lines in jobs_of(path.read_text(encoding="utf-8")):
            for number, line in lines:
                if line.lstrip().startswith("#"):
                    continue
                for pattern in REFERENCES:
                    for m in pattern.finditer(line):
                        value = m.group("value").strip("\"'")
                        where = f"{path.name}:{number} (job `{job}`)"
                        if value == pin:
                            counts["pin"] += 1
                        elif value == msrv:
                            counts["msrv"] += 1
                        elif NIGHTLY.match(value):
                            counts["nightly"] += 1
                        elif value in {"stable", "beta"}:
                            if informational:
                                counts["informational"] += 1
                            else:
                                problems.append(
                                    f"{where}: `{value}` in a blocking job; use {pin} (rust-toolchain.toml), "
                                    "or make the job informational"
                                )
                        elif value.startswith("$"):
                            if (path.name, job) in DYNAMIC:
                                counts["dynamic"] += 1
                            else:
                                problems.append(
                                    f"{where}: toolchain `{value}` is only known at run time; "
                                    "list the job in DYNAMIC if that is deliberate"
                                )
                        else:
                            problems.append(f"{where}: toolchain `{value}`; expected {pin} (rust-toolchain.toml)")
    for problem in problems:
        print(f"error: {problem}")
    if problems:
        return 1
    print(
        f"every workflow toolchain agrees with rust-toolchain.toml ({pin}): "
        + ", ".join(f"{n} {kind}" for kind, n in counts.items())
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
