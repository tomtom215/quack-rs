// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Generic `FfiState<T>` wrapper for safe aggregate state management.
//!
//! # Problem solved
//!
//! Writing the state init/destroy lifecycle using raw pointers is error-prone.
//! The canonical pattern is:
//!
//! ```rust,no_run
//! use libduckdb_sys::{duckdb_function_info, duckdb_aggregate_state, idx_t};
//!
//! #[derive(Default)]
//! struct MyState { count: u64 }
//!
//! #[repr(C)]
//! struct FfiState { inner: *mut MyState }
//!
//! unsafe extern "C" fn state_init(_: duckdb_function_info, state: duckdb_aggregate_state) {
//!     let ffi = &mut *(state as *mut FfiState);
//!     ffi.inner = Box::into_raw(Box::new(MyState::default()));
//! }
//!
//! unsafe extern "C" fn state_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
//!     for i in 0..count as usize {
//!         let state_ptr = *states.add(i);
//!         let ffi = &mut *(state_ptr as *mut FfiState);
//!         if !ffi.inner.is_null() {
//!             drop(Box::from_raw(ffi.inner));
//!             ffi.inner = std::ptr::null_mut();
//!         }
//!     }
//! }
//! ```
//!
//! [`FfiState<T>`] encapsulates this pattern. Your type `T` only needs to
//! implement [`AggregateState`] (which requires `Default`), and you call the
//! provided helper methods instead of writing raw pointer code.
//!
//! # Pitfalls prevented
//!
//! - **L1**: Combine propagates all fields because your type `T`'s `combine`
//!   method is responsible — the `FfiState` wrapper ensures `T`'s method is called.
//! - **L2**: No double-free — `destroy_callback` clears the state's tag before
//!   dropping `T`, and drops only a state whose tag matches.
//! - **L3**: No panic across FFI — `with_state_mut` returns an `Option`, not a panic.

use libduckdb_sys::{duckdb_aggregate_state, duckdb_function_info, idx_t};

/// Trait for types that can be used as `DuckDB` aggregate state.
///
/// Implement this for your state struct. It requires `Default` (used to
/// create the initial state in `state_init`), `Send` (`DuckDB` moves states
/// between worker threads) and `Sync`: a window's segment tree keeps its
/// states in shared, unlocked storage (`WindowSegmentTreeGlobalState`,
/// `window_segment_tree.cpp`), and every thread evaluating a frame that covers
/// one reads it as a `combine` source, so two threads can hold `&T` to the
/// same state at once. A `T` with a `Cell` or `RefCell` would race there.
///
/// **Breaking** in 0.18.0: the `Sync` bound is new.
///
/// # Example
///
/// ```rust
/// use quack_rs::aggregate::AggregateState;
///
/// #[derive(Default)]
/// struct WordCount {
///     count: u64,
/// }
///
/// impl AggregateState for WordCount {}
///
/// // FfiState::<WordCount>::size_callback and other methods are now available.
/// ```
pub trait AggregateState: Default + Send + Sync + 'static {}

