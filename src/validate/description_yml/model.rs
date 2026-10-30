// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

/// A validated representation of a `DuckDB` community extension `description.yml`.
///
/// Construct via [`parse_description_yml`] or [`validate_description_yml_str`].
///
/// Every field was validated during construction. Things the community build
/// accepts but that are probably not what the author meant — a licence outside
/// quack-rs's SPDX shortlist, a list-form `excluded_platforms`, the British
/// spelling `licence:` — do not fail the parse; they are reported in
/// [`warnings`][Self::warnings].
///
/// `#[non_exhaustive]`: new fields can be added without a breaking release, so
/// read fields by name rather than destructuring.
///
/// [`parse_description_yml`]: super::parse_description_yml
/// [`validate_description_yml_str`]: super::validate_description_yml_str
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DescriptionYml {
    // extension section
    /// Extension name (validated: a lowercase letter, then lowercase letters,
    /// digits and underscores).
    pub name: String,
    /// One-line description of the extension.
    pub description: String,
    /// Extension version, when the file declares one (validated: see
    /// [`validate_extension_version`][crate::validate::validate_extension_version]).
    ///
    /// Optional: the community build never reads it (`scripts/build.py` in
    /// `duckdb/community-extensions` takes the version from the extension's
    /// own repository), and 12 of the 346 published files omit it.
    pub version: Option<String>,
    /// Implementation language (for Rust extensions: `"Rust"`).
    pub language: String,
    /// Build system (for Rust extensions: `"cargo"`).
    pub build: String,
    /// The declared license — an SPDX identifier or expression when the file
    /// follows the convention. Only presence is enforced; see
    /// [`warnings`][Self::warnings].
    pub license: String,
    /// Semicolon-separated required toolchains (must include `"rust"` for Rust extensions).
    pub requires_toolchains: String,
    /// Platforms to exclude from CI builds, semicolon-separated. Empty means no
    /// exclusions. A YAML list in the file is joined with `;` here (and warned
    /// about: the community build passes a list through unsplit).
    pub excluded_platforms: String,
    /// Opt-in platforms to build, semicolon-separated (`extension.opt_in_platforms`).
    /// Empty when absent.
    ///
    /// The community build only builds the platforms in
    /// [`DUCKDB_OPT_IN_PLATFORMS`][crate::validate::DUCKDB_OPT_IN_PLATFORMS] when
    /// they are listed here. A name that is not one of them, or that is not a
    /// platform at all, is ignored by the build and reported in
    /// [`warnings`][Self::warnings].
    pub opt_in_platforms: String,
    /// List of maintainer names (at least one required).
    pub maintainers: Vec<String>,
    // repo section
    /// GitHub repository in `owner/repo` format.
    pub github: String,
    /// The git ref the community build checks out: a commit hash. A branch or
    /// tag is accepted with a warning in [`DescriptionYml::warnings`].
    pub git_ref: String,
    /// `repo.ref_next` — a revision compatible with `DuckDB`'s `main` branch,
    /// empty when absent.
    ///
    /// Optional, and documented by `DuckDB` for the window while a new release
    /// is being prepared: the community repository tests an extension against
    /// both the latest stable release and `main`, and once the release hash is
    /// set `ref_next` is swapped in for `ref`.
    pub git_ref_next: String,
    /// `repo.variegata` — the revision the community build checks out when it
    /// builds for `DuckDB` v1.5.6 (the `v1.5-variegata` release line) instead
    /// of [`git_ref`][Self::git_ref]; empty when absent. 133 of the 395
    /// published descriptors set it.
    ///
    /// `ref` is built for the newest release (v2.0.0). An extension stamped
    /// for one exact `DuckDB` 1.x release (`C_STRUCT_UNSTABLE`) needs its
    /// v1.5.6 build here, from a revision whose `Makefile` targets v1.5.6.
    pub git_ref_variegata: String,
    /// `repo.andium` — as [`git_ref_variegata`][Self::git_ref_variegata], for
    /// `DuckDB` v1.4.5 (the `v1.4-andium` line); empty when absent.
    pub git_ref_andium: String,
    /// Non-fatal findings, one human-readable sentence each: the file is
    /// accepted, but something in it is unusual enough to be worth a look.
    pub warnings: Vec<String>,
}
