// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `DuckDB` C Extension API ABI compatibility checking.
//!
//! # Why this module exists
//!
//! A loadable extension does not link against `DuckDB`'s symbols. Instead,
//! `DuckDB` hands the extension a pointer to a `duckdb_ext_api_v1` struct — an
//! array of function pointers — and the extension calls through it. The
//! extension's *compiled-in* idea of that struct's layout comes from the
//! `duckdb_extension.h` shipped with the `libduckdb-sys` version Cargo resolved
//! at build time. `DuckDB`'s idea of the layout comes from whatever `DuckDB`
//! binary is doing the loading. **If those two layouts disagree, every call
//! lands on the wrong function pointer.**
//!
//! `duckdb_ext_api_v1` is split into two regions:
//!
//! | Region | Slots | Guarantee |
//! |--------|-------|-----------|
//! | Stable | <code>0 .. [STABLE_API_SLOT_COUNT]</code> | Frozen since `DuckDB` v1.2.0. Byte-for-byte identical in every release from v1.2.0 through v1.5.5. |
//! | Unstable | <code>[STABLE_API_SLOT_COUNT] ..</code> | `DuckDB` **inserts new entries in the middle**, shifting every later slot. |
//!
//! The unstable region is where `duckdb-1-5` lives: scalar bind/init, copy
//! functions, catalog access, `ErrorData`, `FileSystem`, `Expression`,
//! `SelectionVector`, config options, table descriptions, `TIME_NS` values, and
//! the client context all sit past the stable boundary.
//!
//! `DuckDB` inserted entries into the middle of the unstable region in **four
//! of the last four** minor/patch families:
//!
//! | `DuckDB` | Total slots | What changed |
//! |----------|-------------|--------------|
//! | v1.2.0 – v1.2.2 | 408 | baseline |
//! | v1.3.0 – v1.3.2 | 428 | appended |
//! | v1.4.0 – v1.4.4 | 459 | `duckdb_create_varint` → `duckdb_create_bignum`, appended |
//! | v1.5.0 – v1.5.1 | 545 | `duckdb_appender_clear` **inserted** at slot 410 |
//! | v1.5.2 – v1.5.5 | 546 | `duckdb_geometry_type_get_crs` **inserted** at slot 493 |
//!
//! # Why `DuckDB` does not catch this for you
//!
//! `DuckDB`'s loader validates the extension footer's ABI metadata:
//!
//! - `C_STRUCT` + a C API version (`v1.2.0`) — accepted by *any* `DuckDB` whose
//!   C API version is greater than or equal to it, then handed the **whole**
//!   struct including the unstable region. No engine-version check at all.
//! - `C_STRUCT_UNSTABLE` + a `DuckDB` release version — accepted only by that
//!   exact `DuckDB` release.
//!
//! An extension that touches the unstable region but is stamped `C_STRUCT`
//! therefore loads happily into a `DuckDB` with a different layout and then
//! mis-dispatches. For example, an extension built against `DuckDB` v1.5.0 and
//! loaded into v1.5.2+ would call `duckdb_scalar_function_set_bind_data(info,
//! data, destroy)` and actually invoke
//! `duckdb_scalar_function_get_client_context(info)`.
//!
//! # What this module does
//!
//! [`check`] compares the slot count of the layout this extension was *compiled*
//! against with the slot count of the layout the *running* `DuckDB` uses (looked
//! up from `duckdb_library_version()`, which lives at stable slot 7 and is
//! therefore always safe to call). A mismatch is reported as
//! [`AbiCheck::LayoutMismatch`] instead of being allowed to corrupt memory.
//!
//! [`init_extension`][crate::entry_point::init_extension] and
//! [`init_extension_v2`][crate::entry_point::init_extension_v2] run this check
//! automatically under [`AbiPolicy::Strict`], which is the default whenever the
//! `duckdb-1-5` feature is enabled. Extensions that only use the stable region
//! are unaffected and stay portable across every `DuckDB` release.
//!
//! # A caveat that is not ours to fix
//!
//! `libduckdb-sys` generates its `duckdb_ext_api_v1` with
//! `DUCKDB_EXTENSION_API_VERSION_UNSTABLE` defined, so the Rust struct always
//! has every slot regardless of which quack-rs features are on, and
//! `duckdb_rs_extension_api_init` copies all of them out of the struct `DuckDB`
//! provides. Loading into an *older* `DuckDB` whose struct has fewer slots
//! therefore reads a few bytes past its end. The pointers it lands on are only
//! ever stored, never called, unless the extension actually uses the unstable
//! region — which is what [`check`] is for — but the read itself is an
//! upstream detail of `libduckdb-sys`, not something this crate can prevent.
//!
//! # Shipping an extension that uses the unstable API
//!
//! Use `USE_UNSTABLE_C_API=1` with `extension-ci-tools` (or
//! `--abi-type C_STRUCT_UNSTABLE --duckdb-version vX.Y.Z` with
//! [`append_metadata`][crate::validate]), so `DuckDB` itself refuses to load the
//! binary into the wrong release. The runtime guard in this module is the
//! belt-and-braces for builds where that metadata is missing or wrong — which
//! includes every `LOAD '/path/to/ext.duckdb_extension'` during development.