/// A generic FFI-compatible state wrapper for use with `DuckDB` aggregate functions.
///
/// `FfiState<T>` describes the bytes `DuckDB` allocates for each aggregate
/// group: [`size_callback`][FfiState::size_callback] tells `DuckDB` how many,
/// [`init_callback`][FfiState::init_callback] puts a `T::default()` in them,
/// [`with_state`][FfiState::with_state] and
/// [`with_state_mut`][FfiState::with_state_mut] hand out the `T`, and
/// [`destroy_callback`][FfiState::destroy_callback] drops it. The type is
/// never constructed; it only names the layout and its callbacks.
///
/// # Known `DuckDB` limitation
///
/// **Two query shapes make every C-API aggregate read out of bounds.** In
/// them `DuckDB` calls `update` with a state array holding **one** state while
/// passing `count > 1` rows, so the callback reads `states[1..count]` past the
/// end of the array — undefined behaviour in *any* C-API aggregate, whether
/// built with quack-rs or by hand:
///
/// - **Window aggregates whose frame is the whole partition**, e.g.
///   `agg(x) OVER ()` — `WindowConstantAggregator`
///   (`src/function/window/window_constant_aggregator.cpp`, ~lines 106 and
///   296–299 in `DuckDB` 1.5.5).
/// - **Ordered aggregates**, e.g. `agg(x ORDER BY y)` —
///   `src/function/aggregate/sorted_aggregate_function.cpp`, ~lines 630–633.
///
/// Both pass a `CONSTANT_VECTOR` of states because the function has no
/// `simple_update` (the C API cannot set one), and `CAPIAggregateUpdate`
/// (`src/main/capi/aggregate_function-c.cpp`, ~lines 92–110) hands the
/// vector's data pointer to the extension without flattening it. This is a
/// defect in `DuckDB`'s C API, not in quack-rs, and it cannot be detected
/// from inside the callback — reading `states[1]` to check is itself the
/// out-of-bounds read. Reported upstream as
/// [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109). Until it is fixed, do not use C-API aggregates in
/// those two query shapes.
///
/// # Memory layout
///
/// ```text
/// [ tag: usize ][ T, padded to a multiple of usize ]   T stored inline
/// [ tag: usize ][ *mut T                          ]   T boxed
/// ```
///
/// `T` is stored in the state bytes themselves when its alignment is at most
/// `usize`'s and it is at most 256 bytes; otherwise the slot holds a
/// `Box<T>`. The size is a multiple of `usize` either way, because some
/// `DuckDB` operators lay states out back to back at a stride of exactly
/// `state_size` (the window aggregators), and every state must stay aligned.
/// `DuckDB` aligns each state to 8 bytes (`AlignValue` in `AggregateObject`
/// and `TupleDataLayout`; `new[]`-allocated buffers elsewhere), which covers
/// both forms on 64- and 32-bit targets.
///
/// # States `DuckDB` never destroys
///
/// `DuckDB` 1.4.4 to 1.5.5 destroys a grouped aggregate's states as its result
/// scan passes them. When the scan stops early — a `LIMIT` above the
/// aggregate, an error raised above it, an interrupt — the states it did not
/// reach are never destroyed (`docs/upstream-duckdb-reports.md`, item 20):
/// under `LIMIT 10`, 2,048 of 300,000 on one thread. Their `T` is never
/// dropped. A `T` stored inline costs nothing more: its bytes belong to
/// `DuckDB`'s hash table, which `DuckDB` frees. What leaks is whatever `T`
/// itself owns on the heap (a `Vec`, a `String`, a `HashMap`), and, for a
/// boxed `T`, the box. The same holds for a window frame with an `EXCLUDE`
/// clause, whose segment tree initialises one extra state per row and never
/// destroys it (item 35; 5000 `T`s undropped over a 5000-row window). Until
/// `DuckDB` fixes these, prefer a state that owns no heap memory and is small
/// enough to be stored inline.
///
/// # States `DuckDB` never initialised
///
/// When a `state_init` call fails — a panicking `T::default()`, or an
/// allocation failure — `DuckDB` 1.4.4 to 1.5.5 still calls the destructor
/// on every state row it had created, initialised or not
/// (`RowOperations::InitializeStates` is not exception-safe; see
/// `docs/upstream-duckdb-reports.md`). That includes the states of *other*
/// aggregates in the same query whose `state_init` never ran. Those bytes
/// are whatever the allocation held, and dropping them as a `T` would run a
/// destructor over garbage.
///
/// So [`init_callback`][Self::init_callback] also stores `tag`, a value
/// derived from `T` (and, for a boxed `T`, from the box's address), and
/// [`destroy_callback`][Self::destroy_callback] drops only a slot whose tag
/// matches, clearing it first. A slot the destructor has already processed no
/// longer matches; a slot another aggregate's type initialised does not match
/// either. What is left is chance: uninitialised bytes that happen to hold
/// the tag, one in 2<sup>64</sup> on a 64-bit target. This is a mitigation of
/// a `DuckDB` defect, not a guarantee. Reading bytes `DuckDB` may never have
/// written is, in Rust's abstract machine, a read of uninitialised memory
/// whatever the load: the destructor uses volatile loads so the compiler
/// cannot reason from them, but no Rust construct makes such a read defined.
/// It is the least bad of the options this defect leaves.
///
/// The tag depends on the slot's contents, not its address, because `DuckDB`
/// moves states: radix repartitioning of a hash aggregate copies each state's
/// bytes to a new row (`radix_partitioned_hashtable.cpp`) and destroys it
/// there. The fourth audit's tag used the address, so every moved state was
/// skipped and its `T` leaked — millions per query on eight threads. Moving
/// a Rust value by copying its bytes is always valid; the copy left behind is
/// never destroyed.
///
/// # Usage
///
/// ```rust
/// use quack_rs::aggregate::{AggregateState, FfiState};
/// use libduckdb_sys::{duckdb_function_info, duckdb_aggregate_state, idx_t};
///
/// #[derive(Default)]
/// struct MyState { sum: i64 }
/// impl AggregateState for MyState {}
///
/// // In your registration code:
/// // .ffi_state::<MyState>()
/// ```
///
/// **Breaking** in 0.18.0: `FfiState<T>` was a two-word struct with a public
/// `inner: *mut T` field that always boxed `T`. The layout is now private.
pub struct FfiState<T: AggregateState> {
    _state: core::marker::PhantomData<fn() -> T>,
}

impl<T: AggregateState> FfiState<T> {
    /// The tag's width, and the offset of the payload.
    const WORD: usize = core::mem::size_of::<usize>();

