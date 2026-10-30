#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
"""Check that each GitHub Action is pinned to one commit wherever it is used.

Every remote `uses:` in .github/workflows/ names its action by full commit SHA,
with the release (or branch) in a trailing comment:
`Swatinem/rust-cache@d1527c8f... # v2.9.3`. Dependabot moves those together.
The scaffold carries its own copies of the same pins in
src/scaffold/templates_ci.rs, for the workflows it generates for an extension,
and Dependabot never sees them: when it bumped this repository's actions
(#145), the generated workflows would have kept the old commits with no test
failing (DEPS-15). This fails when

* a remote `uses:` is not pinned to a 40-hex commit;
* two uses of one action repository name different commits, or comments with
  a different first word (`# v2.9.2` beside `# v2.9.3`);
* a pin in src/scaffold/templates_ci.rs names an action repository this
  repository's workflows use, at another commit or with another version;
* either side yields no pins at all (a layout change must not read as "all
  consistent").

A comment's first word is its version (`v2.9.3`, `master`); anything after it
(a date) is not compared. An action only the scaffold uses (setup-python) is
listed, not compared.

Usage:
    python3 scripts/check-action-pins.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"
TEMPLATES = ROOT / "src" / "scaffold" / "templates_ci.rs"

USES = re.compile(
    r"^\s*(?:-\s+)?uses:\s*(?P<action>[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+)@(?P<ref>[^\s#]+)"
    r"(?:\s+#\s*(?P<comment>.*?))?\s*$"
)
# `const NAME: &str = "owner/repo@sha # comment";`, possibly wrapped after `=`.
TEMPLATE_PIN = re.compile(
    r"const\s+(?P<const>\w+)\s*:\s*&str\s*=\s*\"(?P<action>[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+)@(?P<ref>[^\s\"#]+)"
    r"(?:\s+#\s*(?P<comment>[^\"]*?))?\s*\"\s*;"
)
SHA = re.compile(r"^[0-9a-f]{40}$")


def repo_of(action: str) -> str:
    """`actions/cache/restore` -> `actions/cache`: one repository, one pin."""
    return "/".join(action.split("/")[:2])


def version_of(comment: str | None) -> str:
    return comment.split()[0] if comment and comment.split() else "(no comment)"


def main() -> int:
    problems: list[str] = []
    # repo -> {(sha, version): [where, ...]}
    pins: dict[str, dict[tuple[str, str], list[str]]] = {}
    count = 0
    for path in sorted(WORKFLOWS.glob("*.yml")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.lstrip().startswith("#"):
                continue
            m = USES.match(line)
            if not m:
                continue
            count += 1
            where = f"{path.name}:{number}"
            if not SHA.match(m.group("ref")):
                problems.append(f"{where}: {m.group('action')}@{m.group('ref')} is not pinned to a full commit SHA")
                continue
            key = (m.group("ref"), version_of(m.group("comment")))
            pins.setdefault(repo_of(m.group("action")), {}).setdefault(key, []).append(where)
    if count == 0:
        problems.append(f"no `uses:` found under {WORKFLOWS.relative_to(ROOT)}; has the layout changed?")
    for repo, variants in sorted(pins.items()):
        if len(variants) > 1:
            listed = "; ".join(f"{sha[:12]} # {ver} at {', '.join(w)}" for (sha, ver), w in sorted(variants.items()))
            problems.append(f"{repo} is pinned {len(variants)} ways: {listed}")

    template_text = TEMPLATES.read_text(encoding="utf-8") if TEMPLATES.is_file() else ""
    template_pins = list(TEMPLATE_PIN.finditer(template_text))
    if not template_pins:
        problems.append(f"no action pins found in {TEMPLATES.relative_to(ROOT)}; has the layout changed?")
    compared = []
    for m in template_pins:
        repo, sha, ver = repo_of(m.group("action")), m.group("ref"), version_of(m.group("comment"))
        where = f"{TEMPLATES.relative_to(ROOT)} {m.group('const')}"
        if not SHA.match(sha):
            problems.append(f"{where}: {m.group('action')}@{sha} is not pinned to a full commit SHA")
            continue
        variants = pins.get(repo)
        if not variants:
            compared.append(f"{repo} (scaffold only, not compared)")
            continue
        if len(variants) == 1:
            (want_sha, want_ver), _ = next(iter(variants.items()))
            if (sha, ver) != (want_sha, want_ver):
                problems.append(
                    f"{where} pins {repo}@{sha[:12]} # {ver}; the workflows use "
                    f"{want_sha[:12]} # {want_ver}: update the scaffold's pin to `{repo}@{want_sha} # {want_ver}`"
                )
                continue
        compared.append(repo)

    for problem in problems:
        print(f"error: {problem}")
    if problems:
        return 1
    print(
        f"{count} workflow `uses:` pin {len(pins)} action repositories, one commit each; "
        f"the scaffold's pins agree: {', '.join(compared)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