use core::ffi::c_void;
use core::mem::size_of;

use libduckdb_sys::duckdb_ext_api_v1;

/// Number of function-pointer slots in the **stable** prefix of
/// `duckdb_ext_api_v1`.
///
/// These slots have been byte-for-byte identical — same functions, same order —
/// in every `DuckDB` release from v1.2.0 through v1.5.5. An extension that only
/// calls into this prefix is portable across all of them.
pub const STABLE_API_SLOT_COUNT: usize = 357;

/// A `DuckDB` release family and the `duckdb_ext_api_v1` slot count it uses.
///
/// `(major, minor, patch_min, patch_max, slots)` — `patch_min..=patch_max` is
/// the inclusive range of patch releases verified to share `slots`.
type LayoutEntry = (u64, u64, u64, u64, usize);

/// Verified `DuckDB` release → `duckdb_ext_api_v1` slot-count table.
///
/// Derived directly from `src/include/duckdb_extension.h` at each release tag,
/// with `DUCKDB_EXTENSION_API_VERSION_UNSTABLE` defined (which is what
/// `libduckdb-sys` does when generating the `loadable-extension` bindings, and
/// what `DuckDB` itself does when building the struct it hands out).
///
/// Regenerate with `scripts/check-abi-table.py`, which re-downloads every
/// release header and fails if this table drifts.
///
/// Within this range the slot count uniquely identifies the layout: no two
/// releases share a slot count while ordering their entries differently.
const KNOWN_LAYOUTS: &[LayoutEntry] = &[
    (1, 2, 0, 2, 408),
    (1, 3, 0, 2, 428),
    (1, 4, 0, 4, 459),
    (1, 5, 0, 1, 545),
    (1, 5, 2, 5, 546),
];

/// The result of an ABI layout check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiCheck {
    /// The running `DuckDB` uses the same `duckdb_ext_api_v1` layout this
    /// extension was compiled against. All calls dispatch correctly.
    Compatible {
        /// The `duckdb_library_version()` string reported by the engine.
        engine_version: String,
        /// The slot count shared by both sides.
        slots: usize,
    },
    /// The extension only reaches into the stable prefix, so the layout of the
    /// unstable region is irrelevant. Always portable.
    StableOnly,
    /// The running `DuckDB` uses a **different** layout. Calling into the
    /// unstable region would invoke the wrong function pointers.
    LayoutMismatch {
        /// The `duckdb_library_version()` string reported by the engine.
        engine_version: String,
        /// Slot count the running engine uses.
        engine_slots: usize,
        /// Slot count this extension was compiled against.
        compiled_slots: usize,
    },
    /// The running `DuckDB` reports a version this build of `quack-rs` has no
    /// verified layout for — a newer release, or a `-dev` build.
    ///
    /// This is *not* evidence that the layout matches. `DuckDB` has changed the
    /// unstable region in every minor and in some patch releases, so an unknown
    /// version must be treated as potentially incompatible.
    UnknownEngineVersion {
        /// The `duckdb_library_version()` string reported by the engine.
        engine_version: String,
        /// Slot count this extension was compiled against.
        compiled_slots: usize,
    },
    /// `duckdb_library_version()` returned a null or non-UTF-8 pointer.
    EngineVersionUnavailable {
        /// Slot count this extension was compiled against.
        compiled_slots: usize,
    },
}