    /// Whether `T` is stored in the state bytes rather than boxed.
    const INLINE: bool =
        core::mem::align_of::<T>() <= Self::WORD && core::mem::size_of::<T>() <= INLINE_LIMIT;

    /// Bytes per state: the tag, then `T` (rounded up to a whole number of
    /// words) or a `Box<T>`.
    const SIZE: usize = Self::WORD
        + if Self::INLINE {
            core::mem::size_of::<T>().div_ceil(Self::WORD) * Self::WORD
        } else {
            Self::WORD
        };

    /// Mixed into the tag: an arbitrary odd constant, so that a zeroed slot
    /// does not match.
    #[cfg(target_pointer_width = "64")]
    const TAG_KEY: usize = 0x9E37_79B9_7F4A_7C15;
    #[cfg(not(target_pointer_width = "64"))]
    const TAG_KEY: usize = 0x9E37_79B9;

    /// The tag a slot carries once initialised for this `T`: `boxed` is the
    /// box's address for a boxed `T`, and zero for an inline one.
    ///
    /// `TAG_KEY` is salted with the address of `T`'s type name, which differs
    /// between types whose names differ, so a slot initialised for one
    /// aggregate's state type is not accepted by another's destructor. The
    /// salt's low bit is cleared, so the key stays odd and never zero: a zeroed
    /// slot never matches.
    fn tag_for(boxed: *const T) -> usize {
        let salt = Self::salt(core::any::type_name::<T>().as_ptr() as usize);
        (boxed as usize) ^ Self::TAG_KEY ^ salt
    }

    /// `name_addr` with its low bit cleared, so that `TAG_KEY ^ salt` stays
    /// odd.
    const fn salt(name_addr: usize) -> usize {
        name_addr & !1
    }

    /// The payload: `T` itself, or the `Box<T>`'s pointer.
    ///
    /// Derived as the word after the tag, so it keeps the tag's `usize`
    /// alignment, which is all either form needs.
    const fn payload(state: duckdb_aggregate_state) -> *mut usize {
        state.cast::<usize>().wrapping_add(1)
    }

    /// Returns the number of bytes `DuckDB` allocates per state.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::aggregate::{AggregateState, FfiState};
    ///
    /// #[derive(Default)]
    /// struct MyState { val: i64 }
    /// impl AggregateState for MyState {}
    ///
    /// // A tag, then the `i64` itself.
    /// assert_eq!(FfiState::<MyState>::size(), 2 * std::mem::size_of::<usize>());
    /// ```
    #[inline]
    #[must_use]
    pub const fn size() -> usize {
        Self::SIZE
    }

    /// The `state_size` callback function for use in the builder.
    ///
    /// Returns [`size`][Self::size]: the number of bytes `DuckDB` must
    /// allocate per aggregate group.
    ///
    /// # Safety
    ///
    /// This is an `unsafe extern "C"` function pointer. It is safe to pass to
    /// [`AggregateFunctionBuilder::state_size`][crate::aggregate::AggregateFunctionBuilder::state_size].
    pub const unsafe extern "C" fn size_callback(_info: duckdb_function_info) -> idx_t {
        Self::SIZE as idx_t
    }

    /// The `state_init` callback function for use in the builder.
    ///
    /// Stores a `T::default()` in the state at `state` (or a box holding one).
    ///
    /// # Pitfall L3: `T::default()` is user code
    ///
    /// `DuckDB` invokes this through an `extern "C"` function pointer, so a
    /// panic escaping `T::default()` would abort the process (Rust 1.81+ turns
    /// an unwind across `extern "C"` into `panic_cannot_unwind`). The call is
    /// therefore run under
    /// [`catch_ffi_panic`][crate::callback::catch_ffi_panic] and a panic is
    /// reported through `duckdb_aggregate_function_set_error` — which
    /// `DuckDB`'s `CAPIAggregateStateInit` checks, turning it into an ordinary
    /// SQL error — instead of taking the session down.
    ///
    /// # Uninitialised memory
    ///
    /// `DuckDB` does not promise the state allocation is zeroed, so the tag is
    /// cleared (and a boxed slot's pointer nulled) *before* any user code
    /// runs. A slot whose `T::default()` panicked therefore never matches its
    /// tag: [`with_state`][Self::with_state] returns `None` for it and
    /// [`destroy_callback`][Self::destroy_callback] skips it.
    ///
    /// # Safety
    ///
    /// - `state` must point to [`size`][Self::size] writable bytes allocated
    ///   by `DuckDB`, aligned to `usize`.
    /// - This function must only be called once per state allocation.
    pub unsafe extern "C" fn init_callback(
        info: duckdb_function_info,
        state: duckdb_aggregate_state,
    ) {
        let payload = Self::payload(state);
        // SAFETY: `state` is `size()` writable bytes aligned to `usize`, so
        // the tag word and (for a boxed `T`) the pointer word after it are in
        // bounds and aligned. `ptr::write` does not read what was there.
        unsafe {
            core::ptr::write(state.cast::<usize>(), 0);
            if !Self::INLINE {
                core::ptr::write(payload.cast::<*mut T>(), core::ptr::null_mut());
            }
        }
        let made = if Self::INLINE {
            crate::callback::catch_ffi_panic(T::default).map(|value| {
                // SAFETY: the payload is `size_of::<T>()` bytes inside the
                // state, at offset `usize`, which `INLINE` guarantees is a
                // multiple of `align_of::<T>()`.
                unsafe { core::ptr::write(payload.cast::<T>(), value) };
                Self::tag_for(core::ptr::null())
            })
        } else {
            crate::callback::catch_ffi_panic(Box::<T>::default).map(|boxed| {
                let inner = Box::into_raw(boxed);
                // SAFETY: the pointer word was written above.
                unsafe { core::ptr::write(payload.cast::<*mut T>(), inner) };
                Self::tag_for(inner)
            })
        };
        match made {
            // SAFETY: the tag word was written above.
            Ok(tag) => unsafe { core::ptr::write(state.cast::<usize>(), tag) },
            Err(message) => {
                let c_msg = crate::callback::message_to_c_string(&format!(
                    "quack-rs: aggregate state initialiser panicked: {message}"
                ));
                // SAFETY: `info` is the handle DuckDB passed in, and DuckDB
                // checks the error flag on return from the state-init callback.
                unsafe {
                    libduckdb_sys::duckdb_aggregate_function_set_error(info, c_msg.as_ptr());
                }
            }
        }
    }

