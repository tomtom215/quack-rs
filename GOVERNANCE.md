<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215) -->

# Project Governance

This document describes the governance model for the quack-rs project.

## Roles

### Contributor

Anyone who submits a pull request, files an issue, or participates in
discussions. No special permissions are required.

### Committer

Trusted contributors granted write access to the repository. Committers may
merge pull requests and triage issues. Committers are nominated by maintainers
based on sustained, high-quality contributions.

### Maintainer

Maintainers set the project's technical direction, approve architectural
changes, manage releases, and administer repository settings. Maintainers have
final decision-making authority.

**Current Maintainers:**

| Name   | GitHub        | Role               |
| ------ | ------------- | ------------------ |
| Tom F. | @tomtom215    | Initial Maintainer |

**There is one maintainer, and no other committers.** The rules below that need a
second person (a second approver, a majority vote, a tie-break) describe what happens
once there is one; today the maintainer decides. What gates a merge to `main` is the
`CI passed` check that [RELEASING.md](./RELEASING.md) configures as the required status
check, not a second reviewer.

## Decision-Making Process

The project uses **lazy consensus**:

1. Proposals (features, design changes, policy updates) are submitted as GitHub
   issues or pull requests.
2. If no objections are raised within **7 calendar days**, the proposal is
   considered accepted.
3. Trivial changes (typos, minor docs fixes) may be merged immediately by any
   committer.

### Escalation

If consensus cannot be reached:

1. The topic is discussed in a dedicated GitHub issue with all viewpoints
   documented.
2. Maintainers call for a decision with a clear deadline (minimum 7 days).
3. If disagreement persists, maintainers hold a majority vote. In the event of
   a tie, the initial maintainer casts the deciding vote.

## Code of Conduct

All participants are expected to follow the
[Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). Violations may be
reported to the maintainer at tomf@tomtomtech.net; the Code of Conduct describes
how reports are handled.

## Release Process

1. **Semantic Versioning** — All releases follow [SemVer 2.0.0](https://semver.org/).
2. **Changelog** — Every release must include an updated `CHANGELOG.md` entry
   following the [Keep a Changelog](https://keepachangelog.com/) format.
3. **Release Steps:**
   - A maintainer opens a release PR updating version numbers and the changelog.
   - The PR must pass CI. When there is more than one maintainer, it also needs the
     approval of a maintainer who did not write it; with one maintainer, the
     maintainer merges once CI is green.
   - After the PR is merged and CI on `main` passes, the maintainer pushes a
     tag (`vX.Y.Z`, GPG-signed where possible; RELEASING.md Step 5) for the
     merged commit. The tag push starts the
     release workflow (`.github/workflows/release.yml`), which validates the
     tag, re-runs CI, packages the crate and creates the GitHub release.
   - The workflow's `publish` job runs `cargo publish` only after an
     authorised reviewer approves the `crates-io` deployment environment.
     The runbook has no manual `cargo publish` step.
4. **Hotfixes** — Security and critical bug fixes may bypass the normal review
   period at a maintainer's discretion.

See [RELEASING.md](./RELEASING.md) for the complete release runbook.