impl AbiCheck {
    /// Returns `true` if it is safe to call into the unstable API region.
    #[must_use]
    pub const fn is_compatible(&self) -> bool {
        matches!(self, Self::Compatible { .. } | Self::StableOnly)
    }

    /// Returns a diagnostic message suitable for `access.set_error`, or `None`
    /// when the check passed.
    #[must_use]
    pub fn error_message(&self) -> Option<String> {
        match self {
            Self::Compatible { .. } | Self::StableOnly => None,
            Self::LayoutMismatch {
                engine_version,
                engine_slots,
                compiled_slots,
            } => Some(format!(
                "DuckDB C extension API layout mismatch: this extension was built against a \
                 duckdb_ext_api_v1 with {compiled_slots} slots, but DuckDB {engine_version} \
                 provides {engine_slots}. The extension uses the unstable region of the C API \
                 (quack-rs feature `duckdb-1-5`), whose slot indices differ between these \
                 releases, so loading it would dispatch to the wrong functions. Rebuild the \
                 extension against DuckDB {engine_version}, and stamp it with \
                 `--abi-type C_STRUCT_UNSTABLE --duckdb-version {engine_version}` \
                 (or `USE_UNSTABLE_C_API=1` with extension-ci-tools) so this is caught at \
                 install time."
            )),
            Self::UnknownEngineVersion {
                engine_version,
                compiled_slots,
            } => Some(format!(
                "DuckDB C extension API layout cannot be verified: DuckDB reports version \
                 '{engine_version}', which this build of quack-rs has no verified \
                 duckdb_ext_api_v1 layout for (this extension was built against a \
                 {compiled_slots}-slot layout). The extension uses the unstable region of the \
                 C API (quack-rs feature `duckdb-1-5`), and DuckDB has changed that region's \
                 layout in every recent release, so loading is refused rather than risking \
                 mis-dispatch. Rebuild against DuckDB {engine_version}, or opt out with \
                 `AbiPolicy::Trust` if you have verified the layout yourself."
            )),
            Self::EngineVersionUnavailable { compiled_slots } => Some(format!(
                "DuckDB C extension API layout cannot be verified: duckdb_library_version() \
                 returned no usable version string (this extension was built against a \
                 {compiled_slots}-slot layout). Refusing to call into the unstable region of \
                 the C API."
            )),
        }
    }
}

/// Number of `duckdb_ext_api_v1` slots this extension was **compiled** against.
///
/// Determined from the size of the `duckdb_ext_api_v1` struct in the resolved
/// `libduckdb-sys` bindings. Every field is a nullable function pointer, so the
/// struct is exactly `slots * size_of::<*const c_void>()` bytes.
#[must_use]
#[inline]
pub const fn compiled_slot_count() -> usize {
    size_of::<duckdb_ext_api_v1>() / size_of::<*const c_void>()
}

/// Returns `true` if this build of `quack-rs` can reach past the stable prefix.
///
/// Only the `duckdb-1-5` / `duckdb-1-5-3` feature set wraps functions in the
/// unstable region. Without those features the extension is portable across
/// every `DuckDB` release that accepts C API `v1.2.0`.
#[must_use]
#[inline]
pub const fn uses_unstable_api() -> bool {
    cfg!(feature = "duckdb-1-5")
}

