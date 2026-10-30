# Releasing quack-rs

Maintainer runbook for cutting a new release. Every step is documented so any
authorised maintainer can follow the process independently.

---

## Prerequisites

Before you can cut a release, ensure the following are in place:

| Requirement | Where to configure |
|-------------|-------------------|
| Push access to `main` and ability to push tags | GitHub repository settings |
| crates.io **Trusted Publishing** for `quack-rs`: repository `tomtom215/quack-rs`, workflow `release.yml`, environment `crates-io` | crates.io → the crate's Settings → Trusted Publishing |
| `CARGO_REGISTRY_TOKEN`, **only until Trusted Publishing is configured**; then delete it. Store it as a secret **of the `crates-io` environment**, never as a repository secret | Settings → Environments → crates-io → Environment secrets |
| `crates-io` environment with yourself as required reviewer, and **Deployment branches and tags** limited to the tag pattern `v*` | Settings → Environments → crates-io |
| A tag ruleset restricting who may create `v*` tags | Settings → Rules → Rulesets |
| Branch protection on `main` requiring the single check **CI passed** | Settings → Rules (or Branches) |
| GPG key configured for signed tags | `git config --global user.signingkey <KEY_ID>` |

The `publish` job prefers Trusted Publishing: it exchanges the job's OIDC token
for a crates.io token that is revoked when the job ends, so no long-lived
credential exists. Until the crates.io side is configured the exchange fails,
the job warns, and it falls back to `CARGO_REGISTRY_TOKEN`; with neither it
fails. `release.yml` runs from the tagged tree, so the tag ruleset is what stops
a tag on an edited workflow; the workflow itself refuses a lightweight tag and a
tag on a commit that is not on `main`.

Where the fallback token lives matters. A repository secret is readable by
every workflow in the repository: anyone who can push a branch can add a
workflow that prints it, and the `crates-io` environment's required reviewer is
never consulted. An environment secret is released only to a job that names
the environment, after the reviewer approves, and only for the refs the
environment allows.

### Setting up Trusted Publishing (once, in this order)

1. crates.io → `quack-rs` → Settings → Trusted Publishing → add a GitHub
   publisher: owner `tomtom215`, repository `quack-rs`, workflow `release.yml`,
   environment `crates-io`.
2. GitHub → Settings → Environments → `crates-io`: required reviewers, and
   deployment branches and tags limited to `v*`. If a `CARGO_REGISTRY_TOKEN`
   repository secret exists, move it into this environment's secrets and
   delete the repository secret, as a stop-gap until step 4.
3. Cut the next release. The `publish` job must log `Publishing with a Trusted
   Publishing token.` (not the `::warning::` about the fallback), and the
   crates.io version page must show the publisher.
4. Only then: delete `CARGO_REGISTRY_TOKEN` (environment and repository), revoke
   the token on crates.io, and remove the `CARGO_REGISTRY_TOKEN` fallback and
   the `continue-on-error` of the `trusted` step from `release.yml`, so a failed
   exchange fails the release instead of reaching for a long-lived credential.
5. If the crate's Trusted Publishing settings offer to disallow publishing with
   API tokens, turn that on.

---

## Semantic versioning policy

quack-rs is pre-1.0, and Cargo treats the **leftmost non-zero component as the
major version**. For a `0.MINOR.PATCH` crate that means `0.16.1` is a compatible
update to `0.16.0`, while `0.17.0` is a new major line: a dependant written as
`quack-rs = "0.16"` picks up `0.16.1` automatically and never moves to `0.17`.
So the *minor* position — not the major — is where a breaking change goes.

| Change type | Version bump | Real examples |
|-------------|-------------|---------------|
| No public API change: bug fixes, dependency and security bumps, doc corrections, internal refactors | **PATCH** (0.16.0 → 0.16.1) | 0.12.1 (advisory bumps + clippy fixes), 0.7.1, 0.5.1 |
| New public API — backward compatible | **MINOR** (0.16.x → 0.17.0) | 0.13.0 (six new modules), 0.15.0 (`Value::as_blob`) |
| Removed or changed public API, trait incompatibility | **MINOR** (0.16.x → 0.17.0) | 0.16.0 (`read_uuid` `i128` → `u128`), 0.8.0 |
| Declaring the public API stable | **MAJOR** (0.x.y → 1.0.0) | Not yet used |