    /// The `state_destroy` callback function for use in the builder.
    ///
    /// Drops the `T` in each state in `states[0..count]` (freeing its box, if
    /// boxed), clearing the tag first so a second call is a no-op.
    ///
    /// # Count conversion
    ///
    /// If `count` (an `idx_t`) cannot be converted to `usize`, the loop iterates
    /// zero times — no states are freed. This is a defensive choice to avoid
    /// panicking across FFI. In practice, `idx_t` is `u64` and `usize` is at
    /// least 64 bits on all `DuckDB`-supported platforms, so this path is
    /// unreachable on supported targets.
    ///
    /// # Pitfall L3: `T::drop` is user code
    ///
    /// `DuckDB` calls the destructor as `info.destroy(states, count)` — no info
    /// handle, no return value, and therefore **no error channel at all**
    /// (verified in `CAPIAggregateDestructor`). A panic escaping `T::drop`
    /// would abort the process, so each drop runs under
    /// [`catch_ffi_panic`][crate::callback::catch_ffi_panic] and the message is
    /// discarded. Every remaining state is still dropped: one bad destructor
    /// does not leak the rest of the group.
    ///
    /// One case cannot be contained: a `T::drop` that panics while one of
    /// `T`'s fields also panics in its own `drop` during that unwind. Rust
    /// aborts the process for a panic raised while unwinding ("panic in a
    /// destructor during cleanup") before any `catch_unwind` sees it.
    ///
    /// # Safety
    ///
    /// - `states` must point to an array of `count` valid `duckdb_aggregate_state`
    ///   pointers, each to [`size`][Self::size] bytes, aligned to `usize`, that
    ///   `DuckDB` allocated for this aggregate. A slot
    ///   [`init_callback`][Self::init_callback] never initialised is skipped
    ///   (see "States `DuckDB` never initialised" on the type).
    pub unsafe extern "C" fn destroy_callback(states: *mut duckdb_aggregate_state, count: idx_t) {
        for i in 0..usize::try_from(count).unwrap_or(0) {
            // SAFETY: `states` is a valid array of `count` pointers.
            let state = unsafe { *states.add(i) };
            let payload = Self::payload(state);
            // SAFETY: the slot is `size()` bytes DuckDB allocated for this
            // aggregate, aligned to `usize`, so the tag word (and a boxed
            // slot's pointer word) are in bounds. They may never have been
            // written by `init_callback`, so they are read as plain words with
            // volatile loads, which the compiler must perform as written and
            // cannot reason about, and only trusted once the tag matches.
            let tag = unsafe { core::ptr::read_volatile(state.cast::<usize>()) };
            let inner = if Self::INLINE {
                core::ptr::null_mut()
            } else {
                // SAFETY: as above.
                unsafe { core::ptr::read_volatile(payload.cast::<*mut T>()) }
            };
            if tag != Self::tag_for(inner) {
                continue;
            }
            // Clear the tag (and the pointer) *before* dropping: if `T::drop`
            // panics, the value is still released by the unwind, so leaving
            // the slot live would make a second destructor call a double drop.
            // SAFETY: the tag matched, so `init_callback` initialised the slot.
            unsafe {
                core::ptr::write(state.cast::<usize>(), 0);
                if !Self::INLINE {
                    core::ptr::write(payload.cast::<*mut T>(), core::ptr::null_mut());
                }
            }
            // The drop runs arbitrary user code, so it must not be allowed to
            // unwind out of this `extern "C"` function.
            drop(crate::callback::catch_ffi_panic(|| {
                if Self::INLINE {
                    // SAFETY: the tag matched, so the payload holds a `T` that
                    // `init_callback` wrote and nothing has dropped; the tag is
                    // cleared, so nothing will drop it again.
                    unsafe { core::ptr::drop_in_place(payload.cast::<T>()) };
                } else if !inner.is_null() {
                    // SAFETY: `inner` came from `Box::into_raw` in
                    // `init_callback`, and the slot no longer refers to it.
                    drop(unsafe { Box::from_raw(inner) });
                }
            }));
        }
    }