/// Parses a `DuckDB` version string into `(major, minor, patch)`.
///
/// Accepts `"v1.5.4"` and `"1.5.4"`. Returns `None` for anything carrying a
/// pre-release or build suffix (`"v1.6.0-dev1234"`), because the layout of a
/// development build is not knowable from the version string alone.
#[must_use]
pub fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let trimmed = version.strip_prefix('v').unwrap_or(version);
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Returns the verified `duckdb_ext_api_v1` slot count for a `DuckDB` release,
/// or `None` if this build has no verified entry for it.
///
/// # Example
///
/// ```rust
/// use quack_rs::abi::expected_slot_count;
///
/// assert_eq!(expected_slot_count("v1.5.4"), Some(546));
/// assert_eq!(expected_slot_count("v1.5.0"), Some(545));
/// assert_eq!(expected_slot_count("v1.4.4"), Some(459));
/// // Unreleased / unknown versions are explicitly not guessed at.
/// assert_eq!(expected_slot_count("v9.9.9"), None);
/// assert_eq!(expected_slot_count("v1.6.0-dev42"), None);
/// ```
#[must_use]
pub fn expected_slot_count(version: &str) -> Option<usize> {
    let (major, minor, patch) = parse_version(version)?;
    KNOWN_LAYOUTS
        .iter()
        .find(|&&(ma, mi, lo, hi, _)| ma == major && mi == minor && patch >= lo && patch <= hi)
        .map(|&(_, _, _, _, slots)| slots)
}

/// Returns the version string reported by the running `DuckDB`.
///
/// Calls `duckdb_library_version()`, which occupies stable slot 7 and is
/// therefore dispatched correctly no matter how the unstable region is laid
/// out.
///
/// # Safety
///
/// The `DuckDB` C API dispatch table must already have been initialised — that
/// is, `duckdb_rs_extension_api_init` must have returned successfully. Calling
/// this before initialisation panics inside `libduckdb-sys`.
#[must_use]
pub unsafe fn engine_version() -> Option<String> {
    // SAFETY: the caller guarantees the dispatch table is initialised.
    let ptr = unsafe { libduckdb_sys::duckdb_library_version() };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: DuckDB returns a static NUL-terminated string owned by the engine.
    let cstr = unsafe { core::ffi::CStr::from_ptr(ptr) };
    cstr.to_str().ok().map(str::to_owned)
}

/// Checks whether the running `DuckDB` uses the same `duckdb_ext_api_v1` layout
/// this extension was compiled against.
///
/// Returns [`AbiCheck::StableOnly`] immediately when the crate was built without
/// the `duckdb-1-5` feature, since the stable prefix is frozen and no check is
/// needed.
///
/// # Safety
///
/// The `DuckDB` C API dispatch table must already have been initialised — see
/// [`engine_version`].
#[must_use]
pub unsafe fn check() -> AbiCheck {
    let compiled_slots = compiled_slot_count();

    if !uses_unstable_api() {
        return AbiCheck::StableOnly;
    }

    // SAFETY: forwarded from this function's own contract.
    let Some(engine_version) = (unsafe { engine_version() }) else {
        return AbiCheck::EngineVersionUnavailable { compiled_slots };
    };

    match expected_slot_count(&engine_version) {
        Some(engine_slots) if engine_slots == compiled_slots => AbiCheck::Compatible {
            engine_version,
            slots: compiled_slots,
        },
        Some(engine_slots) => AbiCheck::LayoutMismatch {
            engine_version,
            engine_slots,
            compiled_slots,
        },
        None => AbiCheck::UnknownEngineVersion {
            engine_version,
            compiled_slots,
        },
    }
}