Breaking and additive changes share the minor position while the crate is
pre-1.0; that is Cargo's rule, not a shortcut. Mark breaking entries in
`CHANGELOG.md` with a bold **Breaking:** prefix so they are greppable, since the
version number alone no longer distinguishes them from additive ones.

**1.0.0 is a deliberate commitment, not an automatic consequence of a breaking
change.** Cutting it says the public API is stable and that future breakage will
require 2.0.0. Do not let a single incompatible change trigger it.

After 1.0.0 the ordinary semver rules apply: PATCH for fixes, MINOR for additive
API, MAJOR for breaking changes.

API compatibility is defined by all `pub` items in all modules, plus the
`quack_rs::prelude::*` glob export. Internal items (marked `pub(crate)`) are
not part of the public API.

The MSRV has not yet been raised on consumers, so there is no precedent to
follow: 0.13.0 *corrected* a `rust-version` that was already understated, and
0.16.0 *lowered* it. When a genuine raise does happen, treat it as breaking — it
can stop a dependant from building, which is what "breaking" means here — and so
give it a minor bump and a **Breaking:** entry.

Pre-release versions use the suffix convention `vX.Y.Z-alpha.N`,
`vX.Y.Z-beta.N`, or `vX.Y.Z-rc.N`.

---

## TL;DR (happy path)

```bash
# 1. Prepare
vim CHANGELOG.md      # Move [Unreleased] → [X.Y.Z] - YYYY-MM-DD
vim Cargo.toml        # version = "X.Y.Z"
# Every committed lockfile records quack-rs's own version; release.yml packages
# and publishes with --locked, which refuses a stale one (Step 3).
for d in . examples/hello-ext fuzz book/doctest benches/engine; do (cd "$d" && cargo update --workspace); done
scripts/sync-book-changelog.py   # regenerate the book's changelog page
# ...and bump every "0.X" version snippet in README.md / book / CITATION.cff (Step 3b)
python3 scripts/check-release-metadata.py   # lists every place still on the old version
git add -A
git commit -m "chore: release vX.Y.Z"
git push origin main

# 2. Tag (signed — requires GPG key)
git tag -s "vX.Y.Z" -m "Release vX.Y.Z"
git push origin "vX.Y.Z"

# 3. Wait for CI, then approve the crates-io deployment in the GitHub Actions UI

# 4. Verify: tag on origin, crates.io max_version, docs.rs build (Step 9)
```

Version snippets in the docs are bumped in the same commit as `Cargo.toml`
(Step 3b), and that commit is the one tagged.

---

## Step-by-step guide

### Step 1 — Verify CI is green on `main`

**The entire `CI` workflow must be green on `main` before tagging — every job,
not a subset.**

This used to be a hand-maintained list of nine check names. It had drifted to
cover fewer than a third of the jobs in `.github/workflows/ci.yml`, and the
names no longer matched the rendered ones, so a maintainer following it
literally could tag a release with `Miri`, `LeakSanitizer`, `osv-scan`,
`semver` or `abi-guard` red. Read the run, not a list that rots:

```bash
gh run list --branch main --workflow ci.yml --limit 1
gh run view <run-id>   # every blocking job must be green; none may be skipped
```

Do not enumerate the jobs here again. Branch protection on `main` is the
mechanism that should enforce this; this step is the human double-check. The
jobs `.github/workflows/README.md` marks *informational* (`clippy-beta`,
`latest-stable`, `nightly`, `test-bundled-duckdb-2-0`) may be red without
blocking, but read why before tagging: a red `latest-stable`, for example,
means the newest Rust reports something the pinned one does not, which the
next toolchain bump will have to fix (CONTRIBUTING.md, "The Rust toolchain").

### Step 2 — Update CHANGELOG.md

Move items from `[Unreleased]` to a new dated section:

```markdown
## [Unreleased]          ← keep this header; leave it empty for next cycle

## [0.3.0] - 2026-04-01  ← new section; date in YYYY-MM-DD

### Added
- ...

### Fixed
- ...
```

