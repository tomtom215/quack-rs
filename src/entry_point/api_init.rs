// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Filling `libduckdb-sys`'s dispatch table without reading past the end of the
//! engine's `duckdb_ext_api_v1`.
//!
//! # The problem
//!
//! `libduckdb_sys::duckdb_rs_extension_api_init` asks `DuckDB` for its API struct
//! through `access.get_api`, then reads **every field of the struct it was
//! compiled against** through the returned pointer. `DuckDB` sizes that struct by
//! its own release (459 slots on 1.4.x, 545 on 1.5.0 – 1.5.1, 546 on 1.5.2 –
//! 1.5.6, 548 on 2.0.0), so a build against the 546-slot bindings loaded into
//! 1.4.x reads 87 pointers, 696 bytes, past the end of it. The mapped memory
//! after the struct made this harmless in practice, but a read outside the object
//! `get_api` returned is undefined behaviour; Miri reports it on any exactly sized
//! table. See [`crate::abi`] for the layouts and why the layout check cannot run
//! first: it reaches `duckdb_library_version` through this very table.
//!
//! # What this module does instead
//!
//! [`init_dispatch_table`] gives the bindings a different `get_api`. It calls the
//! real one, reads the engine's version through **slot 7**
//! (`duckdb_library_version`, inside the stable prefix every engine that accepts a
//! C API `v1.2.0` extension provides, so safe to read before anything is known
//! about the layout), works out how many slots the engine's table has, and returns
//! a zero-padded copy of that many, as long as the bindings' struct. The bindings
//! read inside that copy; a slot the engine does not have is `NULL`, which the
//! bindings skip, so its wrapper panics ("not initialised", caught like any other
//! panic in a callback) instead of jumping to whatever followed the table.
//!
//! How many slots the engine has, in order:
//!
//! 1. The verified layout table ([`abi::expected_slot_count`]) knows the engine's
//!    release: that count.
//! 2. The build declared it was made against exactly this release
//!    ([`abi::built_against_version`], `QUACK_RS_BUILT_AGAINST_DUCKDB`): the
//!    compiled struct is the engine's, so all of it.
//! 3. Neither, an unknown release. Under [`AbiPolicy::Strict`], what `DuckDB`
//!    guarantees for the C API version the extension asked `get_api` for: the
//!    546 slots of C API v1.5.6 for v1.5.6 or later (its header declares slots
//!    404 – 545 under `DUCKDB_API_VERSION_AT_LEAST(1, 5, 6)`, and `get_api` hands
//!    a struct out for that version only on an engine that has them), and the
//!    357-slot stable prefix for anything older. A build that calls beyond that is
//!    refused on an unknown engine anyway, and one that does not never reads
//!    beyond it. Under any other policy
//!    the whole compiled struct, which is what the bindings always did: those
//!    policies exist so an extension can run on a release quack-rs has not
//!    catalogued (`Trust` with a `C_STRUCT_UNSTABLE` binary, `AllowUnknownEngine`),
//!    and it may legitimately call an unstable slot there.
//!
//! A layout that *inserts* a slot (v1.5.0 – 1.5.1 against v1.5.2 and later) still
//! mis-dispatches, padded or not; that is what the layout check refuses, and
//! padding does not change it.
//!
//! # Ownership and unwinding
//!
//! The padded copy lives in a thread-local owned by this module and is freed when
//! [`init_dispatch_table`] returns; the bindings copy the function pointers out
//! and keep no reference. Nothing here panics: every pointer is checked, the one
//! allocation aborts rather than unwinds if it fails, and the only `RefCell` is
//! borrowed once at a time on the thread that owns it. The `extern "C"`
//! function therefore cannot unwind into `DuckDB`.

use core::cell::{Cell, RefCell};
use core::ffi::{c_char, c_void, CStr};
use core::mem::{offset_of, size_of, transmute};

use libduckdb_sys::{
    duckdb_ext_api_v1, duckdb_extension_access, duckdb_extension_info, duckdb_rs_extension_api_init,
};

