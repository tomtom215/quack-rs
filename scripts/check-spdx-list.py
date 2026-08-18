#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check quack-rs's SPDX shortlist against the official registry.

`validate::spdx::COMMON_SPDX_LICENSES` is a curated shortlist, not the whole
registry — but every entry on it should be a real, current SPDX identifier. A
typo would make quack-rs reject a license nobody could ever satisfy, and SPDX
occasionally deprecates identifiers (`GPL-3.0` became `GPL-3.0-only` /
`GPL-3.0-or-later`), which would make quack-rs recommend a stale name.

The authority is `json/licenses.json` in `spdx/license-list-data`.

Exit codes:
    0  every entry is a real, non-deprecated SPDX identifier
    1  an entry is unknown or deprecated
    2  the registry could not be fetched or parsed (network, layout change)

Usage:
    python3 scripts/check-spdx-list.py
"""

from __future__ import annotations

import json
import re
import sys
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SPDX_RS = REPO_ROOT / "src" / "validate" / "spdx.rs"

REGISTRY_URL = (
    "https://raw.githubusercontent.com/spdx/license-list-data/main/json/licenses.json"
)


def main() -> int:
    text = SPDX_RS.read_text()
    match = re.search(
        r"pub const COMMON_SPDX_LICENSES:\s*&\[&str\]\s*=\s*&\[(.*?)\];", text, re.DOTALL
    )
    if not match:
        sys.exit(f"could not find COMMON_SPDX_LICENSES in {SPDX_RS}")
    ours = re.findall(r'"([^"]+)"', match.group(1))
    print(f"quack-rs shortlist: {len(ours)} identifiers")

    try:
        with urllib.request.urlopen(REGISTRY_URL, timeout=60) as response:
            registry = json.loads(response.read().decode("utf-8"))
    except Exception as error:  # noqa: BLE001 - any failure is "could not check"
        print(f"::warning::could not fetch {REGISTRY_URL}: {error}")
        return 2

    entries = registry.get("licenses")
    if not entries:
        print(f"::warning::{REGISTRY_URL} listed no licenses — layout may have changed")
        return 2
    print(
        f"SPDX registry: {len(entries)} identifiers "
        f"(list version {registry.get('licenseListVersion', 'unknown')})"
    )

    known = {e["licenseId"] for e in entries}
    deprecated = {e["licenseId"] for e in entries if e.get("isDeprecatedLicenseId")}
    not_osi = {e["licenseId"] for e in entries if not e.get("isOsiApproved")}

    unknown = [x for x in ours if x not in known]
    stale = [x for x in ours if x in deprecated]

    if unknown:
        print(f"\nFAIL: not SPDX identifiers at all: {', '.join(unknown)}")
    if stale:
        print(f"\nFAIL: deprecated by SPDX: {', '.join(stale)}")
    if unknown or stale:
        print(f"\nFix COMMON_SPDX_LICENSES in {SPDX_RS.relative_to(REPO_ROOT)}.")
        return 1

    # Informational only — SSPL-1.0 is deliberately listed and deliberately not
    # OSI-approved. This exists so a *new* non-OSI entry is noticed in review.
    flagged = sorted(x for x in ours if x in not_osi)
    if flagged:
        print(f"\nnote: listed but not OSI-approved: {', '.join(flagged)}")

    if ours != sorted(ours):
        print("\nFAIL: COMMON_SPDX_LICENSES is not sorted")
        return 1

    print("\nOK: every listed identifier is current SPDX")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