Update the comparison link at the bottom of `CHANGELOG.md`:

```markdown
[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/tomtom215/quack-rs/compare/v0.2.0...v0.3.0
```

The **book changelog** (`book/src/reference/changelog.md`) is generated from
`CHANGELOG.md`: regenerate it with `scripts/sync-book-changelog.py` (CI fails
if the two diverge). Do not edit it by hand.

### Step 2b — Update SECURITY.md

If this is a **minor or major** release, update the supported versions table in
`SECURITY.md`: add a `| X.Y.x | Yes` row for the new line, and end the support
of the line before the previous one (the table's policy is the two newest minor
lines).

The versioned book (quack-rs.com) publishes a copy for every release line from
`MIN_RELEASE` in `scripts/build-versioned-site.py` upwards. When a line goes
end-of-life here, raise `MIN_RELEASE` to the oldest line still supported, in the
same commit, so the site stops serving documentation for releases nobody
should start with. `scripts/check-release-metadata.py` fails while the table
and `MIN_RELEASE` disagree, or while the table has no row for the version in
`Cargo.toml`.

### Step 3 — Bump the version in `Cargo.toml`

```toml
[package]
version = "0.3.0"   # ← update this
```

Then bring every committed lockfile along, **in the same commit**. Each one
records quack-rs's own version: the root `Cargo.lock` as the package itself,
the others (`examples/hello-ext`, `fuzz`, `book/doctest`, `benches/engine`) as
a path dependency:

```bash
for d in . examples/hello-ext fuzz book/doctest benches/engine; do (cd "$d" && cargo update --workspace); done
git diff --stat -- '*Cargo.lock'   # one line changed in each
```

`--workspace` updates only the workspace's own packages, so no dependency
moves. This is not optional: `release.yml` runs `cargo package --locked` and
`cargo publish --locked`, which refuse a `Cargo.lock` that still names the
previous version ("cannot update the lock file ... because --locked was
passed"), and nothing in CI commits a lock file it rewrote. A `Cargo.toml` bump
without its lockfiles used to pass every pull-request job and fail the
`package` job after the whole of `ci.yml` had run. `ci.yml`'s publish dry-run
now uses `--locked` too, and `scripts/check-release-metadata.py` (the `doc`
job, and `validate` in `release.yml`) names any lockfile left behind.

### Step 3b — Bump the version in the documentation snippets, in the same PR

Every `quack-rs = "0.X"` / `version = "0.X"` snippet a reader copies — in
`README.md`, the book, the feature documentation in `Cargo.toml`, the `src/`
rustdoc, and `CITATION.cff` — must name the new version **in the release PR
itself**, as must `SECURITY.md` (Step 2b) and the README's "The latest is ..."
line. The commit that gets tagged is that PR's merge commit, so a snippet
bumped in a follow-up PR is wrong in the tagged source, in the published
`.crate` (whose README is what crates.io renders) and in the docs.rs build of
the release. Find the stale ones by grepping for the previous version, for
example (replace `<PREV>` with the previous minor, dots escaped — `0\.17` when
releasing 0.18.0):

```bash
git grep -n '<PREV>' -- README.md book/src Cargo.toml CITATION.cff src
```

Two places hold a date, not a version, so the grep cannot find them:
`CITATION.cff`'s `date-released` and the README's "The latest is X.Y.Z
(YYYY-MM-DD)". Give both the date of the CHANGELOG heading (the day you push
the tag). The `publish` job waits for approval and can finish on the next day:
0.19.0 is dated 2026-10-22 everywhere, and crates.io's `created_at` for it is
2026-10-23T06:07Z.

`python3 scripts/check-release-metadata.py` does all of this mechanically: the
lockfiles, the CHANGELOG heading and its two link definitions, `CITATION.cff`
(version and date), the `SECURITY.md` row and `MIN_RELEASE`, the README line,
and every snippet. CI runs it in the `doc` job, so a release PR cannot merge
with a place missed. For a pre-release version only the lockfile and CHANGELOG
checks apply: the docs keep naming the latest stable release.

### Step 4 — Commit the version bump