    /// The `T` in an initialised slot, or `None` if the slot's tag does not
    /// match (its `T::default()` panicked).
    ///
    /// # Safety
    ///
    /// `state` must be a slot [`init_callback`][Self::init_callback] has run on.
    unsafe fn value(state: duckdb_aggregate_state) -> Option<*mut T> {
        let payload = Self::payload(state);
        // SAFETY: `init_callback` wrote the tag word, and for a boxed `T` the
        // pointer word.
        let tag = unsafe { core::ptr::read(state.cast::<usize>()) };
        if Self::INLINE {
            (tag == Self::tag_for(core::ptr::null())).then(|| payload.cast::<T>())
        } else {
            // SAFETY: as above.
            let inner = unsafe { core::ptr::read(payload.cast::<*mut T>()) };
            (!inner.is_null() && tag == Self::tag_for(inner)).then_some(inner)
        }
    }

    /// Provides safe mutable access to the inner `T` value.
    ///
    /// Returns `None` if the slot holds no `T` — its `T::default()` panicked
    /// (and `DuckDB` is about to fail the query), or it was destroyed.
    ///
    /// # Pitfall L3: No panic across FFI
    ///
    /// This method returns `Option<&mut T>` rather than unwrapping, so callers
    /// can use `if let Some(state) = ...` patterns without panicking.
    ///
    /// # Safety
    ///
    /// - `state` must point to a state `DuckDB` allocated for this aggregate
    ///   and [`init_callback`][Self::init_callback] initialised.
    /// - No other reference to the same `T` must exist simultaneously.
    /// - The returned reference must not outlive the callback that received
    ///   `state`. The lifetime `'a` is the caller's to choose and nothing ties
    ///   it to `state`; `DuckDB` destroys the state — and
    ///   [`destroy_callback`][Self::destroy_callback] drops the `T` — once it
    ///   is finalized or merged, and may move its bytes between callbacks.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::aggregate::{AggregateState, FfiState};
    ///
    /// #[derive(Default)]
    /// struct Counter { n: u64 }
    /// impl AggregateState for Counter {}
    ///
    /// // Inside your update callback:
    /// // let ffi_state: duckdb_aggregate_state = ...;
    /// // if let Some(state) = unsafe { FfiState::<Counter>::with_state_mut(ffi_state) } {
    /// //     state.n += 1;
    /// // }
    /// ```
    pub unsafe fn with_state_mut<'a>(state: duckdb_aggregate_state) -> Option<&'a mut T> {
        // SAFETY: the caller guarantees `state` was initialised, and that no
        // other reference to its `T` exists.
        unsafe { Self::value(state).map(|value| &mut *value) }
    }

    /// Provides safe immutable access to the inner `T` value.
    ///
    /// See [`with_state_mut`][Self::with_state_mut] for safety requirements.
    ///
    /// # Safety
    ///
    /// Same as `with_state_mut`, but only borrows immutably: no mutable
    /// reference to the same `T` may exist while this one does.
    pub unsafe fn with_state<'a>(state: duckdb_aggregate_state) -> Option<&'a T> {
        // SAFETY: the caller guarantees `state` was initialised, and that no
        // mutable reference to its `T` exists.
        unsafe { Self::value(state).map(|value| &*value) }
    }
}

/// The largest `T`, in bytes, that [`FfiState`] stores inline. A larger `T`
/// is boxed. Every aggregate state of a group shares one hash-table row, and
/// `DuckDB` allocates rows in blocks of 256 KiB by default
/// (`DEFAULT_BLOCK_ALLOC_SIZE`, `TupleDataAllocator::Build`); boxing a large
/// `T` keeps the row narrow.
const INLINE_LIMIT: usize = 256;