/// How [`init_extension`][crate::entry_point::init_extension] should react to
/// the ABI layout check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AbiPolicy {
    /// Refuse to load unless the layout is verified compatible.
    ///
    /// This is the default. It converts what would otherwise be silent
    /// mis-dispatch into a clear `LOAD` error naming the exact remedy.
    #[default]
    Strict,
    /// Perform the check and, on failure, print the diagnostic to stderr but
    /// continue loading.
    ///
    /// Note the channel: this cannot use `access.set_error`, because `DuckDB`'s
    /// loader throws whenever an extension called `set_error` — regardless of
    /// what the entry point returned — which would make `Warn` behave exactly
    /// like [`Strict`][Self::Strict]. The C extension API has no non-fatal
    /// diagnostic channel, so the message goes to stderr.
    ///
    /// Use this when the crate is built with `duckdb-1-5` but the extension
    /// provably calls only stable-region wrappers, and you want the mismatch
    /// visible without failing the load. **Calling into the unstable region
    /// after a failed check is undefined behaviour** — `Warn` does not make it
    /// safe, it only makes it loud.
    Warn,
    /// Skip the check entirely.
    ///
    /// Appropriate when the binary is stamped `C_STRUCT_UNSTABLE`, because
    /// `DuckDB` then refuses to load it into any release other than the one it
    /// was built for — making the runtime check redundant. Also appropriate when
    /// the `duckdb-1-5` feature is enabled but the extension provably calls only
    /// stable-region wrappers.
    Trust,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_size_is_a_whole_number_of_pointers() {
        assert_eq!(
            size_of::<duckdb_ext_api_v1>() % size_of::<*const c_void>(),
            0
        );
    }

    #[test]
    fn compiled_layout_is_one_we_have_verified() {
        let slots = compiled_slot_count();
        assert!(
            KNOWN_LAYOUTS.iter().any(|&(_, _, _, _, s)| s == slots),
            "libduckdb-sys resolved to a duckdb_ext_api_v1 with {slots} slots, which is not in \
             KNOWN_LAYOUTS. Re-run scripts/check-abi-table.py and add the new DuckDB release."
        );
    }

    #[test]
    fn stable_prefix_fits_inside_every_known_layout() {
        for &(_, _, _, _, slots) in KNOWN_LAYOUTS {
            assert!(slots > STABLE_API_SLOT_COUNT);
        }
    }

    #[test]
    fn known_layouts_are_sorted_and_non_overlapping() {
        let mut prev: Option<(u64, u64, u64)> = None;
        for &(ma, mi, lo, hi, _) in KNOWN_LAYOUTS {
            assert!(lo <= hi, "patch range {lo}..={hi} is inverted");
            let key = (ma, mi, lo);
            if let Some(p) = prev {
                assert!(p < key, "KNOWN_LAYOUTS must be sorted: {p:?} !< {key:?}");
            }
            prev = Some(key);
        }
    }

    #[test]
    fn slot_count_uniquely_identifies_a_layout() {
        // Two release families may not claim the same slot count, otherwise the
        // check in `check()` could pass for a layout it has not verified.
        let mut seen: Vec<usize> = KNOWN_LAYOUTS.iter().map(|&(.., s)| s).collect();
        seen.sort_unstable();
        let len_before = seen.len();
        seen.dedup();
        assert_eq!(
            len_before,
            seen.len(),
            "duplicate slot counts in KNOWN_LAYOUTS"
        );
    }

    #[test]
    fn parses_version_strings() {
        assert_eq!(parse_version("v1.5.4"), Some((1, 5, 4)));
        assert_eq!(parse_version("1.5.4"), Some((1, 5, 4)));
        assert_eq!(parse_version("v1.5"), None);
        assert_eq!(parse_version("v1.5.4.1"), None);
        assert_eq!(parse_version("v1.6.0-dev1234"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("nonsense"), None);
    }

    #[test]
    fn maps_every_documented_release_family() {
        for (version, slots) in [
            ("v1.2.0", 408),
            ("v1.2.2", 408),
            ("v1.3.0", 428),
            ("v1.3.2", 428),
            ("v1.4.0", 459),
            ("v1.4.4", 459),
            ("v1.5.0", 545),
            ("v1.5.1", 545),
            ("v1.5.2", 546),
            ("v1.5.5", 546),
        ] {
            assert_eq!(expected_slot_count(version), Some(slots), "for {version}");
        }
    }

    #[test]
    fn does_not_extrapolate_beyond_verified_patches() {
        // v1.4.5 has not been released and its layout is unknown; guessing would
        // defeat the purpose of the guard.
        assert_eq!(expected_slot_count("v1.4.5"), None);
        assert_eq!(expected_slot_count("v1.5.6"), None);
        assert_eq!(expected_slot_count("v1.6.0"), None);
        assert_eq!(expected_slot_count("v2.0.0"), None);
    }

    #[test]
    fn compatible_and_stable_only_are_the_only_passing_states() {
        assert!(AbiCheck::StableOnly.is_compatible());
        assert!(AbiCheck::Compatible {
            engine_version: "v1.5.4".into(),
            slots: 546
        }
        .is_compatible());
        assert!(!AbiCheck::LayoutMismatch {
            engine_version: "v1.5.0".into(),
            engine_slots: 545,
            compiled_slots: 546
        }
        .is_compatible());
        assert!(!AbiCheck::UnknownEngineVersion {
            engine_version: "v9.9.9".into(),
            compiled_slots: 546
        }
        .is_compatible());
        assert!(!AbiCheck::EngineVersionUnavailable {
            compiled_slots: 546
        }
        .is_compatible());
    }

    #[test]
    fn passing_states_have_no_error_message() {
        assert!(AbiCheck::StableOnly.error_message().is_none());
        assert!(AbiCheck::Compatible {
            engine_version: "v1.5.4".into(),
            slots: 546
        }
        .error_message()
        .is_none());
    }

    #[test]
    fn mismatch_message_names_both_layouts_and_the_remedy() {
        let msg = AbiCheck::LayoutMismatch {
            engine_version: "v1.5.0".into(),
            engine_slots: 545,
            compiled_slots: 546,
        }
        .error_message()
        .expect("mismatch must produce a message");
        assert!(msg.contains("546"));
        assert!(msg.contains("545"));
        assert!(msg.contains("v1.5.0"));
        assert!(msg.contains("C_STRUCT_UNSTABLE"));
    }

    #[test]
    fn unknown_version_message_names_the_version_and_the_opt_out() {
        let msg = AbiCheck::UnknownEngineVersion {
            engine_version: "v1.6.0".into(),
            compiled_slots: 546,
        }
        .error_message()
        .expect("unknown version must produce a message");
        assert!(msg.contains("v1.6.0"));
        assert!(msg.contains("AbiPolicy::Trust"));
    }

    #[test]
    fn unavailable_version_message_mentions_the_missing_probe() {
        let msg = AbiCheck::EngineVersionUnavailable {
            compiled_slots: 546,
        }
        .error_message()
        .expect("unavailable version must produce a message");
        assert!(msg.contains("duckdb_library_version"));
    }

    #[test]
    fn default_policy_is_strict() {
        assert_eq!(AbiPolicy::default(), AbiPolicy::Strict);
    }

    #[test]
    fn unstable_api_flag_tracks_the_feature() {
        assert_eq!(uses_unstable_api(), cfg!(feature = "duckdb-1-5"));
    }
}