use crate::abi::{self, AbiPolicy, STABLE_API_SLOT_COUNT};

/// One slot of `duckdb_ext_api_v1`. Every field of the struct is a nullable
/// function pointer, so the struct is an array of these.
type Slot = Option<unsafe extern "C" fn()>;

/// The slot of `duckdb_library_version`: eighth of the stable prefix, in every
/// release from v1.2.0.
const LIBRARY_VERSION_SLOT: usize = 7;

/// The slots `DuckDB` promises an extension that asks for C API v1.5.6 or later:
/// all of `duckdb_ext_api_v1` as v1.5.6 declares it (slots 404 – 545 are under
/// `DUCKDB_API_VERSION_AT_LEAST(1, 5, 6)` in v1.5.6's, v2.0.0's and v2.0.1's
/// `duckdb_extension.h`; v2.0.x appends two more under the same guard).
const C_API_1_5_6_SLOT_COUNT: usize = 546;

/// What an engine whose release quack-rs does not know is guaranteed to hand an
/// extension that asked for C API `api_version`, the fallback under
/// [`AbiPolicy::Strict`]. An unparseable version falls back to the v1.2.0 prefix.
fn guaranteed_slots_for(api_version: &str) -> usize {
    match abi::parse_version(api_version) {
        Some(version) if version >= (1, 5, 6) => C_API_1_5_6_SLOT_COUNT,
        _ => STABLE_API_SLOT_COUNT,
    }
}

// The bindings are one slot-sized field after another, and the version function
// is where this module believes it is. A bindings release that broke either would
// otherwise read the wrong memory without a diagnostic.
const _: () = {
    assert!(size_of::<Slot>() == size_of::<*const c_void>());
    assert!(size_of::<duckdb_ext_api_v1>() % size_of::<Slot>() == 0);
    assert!(
        offset_of!(duckdb_ext_api_v1, duckdb_library_version)
            == LIBRARY_VERSION_SLOT * size_of::<Slot>()
    );
};

/// What [`bounded_get_api`] needs from the call it runs inside.
#[derive(Clone, Copy)]
struct Context {
    /// The `access` `DuckDB` passed to the entry point.
    access: *const duckdb_extension_access,
    /// Slots to take from an engine whose release is neither in the layout table
    /// nor declared by the build.
    unknown_slots: usize,
}

thread_local! {
    /// Set for the duration of one [`init_dispatch_table`] call, on its thread:
    /// the bindings call `bounded_get_api` synchronously, and the C function
    /// pointer it is installed as has no room for a closure's state.
    static CONTEXT: Cell<Option<Context>> = const { Cell::new(None) };
    /// The padded copy handed to the bindings. Owned here so it is freed with
    /// the call that needed it; the pointer returned from `bounded_get_api` stays
    /// valid until [`init_dispatch_table`] returns.
    static PADDED: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
}

/// Clears [`CONTEXT`] and frees [`PADDED`] when [`init_dispatch_table`] ends,
/// however it ends.
struct Active;

impl Drop for Active {
    fn drop(&mut self) {
        // `try_with`: a thread-local already destroyed at thread exit is not
        // worth a panic from a destructor.
        let _ = CONTEXT.try_with(|context| context.set(None));
        let _ = PADDED.try_with(|padded| *padded.borrow_mut() = Vec::new());
    }
}