```bash
# ...plus SECURITY.md and the snippets from Steps 2b and 3b
git add CHANGELOG.md Cargo.toml Cargo.lock examples/hello-ext/Cargo.lock \
    fuzz/Cargo.lock book/doctest/Cargo.lock benches/engine/Cargo.lock
git commit -m "chore: release v0.3.0"
git push origin main
```

Wait for the CI run on `main` to pass before tagging. If the release goes
through a pull request, tag the commit that PR produced on `main` — with the
changelog, the version and the snippets all in it — never a commit before it.

### Step 5 — Create a signed git tag

Signed tags provide a cryptographic proof of who authorised the release.

```bash
# Ensure your GPG key is available
gpg --list-secret-keys

# Create a signed, annotated tag
git tag -s "v0.3.0" -m "Release v0.3.0"

# Verify the signature before pushing
git verify-tag "v0.3.0"

# Push the tag — this triggers the release workflow
git push origin "v0.3.0"
```

If GPG signing is not available (discouraged), use an annotated tag:

```bash
git tag -a "v0.3.0" -m "Release v0.3.0"
git push origin "v0.3.0"
```

A lightweight tag (`git tag v0.3.0`) is refused by the `validate` job, as is a
tag on a commit that is not on `main`. `validate` checks that the tag is
annotated, not that it is signed: `git verify-tag` (above, and Step 9) is the
only signature check.

### Step 6 — Monitor the release workflow

Navigate to **Actions → Release** in the GitHub UI. The workflow runs these jobs
in order:

```
validate          Tag is annotated and on main; tag format, Cargo.toml version, dated CHANGELOG
    │             entry, and scripts/check-release-metadata.py (lockfiles, CITATION.cff, SECURITY.md, snippets)
ci                All of ci.yml (workflow_call): every gate a pull request runs,
    │             including semver, Miri, the sanitizers, cargo-deny, OSV, the
    │             publish dry-run and extension-load on every pinned DuckDB
package           Build .crate, generate SHA256SUMS, attest SLSA provenance; record rustc
    │
publish           ← awaits manual approval in the crates-io environment;
    │               re-packages with the recorded rustc and stops unless the .crate's SHA-256
    │               equals SHA256SUMS, then cargo publish, then checks crates.io serves the attested .crate
github-release    Create GitHub release with notes, .crate, SHA256SUMS
    │
docs              Dispatch docs.yml on main: the book for this release at / and /vX.Y/
```

`publish` and `github-release` write a summary to the Actions UI; the other
jobs report in their logs.

### Step 7 — Approve the crates.io deployment

The `publish` job is gated behind the `crates-io` environment, which requires a
manual approval from an authorised reviewer. Before approving, read the
`package` job's log (the `.crate` file list in the "Package contents" group,
and its size in the `::notice::` line).

1. In the Actions run for the release workflow, click **Review deployments**
2. Select the `crates-io` environment
3. Click **Approve and deploy**

`publish` first installs the rustc the `package` job recorded (the release
`rust-toolchain.toml` pins; it refuses to continue with any other), packages
the tagged commit again with `cargo package --no-verify --locked`, and stops
unless that `.crate`'s SHA-256 equals the attested `SHA256SUMS`: a crates.io
version can be yanked but never replaced, so a difference must stop the
release before the upload, and it does so before any credential is requested.
It then publishes with a Trusted Publishing token (or, until that is set up,
the `CARGO_REGISTRY_TOKEN` environment secret), using `cargo publish --locked
--no-verify`: ci.yml's publish dry-run already built the packaged sources, and
skipping the second build keeps dependency build scripts away from the token.
Finally it downloads the crate from crates.io and fails if its SHA-256 differs
from the attested artifact's (`cargo package` is reproducible: crates.io's
0.19.0 is byte-identical to a fresh `cargo package` of the v0.19.0 commit).

### Step 8 — Review the GitHub release

`github-release` runs only once crates.io has the crate. When it completes,
inspect the release at:

```
https://github.com/tomtom215/quack-rs/releases/tag/v0.3.0
```

Verify:
- [ ] Release notes match the CHANGELOG section
- [ ] `.crate` artifact is attached
- [ ] `SHA256SUMS` is attached
- [ ] SLSA provenance attestation is linked (visible in the Actions run)
- [ ] Download the `.crate` artifact and verify its SHA-256 matches `SHA256SUMS`
- [ ] Book changelog (`book/src/reference/changelog.md`) is in sync with `CHANGELOG.md`
- [ ] `SECURITY.md` supported versions table is up to date (for minor/major releases)

