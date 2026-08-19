# Releasing quack-rs

Maintainer runbook for cutting a new release. Every step is documented so any
authorised maintainer can follow the process independently.

---

## Prerequisites

Before you can cut a release, ensure the following are in place:

| Requirement | Where to configure |
|-------------|-------------------|
| Push access to `main` and ability to push tags | GitHub repository settings |
| `CARGO_REGISTRY_TOKEN` secret | Settings → Secrets and variables → Actions |
| `crates-io` environment with yourself as required reviewer | Settings → Environments → crates-io |
| GPG key configured for signed tags (recommended) | `git config --global user.signingkey <KEY_ID>` |

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
git add CHANGELOG.md Cargo.toml
git commit -m "chore: release vX.Y.Z"
git push origin main

# 2. Tag (signed — requires GPG key)
git tag -s "vX.Y.Z" -m "Release vX.Y.Z"
git push origin "vX.Y.Z"

# 3. Wait for CI, then approve the crates-io deployment in the GitHub Actions UI
```

---

## Step-by-step guide

### Step 1 — Verify CI is green on `main`

All checks on `main` must pass before tagging:

```
CI / check              ✅
CI / test (Linux)       ✅
CI / test (macOS)       ✅
CI / test (Windows)     ✅
CI / clippy             ✅
CI / fmt                ✅
CI / doc                ✅
CI / MSRV (1.86.0)      ✅
CI / security           ✅
```

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

Also update the **book changelog** (`book/src/reference/changelog.md`) to mirror the
new CHANGELOG.md entry — these must stay in sync.

### Step 2b — Update SECURITY.md

If this is a **minor or major** release, update the supported versions table in
`SECURITY.md` to list the new version as supported and mark old versions
as end-of-life if appropriate.

### Step 3 — Bump the version in `Cargo.toml`

```toml
[package]
version = "0.3.0"   # ← update this
```

Do **not** update `Cargo.lock` manually — it will update itself on next build.

### Step 4 — Commit the version bump

```bash
git add CHANGELOG.md Cargo.toml
git commit -m "chore: release v0.3.0"
git push origin main
```

Wait for the CI run on `main` to pass before tagging.

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

### Step 6 — Monitor the release workflow

Navigate to **Actions → Release** in the GitHub UI. The workflow runs these jobs
in order:

```
validate          Verify tag format, Cargo.toml consistency, CHANGELOG entry
    │
    ├── ci        Full test matrix (Linux/macOS/Windows, clippy, fmt, doc, MSRV)
    └── security  cargo-deny (license policy + advisory database)
            │
            ├── package          Build .crate, generate SHA256SUMS, attest SLSA provenance
            └── publish-dry-run  cargo publish --dry-run
                    │
              github-release     Create GitHub release with notes, .crate, SHA256SUMS
                    │
                publish          ← awaits manual approval in the crates-io environment
```

Each job writes a structured summary visible in the Actions UI.

### Step 7 — Review and approve the GitHub release

When `github-release` completes, inspect the release at:

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

### Step 8 — Approve the crates.io deployment

The `publish` job is gated behind the `crates-io` environment, which requires a
manual approval from an authorised reviewer.

1. In the Actions run for the release workflow, click **Review deployments**
2. Select the `crates-io` environment
3. Click **Approve and deploy**

The job then runs `cargo publish` using the `CARGO_REGISTRY_TOKEN` secret.

### Step 9 — Post-release verification

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

For a critical bug in an already-published version:

```bash
# Branch from the release tag
git checkout -b hotfix/0.2.1 v0.2.0

# Apply the fix, update CHANGELOG and Cargo.toml
# ... make changes ...
git add -p
git commit -m "fix: <description>"

# Tag and push (follow steps 5–9 above)
git tag -s "v0.2.1" -m "Hotfix v0.2.1"
git push origin hotfix/0.2.1 "v0.2.1"

# After release: cherry-pick the fix onto main
git checkout main
git cherry-pick <fix-commit-sha>
git push origin main
```

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

### `security` fails: advisory found

A dependency has a known vulnerability. Either:
- Update the affected dependency, or
- Add the advisory ID to `deny.toml`'s `[advisories] ignore` list with a
  documented justification comment

### `publish` fails: token invalid

Regenerate the crates.io API token and update the `CARGO_REGISTRY_TOKEN`
repository secret.

### `publish` fails: crate already published

crates.io versions are immutable — you cannot overwrite a published version.
Create a patch release instead (e.g., `0.3.1`).

---

## Release artifact inventory

Every release produces and attaches:

| Artifact | Description |
|----------|-------------|
| `quack-rs-X.Y.Z.crate` | Source archive submitted to crates.io |
| `SHA256SUMS` | SHA-256 checksum of the `.crate` file |
| SLSA provenance attestation | Signed link between artifact and workflow run |
| GitHub release notes | Extracted from `CHANGELOG.md` |

The `.crate` file is identical to what crates.io serves. Download and verify it
independently with `sha256sum` or `gh attestation verify`.