/// Populates `libduckdb-sys`'s process-wide dispatch table from the engine
/// behind `access`, reading no further than the engine's own struct.
///
/// Returns what `duckdb_rs_extension_api_init` returns: `Ok(false)` when the
/// engine has no API for `api_version`, an error for a version string the
/// bindings refuse.
///
/// # Safety
///
/// - `access` must be non-null, aligned, and point at the live
///   `duckdb_extension_access` `DuckDB` passed to the entry point, with
///   `get_api` set, for the whole call.
/// - `info` is the pointer `DuckDB` passed alongside it.
/// - `api_version` contains no interior NUL (the bindings unwrap the conversion).
pub(super) unsafe fn init_dispatch_table(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    policy: AbiPolicy,
) -> Result<bool, &'static str> {
    let unknown_slots = if policy == AbiPolicy::Strict {
        guaranteed_slots_for(api_version)
    } else {
        abi::compiled_slot_count()
    };
    CONTEXT.with(|context| {
        context.set(Some(Context {
            access,
            unknown_slots,
        }));
    });
    let _active = Active;

    // SAFETY: `access` is valid per this function's contract. The copy is a
    // plain-data struct of three nullable function pointers; only `get_api`
    // changes.
    let mut bounded = unsafe { *access };
    bounded.get_api = Some(bounded_get_api);

    // SAFETY: `bounded` outlives the call. `info` is DuckDB's, passed through;
    // `bounded_get_api` forwards it to the real `get_api` untouched.
    unsafe { duckdb_rs_extension_api_init(info, &raw const bounded, api_version) }
}

/// The `get_api` the bindings call: `DuckDB`'s own, then a zero-padded copy of as
/// much of its table as the engine has.
///
/// Returns null when the real `get_api` does (the bindings turn that into
/// `Ok(false)`), and when called outside [`init_dispatch_table`].
///
/// # Safety
///
/// Called by `duckdb_rs_extension_api_init` with the arguments it would have
/// passed to `DuckDB`'s `get_api`; only meaningful inside [`init_dispatch_table`].
unsafe extern "C" fn bounded_get_api(
    info: duckdb_extension_info,
    version: *const c_char,
) -> *const c_void {
    let Ok(Some(context)) = CONTEXT.try_with(Cell::get) else {
        return core::ptr::null();
    };
    // SAFETY: `context.access` is the pointer `init_dispatch_table` was given,
    // valid for the whole call that is running this function.
    let Some(real_get_api) = (unsafe { (*context.access).get_api }) else {
        return core::ptr::null();
    };
    // SAFETY: DuckDB's own function, called with the arguments the bindings
    // passed for it.
    let table = unsafe { real_get_api(info, version) }.cast::<Slot>();
    if table.is_null() {
        return core::ptr::null();
    }

    // SAFETY: `table` is the engine's `duckdb_ext_api_v1`, at least as long as
    // the stable prefix (see `engine_table_slots_at`).
    let engine_slots = unsafe { engine_table_slots_at(table, context.unknown_slots) };
    let compiled = abi::compiled_slot_count();
    let copied = engine_slots.min(compiled);

    PADDED
        .try_with(|padded| {
            let mut padded = padded.borrow_mut();
            padded.clear();
            padded.resize(compiled, None);
            // SAFETY: `table` is readable for `engine_slots >= copied` slots
            // (the engine's own struct, or the stable prefix); `padded` holds
            // `compiled >= copied` slots; the two are separate allocations.
            unsafe { core::ptr::copy_nonoverlapping(table, padded.as_mut_ptr(), copied) };
            padded.as_ptr().cast::<c_void>()
        })
        .unwrap_or(core::ptr::null())
}

/// How many slots of the engine's table at `table` may be read.
///
/// # Safety
///
/// `table` must point at an engine's `duckdb_ext_api_v1` that holds at least the
/// stable prefix: `DuckDB` hands that struct to any extension that asks for a C
/// API version of `v1.2.0` or later, and the prefix is the part of it that has
/// not changed since. Slot 7 lies inside it.
unsafe fn engine_table_slots_at(table: *const Slot, unknown_slots: usize) -> usize {
    // SAFETY: slot 7 is within the stable prefix (this function's contract).
    let version_fn = unsafe { table.add(LIBRARY_VERSION_SLOT).read() };
    let version = version_fn.and_then(|f| {
        // SAFETY: that slot holds `duckdb_library_version`, which takes nothing
        // and returns a pointer to a static NUL-terminated string.
        let f = unsafe {
            transmute::<unsafe extern "C" fn(), unsafe extern "C" fn() -> *const c_char>(f)
        };
        // SAFETY: calling DuckDB's own function.
        let text = unsafe { f() };
        if text.is_null() {
            return None;
        }
        // SAFETY: non-null, NUL-terminated, static (as above).
        unsafe { CStr::from_ptr(text) }.to_str().ok()
    });
    engine_table_slots(version, abi::built_against_version(), unknown_slots)
}

