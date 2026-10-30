#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
"""Check that everything a release bumps agrees with Cargo.toml's version.

A release bumps one version in about twenty places: Cargo.toml, the committed
lockfiles (Cargo.lock and those of examples/hello-ext, fuzz, book/doctest and,
once committed, benches/engine, which all record quack-rs itself), the
CHANGELOG heading and its two link definitions, CITATION.cff, SECURITY.md's
supported-versions table, README.md's "The latest is ..." line, and every
`quack-rs = "0.X"` snippet in the README, the book and the rustdoc.
RELEASING.md Step 3b used to tell the maintainer to `git grep` for the old
version. A place missed was found after the tag, in the published crate, where
it cannot be corrected: a stale Cargo.lock fails `cargo package --locked` in
release.yml only after the whole of ci.yml has run, and a stale snippet ships.

This check is dependency-free and runs in two places: ci.yml's `doc` job (so the
release PR cannot merge with a place missed) and release.yml's `validate` job
(so a tag on a commit that slipped through fails in a minute, not after two
hours of CI).

For a pre-release version (`0.20.0-rc.1`) only the lockfile and CHANGELOG
checks apply: the docs keep naming the latest stable release.

Exit codes: 0 consistent, 1 a mismatch (listed), 2 a file could not be read.

Usage:
    python3 scripts/check-release-metadata.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parent.parent

# (lockfile, must exist). benches/engine's lockfile is checked once it is committed.
LOCKFILES = [
    ("Cargo.lock", True),
    ("examples/hello-ext/Cargo.lock", True),
    ("fuzz/Cargo.lock", True),
    ("book/doctest/Cargo.lock", True),
    ("benches/engine/Cargo.lock", False),
]

# Where a "quack-rs = ..." dependency snippet is something a reader copies.
SNIPPET_GLOBS = ["README.md", "Cargo.toml", "src/**/*.rs", "book/src/**/*.md", "examples/**/README.md"]
# Pages that name an old version on purpose.
SNIPPET_SKIP = re.compile(r"(?:^|/)(?:changelog\.md|upgrading-[^/]*\.md)$")
SNIPPET = re.compile(
    r"""quack-rs\s*=\s*(?:"(?P<plain>\d+\.\d+)(?:\.\d+)?"|\{[^}\n]*?version\s*=\s*"(?P<table>\d+\.\d+)(?:\.\d+)?")"""
)


def read(path: str) -> str:
    try:
        return (ROOT / path).read_text(encoding="utf-8")
    except OSError as err:
        print(f"error: cannot read {path}: {err}", file=sys.stderr)
        raise SystemExit(2) from err


def main() -> int:
    version = tomllib.loads(read("Cargo.toml"))["package"]["version"]
    major, minor, *_ = version.split("-")[0].split(".")
    line = f"{major}.{minor}"
    prerelease = "-" in version
    problems: list[str] = []

    # Every committed lockfile records quack-rs itself (the root package, or the
    # path dependency of the example and CI crates). `cargo package --locked`
    # refuses a stale root Cargo.lock; the others go stale the same way and are
    # bumped in the same commit (RELEASING.md Step 3).
    for path, required in LOCKFILES:
        if not required and not (ROOT / path).is_file():
            continue
        lock = tomllib.loads(read(path))
        locked = [p["version"] for p in lock.get("package", []) if p["name"] == "quack-rs" and "source" not in p]
        if locked != [version]:
            directory = str(Path(path).parent)
            why = (
                "release.yml packages with --locked" if directory == "."
                else "the release commit updates every lockfile, RELEASING.md Step 3"
            )
            problems.append(
                f"{path} has quack-rs {locked}, Cargo.toml says {version}: run `cargo update --workspace` "
                f"in {'the repository root' if directory == '.' else directory} and commit {path} ({why})"
            )

    # CHANGELOG: a dated section, and the two link definitions that go with it.
    changelog = read("CHANGELOG.md")
    heading = re.search(rf"^## \[{re.escape(version)}\] - (\d{{4}}-\d{{2}}-\d{{2}})\s*$", changelog, re.MULTILINE)
    if not heading:
        problems.append(f"CHANGELOG.md has no `## [{version}] - YYYY-MM-DD` section")
    if not re.search(rf"^\[{re.escape(version)}\]: \S+", changelog, re.MULTILINE):
        problems.append(f"CHANGELOG.md has no `[{version}]: ...` link definition at the bottom")
    unreleased = re.search(r"^\[Unreleased\]: \S+/compare/(\S+?)\.\.\.HEAD\s*$", changelog, re.MULTILINE)
    if not unreleased or unreleased.group(1) != f"v{version}":
        problems.append(
            f"CHANGELOG.md's `[Unreleased]:` link should compare `v{version}...HEAD`, "
            f"it compares `{unreleased.group(1) if unreleased else '(missing)'}...HEAD`"
        )

    if not prerelease:
        date = heading.group(1) if heading else None
        citation = read("CITATION.cff")
        m = re.search(r"^version:\s*(\S+)\s*$", citation, re.MULTILINE)
        if not m or m.group(1) != version:
            problems.append(f"CITATION.cff version is {m.group(1) if m else '(missing)'}, expected {version}")
        m = re.search(r"^date-released:\s*(\S+)\s*$", citation, re.MULTILINE)
        if date and (not m or m.group(1) != date):
            problems.append(f"CITATION.cff date-released is {m.group(1) if m else '(missing)'}, CHANGELOG says {date}")

        if not re.search(rf"^\|\s*{re.escape(line)}\.x\s*\|\s*Yes\b", read("SECURITY.md"), re.MULTILINE):
            problems.append(f"SECURITY.md's supported-versions table has no `| {line}.x | Yes` row")

        m = re.search(r"The latest is (\d+\.\d+\.\d+) \((\d{4}-\d{2}-\d{2})\)", read("README.md"))
        if not m or m.group(1) != version or (date and m.group(2) != date):
            problems.append(
                f"README.md's \"The latest is ...\" line says {m.groups() if m else '(missing)'}, "
                f"expected ({version!r}, {date!r})"
            )

        for pattern in SNIPPET_GLOBS:
            for path in sorted(ROOT.glob(pattern)):
                rel = path.relative_to(ROOT).as_posix()
                if SNIPPET_SKIP.search(rel):
                    continue
                for number, text in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                    for m in SNIPPET.finditer(text):
                        seen = m.group("plain") or m.group("table")
                        if seen != line:
                            problems.append(f"{rel}:{number}: snippet names quack-rs {seen}, expected {line}")

    for problem in problems:
        print(f"error: {problem}")
    if problems:
        print(f"\n{len(problems)} place(s) disagree with Cargo.toml's version {version}.")
        return 1
    print(f"release metadata agrees with Cargo.toml's version {version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