impl<T: AggregateState> core::fmt::Debug for FfiState<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FfiState")
            .field("state", &core::any::type_name::<T>())
            .field("inline", &Self::INLINE)
            .field("size", &Self::SIZE)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicIsize, Ordering};

    const WORD: usize = core::mem::size_of::<usize>();

    #[derive(Default, Debug, PartialEq)]
    struct Counter {
        value: u64,
    }
    impl AggregateState for Counter {}

    /// Too large to store inline.
    #[derive(Debug, PartialEq)]
    struct Big {
        values: [u64; 64],
    }
    impl Default for Big {
        fn default() -> Self {
            Self { values: [0; 64] }
        }
    }
    impl AggregateState for Big {}

    /// Aligned more strictly than `usize`.
    #[allow(dead_code)] // only its layout matters
    #[derive(Default)]
    #[repr(align(32))]
    struct Aligned(u8);
    impl AggregateState for Aligned {}

    #[derive(Default)]
    struct Zst;
    impl AggregateState for Zst {}

    #[allow(dead_code)] // only its layout matters
    #[derive(Default)]
    struct Odd([u8; 3]);
    impl AggregateState for Odd {}

    #[allow(dead_code)] // only its layout matters
    struct AtLimit([u8; INLINE_LIMIT]);
    impl Default for AtLimit {
        fn default() -> Self {
            Self([0; INLINE_LIMIT])
        }
    }
    impl AggregateState for AtLimit {}

    #[allow(dead_code)] // only its layout matters
    struct OverLimit([u8; INLINE_LIMIT + 1]);
    impl Default for OverLimit {
        fn default() -> Self {
            Self([0; INLINE_LIMIT + 1])
        }
    }
    impl AggregateState for OverLimit {}

    /// Live `Tracked` / `TrackedBig` values, to see whether a destructor ran.
    static TRACKED_LIVE: AtomicIsize = AtomicIsize::new(0);

    /// Held by every test that counts `TRACKED_LIVE`: the tests run in
    /// parallel, and one test's live value would otherwise show in another's
    /// count.
    static TRACKED: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn tracked() -> std::sync::MutexGuard<'static, ()> {
        TRACKED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    struct Tracked {
        value: u64,
    }
    impl Default for Tracked {
        fn default() -> Self {
            TRACKED_LIVE.fetch_add(1, Ordering::SeqCst);
            Self { value: 7 }
        }
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            TRACKED_LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl AggregateState for Tracked {}

    struct TrackedBig {
        values: [u64; 64],
    }
    impl Default for TrackedBig {
        fn default() -> Self {
            TRACKED_LIVE.fetch_add(1, Ordering::SeqCst);
            Self { values: [7; 64] }
        }
    }
    impl Drop for TrackedBig {
        fn drop(&mut self) {
            TRACKED_LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl AggregateState for TrackedBig {}

    /// `count` states of `T`'s size, back to back like `DuckDB`'s window
    /// arrays, each word filled with `fill`.
    fn slots<T: AggregateState>(count: usize, fill: usize) -> Vec<usize> {
        assert_eq!(FfiState::<T>::size() % WORD, 0);
        vec![fill; count * FfiState::<T>::size() / WORD]
    }

    /// State `index` of the array starting at `base`. Every state of one
    /// array is derived from the same `base`, so none invalidates another
    /// (Miri, Stacked Borrows).
    fn state_at<T: AggregateState>(base: *mut usize, index: usize) -> duckdb_aggregate_state {
        base.wrapping_add(index * FfiState::<T>::size() / WORD)
            .cast()
    }

    fn init<T: AggregateState>(state: duckdb_aggregate_state) {
        // SAFETY: `state` is `size()` writable, word-aligned bytes; a
        // non-panicking `T::default()` never touches `info`.
        unsafe { FfiState::<T>::init_callback(core::ptr::null_mut(), state) };
    }

    fn destroy<T: AggregateState>(states: &mut [duckdb_aggregate_state]) {
        // SAFETY: each state is `size()` word-aligned bytes, initialised or
        // not, which is what the destructor must cope with.
        unsafe { FfiState::<T>::destroy_callback(states.as_mut_ptr(), states.len() as idx_t) };
    }

    /// A tag is the box's address XOR a per-type key, so two tags of one
    /// type differ exactly where the addresses do, and an inline state's tag
    /// (no box) is the key itself.
    #[test]
    fn a_tag_is_the_box_address_xor_the_types_key() {
        let key = FfiState::<Counter>::tag_for(core::ptr::null());
        for addr in [0x10_usize, 0x1000, 0xFFFF_FFF0, usize::MAX & !0xF] {
            let boxed = core::ptr::without_provenance::<Counter>(addr);
            assert_eq!(FfiState::<Counter>::tag_for(boxed) ^ key, addr, "{addr:#x}");
        }
    }

    /// The key is odd, whatever the type name's address, so no zeroed slot
    /// and no aligned box address can produce a tag of zero.
    #[test]
    fn the_salt_clears_the_low_bit_so_the_key_stays_odd() {
        for name_addr in [0x1000_usize, 0x1001, 0x7FFF_FFFF, usize::MAX] {
            let salt = FfiState::<Counter>::salt(name_addr);
            assert_eq!(salt & 1, 0, "{name_addr:#x}");
            assert_eq!(salt | 1, name_addr | 1, "{name_addr:#x}");
            assert_eq!((FfiState::<Counter>::TAG_KEY ^ salt) & 1, 1);
        }
    }

    #[test]
    fn a_small_state_is_stored_inline_after_a_tag() {
        const { assert!(FfiState::<Counter>::INLINE) };
        assert_eq!(FfiState::<Counter>::size(), 2 * WORD);
        const { assert!(FfiState::<Zst>::INLINE) };
        assert_eq!(FfiState::<Zst>::size(), WORD);
        // Padded to whole words, so back-to-back states stay aligned.
        const { assert!(FfiState::<Odd>::INLINE) };
        assert_eq!(FfiState::<Odd>::size(), 2 * WORD);
        const { assert!(FfiState::<AtLimit>::INLINE) };
        assert_eq!(FfiState::<AtLimit>::size(), WORD + INLINE_LIMIT);
    }

    #[test]
    fn a_large_or_over_aligned_state_is_boxed() {
        for (inline, size) in [
            (FfiState::<Big>::INLINE, FfiState::<Big>::size()),
            (FfiState::<OverLimit>::INLINE, FfiState::<OverLimit>::size()),
            (FfiState::<Aligned>::INLINE, FfiState::<Aligned>::size()),
        ] {
            assert!(!inline);
            assert_eq!(size, 2 * WORD);
        }
    }

    #[test]
    fn size_callback_returns_size() {
        // SAFETY: size_callback does not read its argument.
        let size = unsafe { FfiState::<Odd>::size_callback(core::ptr::null_mut()) };
        assert_eq!(usize::try_from(size).unwrap(), FfiState::<Odd>::size());
        // SAFETY: as above.
        let size = unsafe { FfiState::<Big>::size_callback(core::ptr::null_mut()) };
        assert_eq!(usize::try_from(size).unwrap(), FfiState::<Big>::size());
    }

    #[test]
    fn debug_names_the_type_and_the_storage() {
        let text = format!(
            "{:?}",
            FfiState::<Counter> {
                _state: core::marker::PhantomData
            }
        );
        assert!(text.contains("Counter"), "{text}");
        assert!(text.contains("inline: true"), "{text}");
        assert!(text.contains(&format!("size: {}", 2 * WORD)), "{text}");
    }

    fn lifecycle<T: AggregateState, V: PartialEq + core::fmt::Debug>(
        read: fn(&T) -> V,
        write: fn(&mut T),
        written: V,
    ) {
        let mut words = slots::<T>(3, 0xA5A5_A5A5);
        let base = words.as_mut_ptr();
        let mut states: Vec<_> = (0..3).map(|i| state_at::<T>(base, i)).collect();
        for &state in &states {
            init::<T>(state);
        }
        // SAFETY: initialised above; no other reference exists.
        let value = unsafe { FfiState::<T>::with_state_mut(states[1]) }.expect("initialised");
        write(value);
        // SAFETY: initialised; the mutable borrow above has ended.
        let read_back = unsafe { FfiState::<T>::with_state(states[1]) }.map(read);
        assert_eq!(read_back, Some(written));
        destroy::<T>(&mut states);
        for &state in &states {
            // SAFETY: the slots stay allocated; destroyed ones hold no `T`.
            assert!(unsafe { FfiState::<T>::value(state) }.is_none());
        }
    }

    #[test]
    fn init_access_and_destroy_an_inline_state() {
        lifecycle::<Counter, u64>(|c| c.value, |c| c.value = 42, 42);
    }

    #[test]
    fn init_access_and_destroy_a_boxed_state() {
        lifecycle::<Big, u64>(|b| b.values[63], |b| b.values[63] = 42, 42);
    }

    /// The fourth audit's F3. `DuckDB` destroys states whose `state_init`
    /// never ran when another init fails, so a slot can hold any bytes. A
    /// slot whose tag does not match is skipped, not dropped. Under Miri,
    /// dropping a boxed slot's garbage pointer would be reported as
    /// undefined behaviour.
    fn skips_a_slot_init_never_initialised<T: AggregateState>() {
        let _serial = tracked();
        for fill in [0, 0x5a5a_5a5a, usize::MAX] {
            let mut words = slots::<T>(1, fill);
            let base = words.as_mut_ptr();
            let before = words.clone();
            let mut states = [state_at::<T>(base, 0)];
            destroy::<T>(&mut states);
            assert_eq!(words, before, "an untagged slot is left alone");
            // SAFETY: the slot is allocated; a mismatched tag reads as no `T`.
            assert!(unsafe { FfiState::<T>::value(states[0]) }.is_none());
        }
    }

    #[test]
    fn destroy_skips_an_uninitialised_inline_slot() {
        skips_a_slot_init_never_initialised::<Tracked>();
    }

    #[test]
    fn destroy_skips_an_uninitialised_boxed_slot() {
        skips_a_slot_init_never_initialised::<TrackedBig>();
    }

    /// `DuckDB` moves states: radix repartitioning copies each state's bytes
    /// to another row and destroys it there. A tag derived from the slot's
    /// address (the fourth audit's) made the destructor skip every moved
    /// state, leaking its `T`. The tag follows the contents, so a moved state
    /// is destroyed.
    fn a_moved_state_is_destroyed<T: AggregateState>() {
        let _serial = tracked();
        let before = TRACKED_LIVE.load(Ordering::SeqCst);
        let mut words = slots::<T>(2, 0);
        let base = words.as_mut_ptr();
        let from = state_at::<T>(base, 0);
        let to = state_at::<T>(base, 1);
        init::<T>(from);
        // SAFETY: both slots are `size()` bytes of one allocation; the copy
        // is what `DuckDB` does when it moves a row.
        unsafe {
            core::ptr::copy_nonoverlapping(
                from.cast::<u8>(),
                to.cast::<u8>(),
                FfiState::<T>::size(),
            );
        }
        assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), before + 1);
        destroy::<T>(&mut [to]);
        assert_eq!(
            TRACKED_LIVE.load(Ordering::SeqCst),
            before,
            "the moved state's T was dropped"
        );
    }

    #[test]
    fn a_moved_inline_state_is_destroyed() {
        a_moved_state_is_destroyed::<Tracked>();
    }

    #[test]
    fn a_moved_boxed_state_is_destroyed() {
        a_moved_state_is_destroyed::<TrackedBig>();
    }

    /// A slot initialised for one state type is not dropped by another type's
    /// destructor: that would drop an `A` as a `B` (the soundness audit's F4,
    /// reachable when `DuckDB` reuses the bytes of a state it never destroyed).
    #[test]
    fn a_slot_initialised_for_another_type_is_skipped() {
        let _serial = tracked();
        let before = TRACKED_LIVE.load(Ordering::SeqCst);
        let mut words = slots::<Tracked>(1, 0);
        let base = words.as_mut_ptr();
        let mut states = [state_at::<Tracked>(base, 0)];
        init::<Counter>(states[0]);
        let initialised = words.clone();
        destroy::<Tracked>(&mut states);
        assert_eq!(words, initialised, "another type's slot is left alone");
        assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), before);
        destroy::<Counter>(&mut states);
        assert_eq!(words[0], 0, "its own destructor clears the tag");

        let mut words = slots::<TrackedBig>(1, 0);
        let base = words.as_mut_ptr();
        let mut states = [state_at::<TrackedBig>(base, 0)];
        init::<Big>(states[0]);
        let initialised = words.clone();
        destroy::<TrackedBig>(&mut states);
        assert_eq!(words, initialised, "another type's slot is left alone");
        destroy::<Big>(&mut states);
        assert_eq!(words, [0, 0]);
    }

    /// A destroyed slot's tag is cleared, so destroying it again is a no-op
    /// even though its other bytes are unchanged.
    fn destroying_twice_drops_once<T: AggregateState>() {
        let _serial = tracked();
        let before = TRACKED_LIVE.load(Ordering::SeqCst);
        let mut words = slots::<T>(1, 0);
        let base = words.as_mut_ptr();
        let mut states = [state_at::<T>(base, 0)];
        init::<T>(states[0]);
        assert_ne!(words[0], 0);
        destroy::<T>(&mut states);
        assert_eq!(words[0], 0);
        assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), before);
        destroy::<T>(&mut states);
        assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), before);
    }

    #[test]
    fn destroying_an_inline_state_twice_drops_it_once() {
        destroying_twice_drops_once::<Tracked>();
    }

    #[test]
    fn destroying_a_boxed_state_twice_drops_it_once() {
        destroying_twice_drops_once::<TrackedBig>();
    }

    #[test]
    fn the_initial_value_is_t_default() {
        let _serial = tracked();
        let mut words = slots::<Tracked>(1, usize::MAX);
        let base = words.as_mut_ptr();
        let mut states = [state_at::<Tracked>(base, 0)];
        init::<Tracked>(states[0]);
        // SAFETY: initialised above.
        assert_eq!(
            unsafe { FfiState::<Tracked>::with_state(states[0]) }.map(|t| t.value),
            Some(7)
        );
        destroy::<Tracked>(&mut states);
        let mut words = slots::<TrackedBig>(1, usize::MAX);
        let base = words.as_mut_ptr();
        let mut states = [state_at::<TrackedBig>(base, 0)];
        init::<TrackedBig>(states[0]);
        // SAFETY: initialised above.
        let first = unsafe { FfiState::<TrackedBig>::with_state(states[0]) }.map(|t| t.values[0]);
        assert_eq!(first, Some(7));
        destroy::<TrackedBig>(&mut states);
    }
}