### Step 9 — Post-release verification

A release is not done until all four of these hold. Nothing else notices when
one does not: `0.17.0` was merged to `main` as a release commit, yet on
2026-09-23 crates.io still reported `0.16.0` as the newest version, so
`cargo add quack-rs` kept resolving to the previous release.

```bash
V=0.3.0   # the version in Cargo.toml

# 1. The tag exists on origin, not only locally
git ls-remote --tags origin "refs/tags/v$V"          # must print one line

# 2. crates.io's newest version is this one
curl -fsSL -A "quack-rs release check" https://crates.io/api/v1/crates/quack-rs \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["crate"]["max_version"])'
# must print $V (for a pre-release, check "newest_version" instead)

# 3. docs.rs built this version
curl -fsSL "https://docs.rs/crate/quack-rs/$V/status.json"
# must print {"doc_status":true,"version":"$V"}; a 404 means no build (yet --
# allow 5-10 minutes after publishing, then investigate)

# 4. The book serves this release at the root and at /vX.Y/
curl -fsSL https://quack-rs.com/versions.json
# "latest" must be "v${V%.*}" and a version entry must read "$V". release.yml's
# `docs` job dispatches docs.yml on main after publishing; if it did not run,
# run `gh workflow run docs.yml --ref main`. For a pre-release, skip this check:
# the `docs` job does not run and versions.json does not change.
```

If any of them fails, the release is incomplete: finish it (push the tag,
approve the `crates-io` deployment, check the docs.rs build log, dispatch
docs.yml) before doing anything else.

The book at the root follows the latest release, not `main`: a correction to the
book reaches `/dev/` when it is merged and the root with the next release. To
publish a correction that cannot wait, cut a patch release.

The remaining checks:

```bash
# Verify the crate is live
cargo search quack-rs

# Verify docs.rs (may take 5–10 minutes to build)
open https://docs.rs/quack-rs/0.3.0

# Verify crate provenance attestation
gh attestation verify \
  --repo tomtom215/quack-rs \
  /path/to/quack-rs-0.3.0.crate

# Verify tag signature
git verify-tag v0.3.0

# Verify checksum against the published artifact
curl -L https://static.crates.io/crates/quack-rs/quack-rs-0.3.0.crate \
  -o quack-rs-0.3.0.crate
sha256sum quack-rs-0.3.0.crate
# Compare with SHA256SUMS in the GitHub release
```

---

## Hotfix releases

For a critical bug in an already-published version. The `validate` job refuses
a tag whose commit is not an ancestor of `main`, so the hotfix branch has to
reach `main` *before* it is tagged, and by a merge commit: a squash merge or a
cherry-pick creates a different commit, and the hotfix tip is then not on
`main` (tagging the branch and cherry-picking the fix onto `main` afterwards,
as this section used to say, is refused).

```bash
# Branch from the release tag
git checkout -b hotfix/0.2.1 v0.2.0

# Apply the fix; update CHANGELOG.md, Cargo.toml and the lockfiles (Steps 2-4)
# ... make changes ...
git add -p
git commit -m "fix: <description>"
git push origin hotfix/0.2.1

# Open a pull request from hotfix/0.2.1 into main and merge it with
# "Create a merge commit". main may carry newer unreleased changes: that is
# fine, the tag below points at the hotfix tip, not at the merge.
# Then tag the hotfix tip (follow steps 5–9 above):
git fetch origin
git tag -s "v0.2.1" -m "Hotfix v0.2.1" origin/hotfix/0.2.1
git push origin "v0.2.1"
```

After the merge `main` must still agree with itself (`scripts/check-release-metadata.py`
runs on it). Where only the hotfix changed the version, the lockfiles, the
CHANGELOG, `CITATION.cff` or the README's "latest" line, `main` simply takes
them, which is right: they describe the newest release. Where `main` changed
them too (it is already on a newer version), keep `main`'s and add the hotfix's
CHANGELOG section below `main`'s own.

