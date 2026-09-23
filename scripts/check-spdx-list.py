#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check quack-rs's SPDX license and exception lists against the official registry.

`validate::spdx::COMMON_SPDX_LICENSES` is a curated shortlist, not the whole
registry — but every entry on it should be a real, current SPDX identifier. A
typo would make quack-rs reject a license nobody could ever satisfy, and SPDX
occasionally deprecates identifiers (`GPL-3.0` became `GPL-3.0-only` /
`GPL-3.0-or-later`), which would make quack-rs recommend a stale name.

`validate::spdx::SPDX_LICENSE_EXCEPTIONS` (the identifiers accepted after
`WITH`) is meant to be the *whole* exception registry, so it is checked in both
directions: an entry that is not a current exception fails, and so does a
current exception the list is missing — that is how a registry update is
noticed instead of quack-rs rejecting a newly added exception forever.

The authority is `json/licenses.json` and `json/exceptions.json` in
`spdx/license-list-data`.

Exit codes:
    0  both lists are correct
    1  an entry is unknown, deprecated or unsorted, or an exception is missing
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
EXCEPTIONS_RS = REPO_ROOT / "src" / "validate" / "spdx_exceptions.rs"

REGISTRY = "https://raw.githubusercontent.com/spdx/license-list-data/main/json/"
LICENSES_URL = REGISTRY + "licenses.json"
EXCEPTIONS_URL = REGISTRY + "exceptions.json"


def rust_list(path: Path, name: str) -> list[str]:
    text = path.read_text()
    match = re.search(
        rf"pub const {name}:\s*&\[&str\]\s*=\s*&\[(.*?)\];", text, re.DOTALL
    )
    if not match:
        sys.exit(f"could not find {name} in {path}")
    return re.findall(r'"([^"]+)"', match.group(1))


def fetch(url: str, key: str) -> tuple[list[dict], str] | None:
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            registry = json.loads(response.read().decode("utf-8"))
    except Exception as error:  # noqa: BLE001 - any failure is "could not check"
        print(f"::warning::could not fetch {url}: {error}")
        return None
    entries = registry.get(key)
    if not entries:
        print(f"::warning::{url} listed no {key} — layout may have changed")
        return None
    return entries, registry.get("licenseListVersion", "unknown")


def main() -> int:
    ours = rust_list(SPDX_RS, "COMMON_SPDX_LICENSES")
    our_exceptions = rust_list(EXCEPTIONS_RS, "SPDX_LICENSE_EXCEPTIONS")
    print(f"quack-rs shortlist: {len(ours)} identifiers")
    print(f"quack-rs exception list: {len(our_exceptions)} identifiers")

    licenses = fetch(LICENSES_URL, "licenses")
    exceptions = fetch(EXCEPTIONS_URL, "exceptions")
    if licenses is None or exceptions is None:
        return 2
    entries, version = licenses
    print(f"SPDX registry: {len(entries)} identifiers (list version {version})")
    exception_entries, exception_version = exceptions
    print(
        f"SPDX exception registry: {len(exception_entries)} identifiers "
        f"(list version {exception_version})"
    )

    known = {e["licenseId"] for e in entries}
    deprecated = {e["licenseId"] for e in entries if e.get("isDeprecatedLicenseId")}
    not_osi = {e["licenseId"] for e in entries if not e.get("isOsiApproved")}

    current_exceptions = {
        e["licenseExceptionId"]
        for e in exception_entries
        if not e.get("isDeprecatedLicenseId")
    }

    failed = False

    unknown = [x for x in ours if x not in known]
    stale = [x for x in ours if x in deprecated]
    if unknown:
        print(f"\nFAIL: not SPDX identifiers at all: {', '.join(unknown)}")
    if stale:
        print(f"\nFAIL: deprecated by SPDX: {', '.join(stale)}")
    if unknown or stale:
        print(f"\nFix COMMON_SPDX_LICENSES in {SPDX_RS.relative_to(REPO_ROOT)}.")
        failed = True

    not_exceptions = [x for x in our_exceptions if x not in current_exceptions]
    missing = sorted(current_exceptions - set(our_exceptions))
    if not_exceptions:
        print(
            "\nFAIL: not current SPDX license exceptions (unknown or deprecated): "
            f"{', '.join(not_exceptions)}"
        )
    if missing:
        print(f"\nFAIL: SPDX exceptions missing from the list: {', '.join(missing)}")
    if not_exceptions or missing:
        print(
            f"\nFix SPDX_LICENSE_EXCEPTIONS in {EXCEPTIONS_RS.relative_to(REPO_ROOT)} and "
            f"update the license list version its doc comment names (now {exception_version})."
        )
        failed = True

    # Informational only — SSPL-1.0 is deliberately listed and deliberately not
    # OSI-approved. This exists so a *new* non-OSI entry is noticed in review.
    flagged = sorted(x for x in ours if x in not_osi)
    if flagged:
        print(f"\nnote: listed but not OSI-approved: {', '.join(flagged)}")

    for name, values in (
        ("COMMON_SPDX_LICENSES", ours),
        ("SPDX_LICENSE_EXCEPTIONS", our_exceptions),
    ):
        if values != sorted(values):
            print(f"\nFAIL: {name} is not sorted")
            failed = True

    if failed:
        return 1
    print("\nOK: every listed identifier is current SPDX, and the exception list is complete")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
