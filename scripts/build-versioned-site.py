#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F.
"""Build the versioned quack-rs book: one copy per release line, plus main.

Layout of the output directory (what docs.yml deploys to quack-rs.com):

    /               the latest release's book (canonical, indexed, sitemap)
    /vX.Y/          each release line X.Y from MIN_RELEASE on, built from the
                    newest non-prerelease tag vX.Y.Z (the latest line too, so
                    a /vX.Y/ link keeps meaning X.Y after the next release)
    /dev/           the book from the working tree (main in CI), with the
                    rustdoc of the same tree under /dev/api/ when --dev-api
    /versions.json  the published versions, for tools
    /version-switcher.{js,css}

Every page of every copy gets a version menu, and pages that are not the
latest release get a banner saying so (book/versioning/). Pages outside the
root are marked noindex, and their canonical URL is the same page at the root
when the latest release has it, so search engines index one copy.

Links to `https://docs.rs/quack-rs/latest/` are pinned to the release a copy
was built from (`https://docs.rs/quack-rs/X.Y.Z/`), since `latest` moves on at
the next release; in /dev/ they point at /dev/api/ when it is built.

Each release is built the way its own tag built it: with the mdBook version
named in that tree's `book/mdbook-version` (tags up to v0.18.0 predate the
file and were all built with LEGACY_MDBOOK), after that tree's own
`scripts/generate-sitemap.py`, followed by its `scripts/seo-postbuild.py`
when it has one. The output depends only on the set of tags, the working
tree and those pinned tool versions.

mdBook binaries are taken from --tools DIR (as `mdbook-<version>`), else from
`mdbook` on PATH when its version matches, else downloaded from the mdBook
GitHub release and checked against MDBOOK_SHA256 (x86_64 Linux only).

Usage:
    scripts/build-versioned-site.py --out site                # needs git tags
    scripts/build-versioned-site.py --out site --dev-api target/doc
    python3 -m http.server -d site                          # preview

Exit status 1 on a failed build or a failed post-build check.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SITE_URL = "https://quack-rs.com"
DOCS_RS_LATEST = "https://docs.rs/quack-rs/latest/"

# The oldest release line published. 0.16 is the oldest line SECURITY.md
# still lists as supported; raise this when a line reaches end of life.
MIN_RELEASE = (0, 18, 0)

# Every tag up to and including v0.18.0 was built by docs.yml with this
# version and has no book/mdbook-version file (mdBook 0.5 rejects their
# book.toml: `multilingual` is no longer a known key).
LEGACY_MDBOOK = "0.4.40"

# sha256 of mdbook-v<version>-x86_64-unknown-linux-gnu.tar.gz from
# https://github.com/rust-lang/mdBook/releases. Add an entry when
# book/mdbook-version moves to a new version.
MDBOOK_SHA256 = {
    "0.4.40": "9ef07fd288ba58ff3b99d1c94e6d414d431c9a61fdb20348e5beb74b823d546b",
    "0.5.4": "3f28de05dafca9d0f2eab99c662116b0e37b89b1d96a08f8f430b9eeae958cd7",
}

TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
# What git archive / the working-tree copy needs to build a book.
BOOK_PATHS = ["book.toml", "book", "scripts"]
SWITCHER_SRC = REPO_ROOT / "book" / "versioning"
VERSIONS_PLACEHOLDER = "/*@@VERSIONS@@*/ null"
INJECT_MARK = "<!-- versioned-site -->"


@dataclass
class Build:
    slug: str  # "v0.18" or "dev"
    label: str  # shown in the version menu
    version: str  # "0.18.0", or "main" for dev
    ref: str | None  # git tag, or None for the working tree
    out: Path = field(default=Path())
    pages: list[str] = field(default_factory=list)


def run(cmd: list[str], cwd: Path | None = None) -> str:
    result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.stderr.write(result.stdout + result.stderr)
        raise SystemExit(f"error: `{' '.join(cmd)}` failed with exit status {result.returncode}")
    return result.stdout


def release_lines(tags: list[str]) -> list[tuple[tuple[int, int, int], str]]:
    """The newest tag of each X.Y line at or above MIN_RELEASE, newest line first."""
    newest: dict[tuple[int, int], tuple[tuple[int, int, int], str]] = {}
    for tag in tags:
        m = TAG_RE.match(tag)
        if not m:
            continue  # prereleases (v1.0.0-rc.1) and other tags
        ver = (int(m[1]), int(m[2]), int(m[3]))
        if ver < MIN_RELEASE:
            continue
        line = ver[:2]
        if line not in newest or ver > newest[line][0]:
            newest[line] = (ver, tag)
    return sorted(newest.values(), reverse=True)


def mdbook_for(version: str, tools: Path) -> Path:
    candidate = tools / f"mdbook-{version}"
    if candidate.is_file():
        return candidate
    on_path = shutil.which("mdbook")
    if on_path:
        out = subprocess.run([on_path, "--version"], capture_output=True, text=True).stdout
        if out.strip() == f"mdbook v{version}":
            return Path(on_path)
    if version not in MDBOOK_SHA256:
        raise SystemExit(f"error: no checksum for mdBook {version}; add it to MDBOOK_SHA256")
    if sys.platform != "linux" or os.uname().machine != "x86_64":
        raise SystemExit(
            f"error: mdBook {version} not found; put it at {candidate} "
            "(automatic download is x86_64 Linux only)"
        )
    url = (
        f"https://github.com/rust-lang/mdBook/releases/download/v{version}/"
        f"mdbook-v{version}-x86_64-unknown-linux-gnu.tar.gz"
    )
    print(f"downloading mdBook {version}")
    with urllib.request.urlopen(url, timeout=120) as resp:
        data = resp.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != MDBOOK_SHA256[version]:
        raise SystemExit(f"error: {url}: sha256 {digest}, expected {MDBOOK_SHA256[version]}")
    tools.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        member = tar.getmember("mdbook")
        src = tar.extractfile(member)
        assert src is not None
        candidate.write_bytes(src.read())
    candidate.chmod(0o755)
    return candidate


def export_tree(ref: str | None, dest: Path) -> None:
    dest.mkdir(parents=True)
    if ref is None:
        for name in BOOK_PATHS:
            src = REPO_ROOT / name
            if src.is_dir():
                shutil.copytree(src, dest / name, ignore=shutil.ignore_patterns("book", "target"))
            else:
                shutil.copy2(src, dest / name)
        return
    archive = subprocess.run(
        ["git", "archive", "--format=tar", ref, *BOOK_PATHS],
        cwd=REPO_ROOT,
        capture_output=True,
        check=True,
    ).stdout
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(dest, filter="data")


def build_book(build: Build, work: Path, tools: Path) -> Path:
    tree = work / build.slug
    export_tree(build.ref, tree)
    pin = tree / "book" / "mdbook-version"
    version = pin.read_text(encoding="utf-8").strip() if pin.is_file() else LEGACY_MDBOOK
    mdbook = mdbook_for(version, tools)
    print(f"building {build.slug} from {build.ref or 'the working tree'} with mdBook {version}")
    if (tree / "scripts" / "generate-sitemap.py").is_file():
        run([sys.executable, "scripts/generate-sitemap.py"], cwd=tree)
    # The book's build-dir is book/book; a stale one from the working tree
    # was excluded by export_tree.
    run([str(mdbook), "build"], cwd=tree)
    out = tree / "book" / "book"
    if (tree / "scripts" / "seo-postbuild.py").is_file():
        run([sys.executable, "scripts/seo-postbuild.py", str(out)], cwd=tree)
    return out


def is_book_page(path: Path) -> bool:
    head = path.read_text(encoding="utf-8", errors="replace")[:600]
    return "Book generated using mdBook" in head


def book_pages(root: Path) -> list[str]:
    return sorted(
        p.relative_to(root).as_posix()
        for p in root.rglob("*.html")
        if p.name not in {"404.html", "print.html"} and is_book_page(p)
    )


CANONICAL_RE = re.compile(r'\s*<link rel="canonical"[^>]*>')
OG_URL_RE = re.compile(r'\s*<meta property="og:url"[^>]*>')
ROBOTS_RE = re.compile(r'\s*<meta name="robots"[^>]*>')


def root_url(page: str) -> str:
    if page in {"index.html", "introduction.html"}:
        return SITE_URL + "/"
    return f"{SITE_URL}/{page}"


def postprocess(
    site: Path,
    copy: Path,
    build: Build,
    *,
    is_root: bool,
    root_pages: set[str],
    dev_api: bool,
) -> int:
    """Rewrite every book page under `copy`; return how many were rewritten."""
    count = 0
    for path in sorted(copy.rglob("*.html")):
        rel_in_copy = path.relative_to(copy).as_posix()
        if rel_in_copy.startswith("api/") or not is_book_page(path):
            continue
        text = path.read_text(encoding="utf-8")
        if INJECT_MARK in text:
            raise SystemExit(f"error: {path} was already post-processed")
        depth = len(path.relative_to(site).parts) - 1
        to_site = "../" * depth
        to_copy = "../" * (len(path.relative_to(copy).parts) - 1)

        if build.ref is not None:
            text = text.replace(DOCS_RS_LATEST, f"https://docs.rs/quack-rs/{build.version}/")
        elif dev_api:
            text = text.replace(DOCS_RS_LATEST, f"{to_copy}api/")

        head = [INJECT_MARK]
        if not is_root:
            text = CANONICAL_RE.sub("", text)
            text = OG_URL_RE.sub("", text)
            text = ROBOTS_RE.sub("", text)
            head.append('<meta name="robots" content="noindex, follow">')
            if rel_in_copy in root_pages or rel_in_copy == "index.html":
                head.append(f'<link rel="canonical" href="{html.escape(root_url(rel_in_copy))}">')
        head.append(f'<meta name="docs-version" content="{html.escape(build.slug)}">')
        head.append(f'<link rel="stylesheet" href="{to_site}version-switcher.css">')
        head.append(f'<script src="{to_site}version-switcher.js" defer></script>')
        if "</head>" not in text:
            raise SystemExit(f"error: {path} has no </head>")
        text = text.replace("</head>", "\n        ".join(head) + "\n    </head>", 1)
        path.write_text(text, encoding="utf-8")
        count += 1
    return count


def write_switcher(site: Path, builds: list[Build], latest: str) -> None:
    data = {
        "latest": latest,
        "versions": [
            {"slug": b.slug, "label": b.label, "version": b.version, "pages": b.pages}
            for b in builds
        ],
    }
    js = (SWITCHER_SRC / "version-switcher.js").read_text(encoding="utf-8")
    if VERSIONS_PLACEHOLDER not in js:
        raise SystemExit("error: version-switcher.js lost its VERSIONS placeholder")
    payload = json.dumps(data, separators=(",", ":"), sort_keys=True)
    (site / "version-switcher.js").write_text(js.replace(VERSIONS_PLACEHOLDER, payload), "utf-8")
    shutil.copy2(SWITCHER_SRC / "version-switcher.css", site / "version-switcher.css")
    public = {
        "latest": latest,
        "versions": [
            {
                "slug": b.slug,
                "version": b.version,
                "url": SITE_URL + "/" if b.slug == latest else f"{SITE_URL}/{b.slug}/",
            }
            for b in builds
        ],
    }
    (site / "versions.json").write_text(json.dumps(public, indent=2) + "\n", "utf-8")


def check(site: Path, builds: list[Build], latest: str) -> list[str]:
    errors = []
    for b in builds:
        base = site / b.slug
        if not (base / "index.html").is_file():
            errors.append(f"{b.slug}/index.html missing")
        for extra in ("CNAME", "sitemap.xml", "robots.txt"):
            if (base / extra).exists():
                errors.append(f"{b.slug}/{extra} should exist only at the root")
    for name in ("index.html", "CNAME", "sitemap.xml", "robots.txt", "versions.json"):
        if not (site / name).is_file():
            errors.append(f"/{name} missing")
    for path in site.rglob("*.html"):
        rel = path.relative_to(site).as_posix()
        if "/api/" in f"/{rel}" or not is_book_page(path):
            continue
        text = path.read_text(encoding="utf-8")
        if INJECT_MARK not in text:
            errors.append(f"{rel}: no version switcher")
        top = rel.split("/", 1)[0]
        in_root = not any(top == b.slug for b in builds)
        noindex = 'name="robots" content="noindex' in text
        if not in_root and not noindex:
            errors.append(f"{rel}: a non-root copy must be noindex")
        if in_root and rel not in {"404.html", "print.html"} and noindex:
            errors.append(f"{rel}: a root page must be indexable")
        if in_root and DOCS_RS_LATEST in text:
            errors.append(f"{rel}: unpinned docs.rs link")
    if not any(b.slug == latest for b in builds):
        errors.append(f"latest {latest} is not among the builds")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--out", type=Path, required=True, help="output directory (replaced)")
    parser.add_argument("--tools", type=Path, default=REPO_ROOT / "target" / "mdbook-tools")
    parser.add_argument(
        "--dev-api",
        type=Path,
        help="rustdoc output directory (target/doc) to publish under /dev/api/",
    )
    parser.add_argument("--no-dev", action="store_true", help="skip the /dev/ build")
    args = parser.parse_args()

    tags = run(["git", "tag", "--list", "v*"], cwd=REPO_ROOT).split()
    lines = release_lines(tags)
    if not lines:
        print(
            "error: no release tags found; fetch them first (git fetch --tags, "
            "or actions/checkout with fetch-depth: 0)",
            file=sys.stderr,
        )
        return 1
    latest = f"v{lines[0][0][0]}.{lines[0][0][1]}"
    builds = []
    for ver, tag in lines:
        slug = f"v{ver[0]}.{ver[1]}"
        version = ".".join(map(str, ver))
        label = f"{version} (latest)" if slug == latest else version
        builds.append(Build(slug, label, version, tag))
    if not args.no_dev:
        builds.append(Build("dev", "main", "main", None))

    site: Path = args.out.resolve()
    if site.exists():
        shutil.rmtree(site)
    site.mkdir(parents=True)

    with tempfile.TemporaryDirectory(prefix="quack-rs-site-") as tmp:
        work = Path(tmp)
        for b in builds:
            b.out = build_book(b, work, args.tools.resolve())
            b.pages = book_pages(b.out)

        root_build = next(b for b in builds if b.slug == latest)
        root_pages = set(root_build.pages)
        shutil.copytree(root_build.out, site, dirs_exist_ok=True)
        n = postprocess(site, site, root_build, is_root=True, root_pages=root_pages, dev_api=False)
        print(f"/: {root_build.version}, {n} pages")
        for b in builds:
            dest = site / b.slug
            shutil.copytree(b.out, dest)
            for extra in ("CNAME", "sitemap.xml", "robots.txt"):
                (dest / extra).unlink(missing_ok=True)
            dev_api = False
            if b.ref is None and args.dev_api:
                api = args.dev_api.resolve()
                if not (api / "quack_rs" / "index.html").is_file():
                    print(f"error: {api} has no quack_rs/index.html", file=sys.stderr)
                    return 1
                shutil.copytree(api, dest / "api")
                dev_api = True
            n = postprocess(site, dest, b, is_root=False, root_pages=root_pages, dev_api=dev_api)
            print(f"/{b.slug}/: {b.version}, {n} pages")

    write_switcher(site, builds, latest)
    errors = check(site, builds, latest)
    for e in errors:
        print(f"error: {e}", file=sys.stderr)
    if errors:
        return 1
    print(f"site written to {site}: latest {latest}, versions {', '.join(b.slug for b in builds)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