/// The slot count to take from an engine, from what is known about it. Pure, so
/// every branch is testable without an engine.
fn engine_table_slots(
    engine_version: Option<&str>,
    declared: Option<&str>,
    unknown_slots: usize,
) -> usize {
    let Some(version) = engine_version else {
        return unknown_slots;
    };
    if let Some(slots) = abi::expected_slot_count(version) {
        return slots;
    }
    if declared.is_some_and(|d| abi::parse_version(d) == abi::parse_version(version)) {
        return abi::compiled_slot_count();
    }
    unknown_slots
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPILED: usize = abi::compiled_slot_count();

    #[test]
    fn a_release_in_the_layout_table_gets_its_own_slot_count() {
        for (version, slots) in [
            ("v1.2.0", 408),
            ("v1.3.2", 428),
            ("v1.4.4", 459),
            ("v1.5.0", 545),
            ("v1.5.1", 545),
            ("v1.5.6", 546),
            ("v2.0.0", 548),
        ] {
            // Whatever the build declares or the policy allows.
            for declared in [None, Some("v9.9.9"), Some(version)] {
                for unknown in [STABLE_API_SLOT_COUNT, COMPILED] {
                    assert_eq!(
                        engine_table_slots(Some(version), declared, unknown),
                        slots,
                        "{version}, declared {declared:?}, unknown {unknown}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unknown_release_the_build_declared_gets_the_whole_compiled_struct() {
        assert_eq!(
            engine_table_slots(Some("v9.9.9"), Some("v9.9.9"), STABLE_API_SLOT_COUNT),
            COMPILED
        );
        // The same release written with or without the `v`.
        assert_eq!(
            engine_table_slots(Some("9.9.9"), Some("v9.9.9"), STABLE_API_SLOT_COUNT),
            COMPILED
        );
    }

    #[test]
    fn an_unknown_release_nobody_declared_gets_the_fallback_for_its_policy() {
        for declared in [None, Some("v9.9.8")] {
            assert_eq!(
                engine_table_slots(Some("v9.9.9"), declared, STABLE_API_SLOT_COUNT),
                STABLE_API_SLOT_COUNT
            );
            assert_eq!(
                engine_table_slots(Some("v9.9.9"), declared, COMPILED),
                COMPILED
            );
        }
        // A development build has no version a table or a declaration can name.
        assert_eq!(
            engine_table_slots(Some("v1.6.0-dev42"), None, STABLE_API_SLOT_COUNT),
            STABLE_API_SLOT_COUNT
        );
    }

    #[test]
    fn an_engine_that_reports_no_version_gets_the_fallback_for_its_policy() {
        assert_eq!(
            engine_table_slots(None, Some("v1.5.6"), STABLE_API_SLOT_COUNT),
            STABLE_API_SLOT_COUNT
        );
        assert_eq!(engine_table_slots(None, None, COMPILED), COMPILED);
    }

    #[test]
    fn strict_takes_what_the_requested_c_api_version_guarantees() {
        assert_eq!(guaranteed_slots_for("v1.2.0"), STABLE_API_SLOT_COUNT);
        assert_eq!(guaranteed_slots_for("v1.5.5"), STABLE_API_SLOT_COUNT);
        assert_eq!(guaranteed_slots_for("v1.5.6"), C_API_1_5_6_SLOT_COUNT);
        assert_eq!(guaranteed_slots_for("1.5.6"), C_API_1_5_6_SLOT_COUNT);
        assert_eq!(guaranteed_slots_for("v1.6.0"), C_API_1_5_6_SLOT_COUNT);
        assert_eq!(guaranteed_slots_for("v2.0.0"), C_API_1_5_6_SLOT_COUNT);
        // Not a version the bindings could have asked for: the safe floor.
        assert_eq!(guaranteed_slots_for("v1"), STABLE_API_SLOT_COUNT);
        assert_eq!(guaranteed_slots_for("v1.5.6-dev1"), STABLE_API_SLOT_COUNT);
        // The constants agree with the layout table: C API v1.5.6 is the whole
        // v1.5.6 struct, and the v1.2.0 prefix fits inside every known layout.
        assert_eq!(
            abi::expected_slot_count("v1.5.6"),
            Some(C_API_1_5_6_SLOT_COUNT)
        );
        assert!(abi::expected_slot_count("v1.2.0").is_some_and(|s| s >= STABLE_API_SLOT_COUNT));
    }

    // ── bounded_get_api against a mock engine ────────────────────────────────
    //
    // The mock table is allocated at exactly the engine's slot count, so Miri
    // (which the CI `miri` job runs over these library tests) reports any read
    // past it. None of this touches `libduckdb-sys`'s process-wide dispatch
    // table: the function under test is `bounded_get_api`, not the bindings'
    // `duckdb_rs_extension_api_init`.

    thread_local! {
        static MOCK_TABLE: Cell<*const Slot> = const { Cell::new(core::ptr::null()) };
        static MOCK_VERSION: Cell<*const c_char> = const { Cell::new(core::ptr::null()) };
    }

    unsafe extern "C" fn mock_get_api(
        _info: duckdb_extension_info,
        _version: *const c_char,
    ) -> *const c_void {
        MOCK_TABLE.with(Cell::get).cast()
    }

    unsafe extern "C" fn mock_library_version() -> *const c_char {
        MOCK_VERSION.with(Cell::get)
    }

    unsafe extern "C" fn some_function() {}

    /// Runs `bounded_get_api` against an engine table of exactly `engine_slots`
    /// slots whose `duckdb_library_version` reports `version` (`None`: slot 7 is
    /// empty), and returns which of the bindings' slots are non-null in what it
    /// handed back, reading every one of them.
    fn handed_back(engine_slots: usize, version: Option<&CStr>, unknown_slots: usize) -> Vec<bool> {
        let mut table: Vec<Slot> =
            vec![Some(some_function as unsafe extern "C" fn()); engine_slots];
        table[LIBRARY_VERSION_SLOT] = version.map(|v| {
            MOCK_VERSION.with(|cell| cell.set(v.as_ptr()));
            // SAFETY: a function pointer to another function pointer type with
            // the same size; it is transmuted back before it is called.
            unsafe {
                transmute::<unsafe extern "C" fn() -> *const c_char, unsafe extern "C" fn()>(
                    mock_library_version,
                )
            }
        });
        MOCK_TABLE.with(|cell| cell.set(table.as_ptr()));
        let access = duckdb_extension_access {
            set_error: None,
            get_database: None,
            get_api: Some(mock_get_api),
        };
        CONTEXT.with(|cell| {
            cell.set(Some(Context {
                access: &raw const access,
                unknown_slots,
            }));
        });
        let _active = Active;

        // SAFETY: inside the context set above; null `info` and `version` are
        // ignored by the mock.
        let handed = unsafe { bounded_get_api(core::ptr::null_mut(), core::ptr::null()) };
        assert!(!handed.is_null());
        let handed = handed.cast::<Slot>();
        (0..COMPILED)
            // SAFETY: the copy holds `COMPILED` slots.
            .map(|i| unsafe { handed.add(i).read() }.is_some())
            .collect()
    }

    /// Asserts slot by slot that exactly the first `copied` slots came from the
    /// engine (all of them non-null except `hole`, a slot the mock left empty)
    /// and every later one is padding.
    fn assert_copied(slots: &[bool], copied: usize, hole: Option<usize>) {
        assert_eq!(slots.len(), COMPILED);
        for (i, &present) in slots.iter().enumerate() {
            assert_eq!(present, i < copied && Some(i) != hole, "slot {i}");
        }
    }

    #[test]
    fn the_bindings_are_handed_exactly_the_engines_slots_padded_to_their_length() {
        for (engine_slots, version, policy_unknown) in [
            (459, c"v1.4.4", STABLE_API_SLOT_COUNT),
            (459, c"v1.4.4", COMPILED), // a known release is the same under any policy
            (545, c"v1.5.0", STABLE_API_SLOT_COUNT),
            (546, c"v1.5.6", STABLE_API_SLOT_COUNT),
            (548, c"v2.0.0", STABLE_API_SLOT_COUNT),
        ] {
            let slots = handed_back(engine_slots, Some(version), policy_unknown);
            assert_copied(&slots, engine_slots.min(COMPILED), None);
        }
    }

    #[test]
    fn an_unknown_release_under_strict_is_read_no_further_than_the_stable_prefix() {
        // A table of exactly the stable prefix (a real engine's is longer, so
        // this is the least the code may rely on): a read of even one slot more
        // is out of bounds, and Miri reports it.
        let slots = handed_back(
            STABLE_API_SLOT_COUNT,
            Some(c"v9.9.9"),
            STABLE_API_SLOT_COUNT,
        );
        assert_copied(&slots, STABLE_API_SLOT_COUNT, None);
        // An engine whose slot 7 is empty reports no version: the same bound,
        // with that one slot a hole in what was copied.
        let slots = handed_back(STABLE_API_SLOT_COUNT, None, STABLE_API_SLOT_COUNT);
        assert_copied(&slots, STABLE_API_SLOT_COUNT, Some(LIBRARY_VERSION_SLOT));
    }

    #[test]
    fn an_unknown_release_asked_for_c_api_1_5_6_under_strict_gets_the_whole_1_5_6_struct() {
        // Exactly the 546 slots C API v1.5.6 guarantees, from an engine with
        // no more than that: Miri reports a read of even one slot more.
        let slots = handed_back(
            C_API_1_5_6_SLOT_COUNT,
            Some(c"v9.9.9"),
            guaranteed_slots_for("v1.5.6"),
        );
        assert_copied(&slots, C_API_1_5_6_SLOT_COUNT.min(COMPILED), None);
    }

    #[test]
    fn an_unknown_release_under_a_permissive_policy_keeps_the_whole_compiled_struct() {
        // `Trust`, `Warn` and `AllowUnknownEngine` let an extension call an
        // unstable slot on a release quack-rs has not catalogued; the engine's
        // table is taken to be at least as long as the build's.
        let slots = handed_back(COMPILED, Some(c"v9.9.9"), COMPILED);
        assert_copied(&slots, COMPILED, None);
    }

    #[test]
    fn a_null_table_from_get_api_stays_null() {
        extern "C" fn no_api(_: duckdb_extension_info, _: *const c_char) -> *const c_void {
            core::ptr::null()
        }
        let access = duckdb_extension_access {
            set_error: None,
            get_database: None,
            get_api: Some(no_api),
        };
        CONTEXT.with(|cell| {
            cell.set(Some(Context {
                access: &raw const access,
                unknown_slots: STABLE_API_SLOT_COUNT,
            }));
        });
        let _active = Active;
        // SAFETY: inside the context set above.
        let handed = unsafe { bounded_get_api(core::ptr::null_mut(), core::ptr::null()) };
        assert!(handed.is_null());
    }

    #[test]
    fn outside_an_init_call_there_is_nothing_to_forward_to() {
        // SAFETY: no context is set on this thread, which this function reports.
        let handed = unsafe { bounded_get_api(core::ptr::null_mut(), core::ptr::null()) };
        assert!(handed.is_null());
    }

    #[test]
    fn ending_a_call_clears_the_context_and_frees_the_copy() {
        let slots = handed_back(459, Some(c"v1.4.4"), STABLE_API_SLOT_COUNT);
        assert_eq!(slots.len(), COMPILED);
        // `handed_back`'s `Active` has dropped.
        assert!(CONTEXT.with(Cell::get).is_none());
        assert!(PADDED.with(|padded| padded.borrow().is_empty()));
    }
}