/// End-to-end checks that need a live `DuckDB` dispatch table.
///
/// `InMemoryDb::open()` initialises the `loadable-extension` dispatch table from
/// the linked `DuckDB`, which is exactly the state an extension is in after
/// `duckdb_rs_extension_api_init` succeeds — so these exercise the real probe
/// rather than a stub.
#[cfg(all(test, feature = "_duckdb-testing"))]
mod live_tests {
    use super::*;

    #[test]
    fn engine_version_is_readable_and_parses() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        let version = unsafe { engine_version() }.expect("duckdb_library_version()");
        assert!(
            parse_version(&version).is_some(),
            "engine reported an unparseable version: {version:?}"
        );
    }

    #[test]
    fn linked_engine_layout_matches_the_compiled_bindings() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        let version = unsafe { engine_version() }.expect("duckdb_library_version()");
        assert_eq!(
            expected_slot_count(&version),
            Some(compiled_slot_count()),
            "KNOWN_LAYOUTS says DuckDB {version} has a different duckdb_ext_api_v1 slot count \
             than the libduckdb-sys bindings this crate compiled against ({} slots). Re-run \
             scripts/check-abi-table.py.",
            compiled_slot_count()
        );
    }

    #[test]
    fn check_reports_compatibility_against_the_linked_engine() {
        let _db = crate::testing::InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        let result = unsafe { check() };
        assert!(
            result.is_compatible(),
            "ABI check failed against the linked DuckDB: {result:?}"
        );
        assert!(result.error_message().is_none());
    }
}