The release workflow that runs is the one in the tagged commit, i.e. the
hotfix branch's: a hotfix of a line released before this runbook gained the
release-metadata check runs that line's older `validate`. Copy
`scripts/check-release-metadata.py` from `main` into the hotfix checkout and
run it there before tagging (it checks the tree it sits in); do not commit it
on the hotfix branch unless the fix needs it.

---

## Pre-release versions

Pre-releases use the suffix `alpha`, `beta`, or `rc`:

```bash
# Version in Cargo.toml: 0.3.0-alpha.1
git tag -s "v0.3.0-alpha.1" -m "Pre-release v0.3.0-alpha.1"
git push origin "v0.3.0-alpha.1"
```

The release workflow marks these as **pre-release** on GitHub automatically
(because the tag contains a hyphen after X.Y.Z).

Pre-releases are published to crates.io and are **not** the default `cargo add`
version. Consumers must opt in explicitly:

```toml
quack-rs = "=0.3.0-alpha.1"
```

---

## Troubleshooting

### `validate` fails: version mismatch

The tag version does not match `Cargo.toml`. Delete the tag and re-tag after
fixing `Cargo.toml`:

```bash
git tag -d "v0.3.0"
git push origin --delete "v0.3.0"
# Fix Cargo.toml, commit, push, then re-tag
```

### `validate` fails: no CHANGELOG entry

Add a `## [0.3.0] - YYYY-MM-DD` section to `CHANGELOG.md`, commit, push, then
delete and recreate the tag.

### `validate` fails: release metadata

`scripts/check-release-metadata.py` lists each place that still names another
version (a lockfile, a CHANGELOG link, `CITATION.cff`, `SECURITY.md`, a
snippet). Run it locally, fix what it lists on `main`, then delete and recreate
the tag on the new commit (Steps 3-5).

### `security` or `osv-scan` fails: advisory found

A dependency has a known vulnerability. Either:
- Update the affected dependency, or
- Suppress it with a documented justification: in `deny.toml`'s `[advisories]
  ignore` for the root graph, in `deny.ci-crates.toml` for the CI-only graphs
  (hello-ext, fuzz, book/doctest, benches/engine), and in `osv-scanner.toml`
  for the `osv-scan` job, which reads every committed lockfile and has its own
  rules for a suppression (the file's header).

### `publish` stops before uploading: the `.crate` differs

"this job packages a different quack-rs-X.Y.Z.crate than the attested one;
nothing was published" (or "this job's rustc is ..."): crates.io has nothing,
so nothing needs undoing. The `package` job's `.crate` and the one `publish`
just built from the same commit differ, which `cargo package` reproducibility
says they must not; compare the two file lists (`tar -tzf`) before re-running.

### `publish` fails: token invalid

Regenerate the crates.io API token and update the `CARGO_REGISTRY_TOKEN`
secret of the `crates-io` environment, or finish the Trusted Publishing set-up
(Prerequisites), which needs no token.

### `publish` fails: crate already published

A re-run of `publish` after a successful upload is not a failure: the job
recognises cargo's "already exists on crates.io index" (or, from older cargo,
"already uploaded") and skips. Otherwise, crates.io versions are immutable —
you cannot overwrite a published version. Create a patch release instead
(e.g., `0.3.1`).

---

## Release artifact inventory

Every release produces and attaches:

| Artifact | Description |
|----------|-------------|
| `quack-rs-X.Y.Z.crate` | Source archive submitted to crates.io |
| `SHA256SUMS` | SHA-256 checksum of the `.crate` file |
| SLSA provenance attestation | Signed link between artifact and workflow run |
| GitHub release notes | Extracted from `CHANGELOG.md` |

The attached `.crate` is the one the `package` job built and attested. The
`publish` job packages again from the same tagged commit with the same rustc,
and refuses to upload unless that `.crate` hashes the same as `SHA256SUMS`
("The .crate about to be published is the attested one"); after the upload,
"crates.io serves the attested .crate" checks crates.io's copy against the same
file. To check by hand, compare `sha256sum` of
`https://static.crates.io/crates/quack-rs/quack-rs-X.Y.Z.crate` against
`SHA256SUMS`, or check the attached file with `gh attestation verify`.
