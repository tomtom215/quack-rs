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
//! - **L2**: No double-free — `destroy_callback` sets `inner` to null after freeing.
//! - **L3**: No panic across FFI — `with_state_mut` returns an `Option`, not a panic.

use libduckdb_sys::{duckdb_aggregate_state, duckdb_function_info, idx_t};

/// Trait for types that can be used as `DuckDB` aggregate state.
///
/// Implement this for your state struct. The only requirement is `Default`
/// (used to create the initial state in `state_init`) and `Send` (since `DuckDB`
/// may call `combine` across threads).
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
pub trait AggregateState: Default + Send + 'static {}

/// A generic FFI-compatible state wrapper for use with `DuckDB` aggregate functions.
///
/// `FfiState<T>` is a `#[repr(C)]` struct holding a raw pointer to a
/// heap-allocated `T` and a tag that marks the slot initialised. `DuckDB` allocates `size_of::<FfiState<T>>()` bytes per
/// aggregate group via [`size_callback`][FfiState::size_callback], then calls
/// [`init_callback`][FfiState::init_callback] to initialize each allocation.
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
/// FfiState<T> = { inner: *mut T, tag: usize }  // two words, repr(C)
/// ```
///
/// # States `DuckDB` never initialised
///
/// When a `state_init` call fails — a panicking `T::default()`, or an
/// allocation failure — `DuckDB` 1.4.4 to 1.5.5 still calls the destructor
/// on every state row it had created, initialised or not
/// (`RowOperations::InitializeStates` is not exception-safe; see
/// `docs/upstream-duckdb-reports.md`). That includes the states of *other*
/// aggregates in the same query whose `state_init` never ran. Those bytes
/// are whatever the allocation held, and treating a stale `inner` as a
/// `Box<T>` would free a wild pointer.
///
/// So [`init_callback`][Self::init_callback] also stores `tag`, a value
/// derived from the slot's own address, and
/// [`destroy_callback`][Self::destroy_callback] frees only a slot whose tag
/// matches, clearing it first. A slot the destructor has already processed
/// no longer matches; a stale tag can survive only in a slot that was never
/// destroyed, whose `T` is therefore still allocated. What is left is chance:
/// uninitialised bytes that happen to equal this slot's tag, one value in
/// 2<sup>64</sup> on a 64-bit target. This is a mitigation of a `DuckDB` defect,
/// not a guarantee.
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
/// // .state_size(FfiState::<MyState>::size_callback)
/// // .init(FfiState::<MyState>::init_callback)
/// // .destructor(FfiState::<MyState>::destroy_callback)
/// ```
#[repr(C)]
pub struct FfiState<T: AggregateState> {
    /// Raw pointer to the heap-allocated `T` value.
    ///
    /// - Set to non-null by [`init_callback`][FfiState::init_callback].
    /// - Set to null after freeing by [`destroy_callback`][FfiState::destroy_callback].
    pub inner: *mut T,
    /// [`tag_for`][Self::tag_for] this slot's address once
    /// [`init_callback`][FfiState::init_callback] has initialised it, and zero
    /// once [`destroy_callback`][FfiState::destroy_callback] has processed it.
    tag: usize,
}

impl<T: AggregateState> FfiState<T> {
    /// Mixed into a slot's address to make its tag: an arbitrary odd constant,
    /// so that neither a zeroed slot nor one holding its own address matches.
    #[cfg(target_pointer_width = "64")]
    const TAG_KEY: usize = 0x9E37_79B9_7F4A_7C15;
    #[cfg(not(target_pointer_width = "64"))]
    const TAG_KEY: usize = 0x9E37_79B9;

    /// The tag an initialised state at `slot` carries.
    fn tag_for(slot: *const Self) -> usize {
        (slot as usize) ^ Self::TAG_KEY
    }

    /// Returns the size of `FfiState<T>` in bytes, for use as the `state_size` callback.
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
    /// let size = FfiState::<MyState>::size();
    /// assert_eq!(size, std::mem::size_of::<FfiState<MyState>>());
    /// ```
    #[inline]
    #[must_use]
    pub const fn size() -> usize {
        core::mem::size_of::<Self>()
    }

    /// The `state_size` callback function for use in the builder.
    ///
    /// Returns the number of bytes `DuckDB` must allocate per aggregate group.
    ///
    /// # Safety
    ///
    /// This is an `unsafe extern "C"` function pointer. It is safe to pass to
    /// [`AggregateFunctionBuilder::state_size`][crate::aggregate::AggregateFunctionBuilder::state_size].
    pub const unsafe extern "C" fn size_callback(_info: duckdb_function_info) -> idx_t {
        core::mem::size_of::<Self>() as idx_t
    }

    /// The `state_init` callback function for use in the builder.
    ///
    /// Allocates a `T::default()` on the heap and stores the raw pointer in
    /// the `FfiState` at `state`.
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
    /// `DuckDB` does not promise the state allocation is zeroed, so the slot is
    /// written with a null `inner` *before* any user code runs. That keeps
    /// [`with_state`][Self::with_state] and
    /// [`destroy_callback`][Self::destroy_callback] sound even on the panicking
    /// path, and avoids ever forming a `&mut Self` over uninitialised bytes.
    ///
    /// # Safety
    ///
    /// - `state` must point to `size_of::<FfiState<T>>()` bytes of writable memory
    ///   allocated by `DuckDB`.
    /// - This function must only be called once per state allocation.
    pub unsafe extern "C" fn init_callback(
        info: duckdb_function_info,
        state: duckdb_aggregate_state,
    ) {
        let slot = state.cast::<Self>();
        // SAFETY: DuckDB allocated `size_of::<FfiState<T>>()` bytes at `state`.
        // `ptr::write` initialises them without reading what was there, which
        // `&mut *slot` would have required.
        unsafe {
            core::ptr::write(
                slot,
                Self {
                    inner: core::ptr::null_mut(),
                    tag: Self::tag_for(slot),
                },
            );
        }

        match crate::callback::catch_ffi_panic(Box::<T>::default) {
            Ok(boxed) => {
                // SAFETY: `slot` was initialised immediately above.
                unsafe { (*slot).inner = Box::into_raw(boxed) };
            }
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
    /// Frees the heap-allocated `T` for each state in `states[0..count]`.
    /// Sets `inner` to null after freeing to prevent double-free.
    ///
    /// # Pitfall L2: No double-free
    ///
    /// After `Box::from_raw`, we set `inner = null` so that if `destroy_callback`
    /// is accidentally called twice, the second call is a no-op.
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
    /// discarded. Every remaining state is still freed: one bad destructor does
    /// not leak the rest of the group.
    ///
    /// One case cannot be contained: a `T::drop` that panics while one of
    /// `T`'s fields also panics in its own `drop` during that unwind. Rust
    /// aborts the process for a panic raised while unwinding ("panic in a
    /// destructor during cleanup") before any `catch_unwind` sees it.
    ///
    /// # Safety
    ///
    /// - `states` must point to an array of `count` valid `duckdb_aggregate_state`
    ///   pointers, each to `size_of::<FfiState<T>>()` bytes that `DuckDB`
    ///   allocated for this aggregate. A slot [`init_callback`][Self::init_callback]
    ///   never initialised is skipped (see "States `DuckDB` never initialised"
    ///   on the type).
    /// - Each state must not have been freed already (or have `inner == null`).
    pub unsafe extern "C" fn destroy_callback(states: *mut duckdb_aggregate_state, count: idx_t) {
        for i in 0..usize::try_from(count).unwrap_or(0) {
            // SAFETY: `states` is a valid array of `count` pointers.
            let state_ptr = unsafe { *states.add(i) };
            let slot = state_ptr.cast::<Self>();
            // SAFETY: the slot is `size_of::<Self>()` bytes DuckDB allocated
            // for this aggregate, so both words are in bounds. They may never
            // have been written by `init_callback`, so they are read as plain
            // words with volatile loads, which the compiler must perform as
            // written and cannot reason about, and only trusted once the tag
            // matches.
            let (tag, inner) = unsafe {
                (
                    core::ptr::read_volatile(core::ptr::addr_of!((*slot).tag)),
                    core::ptr::read_volatile(core::ptr::addr_of!((*slot).inner)),
                )
            };
            if tag != Self::tag_for(slot) {
                continue;
            }
            // Clear the tag and null the pointer *before* dropping: if
            // `T::drop` panics, the allocation is still released by the
            // unwind, so leaving the slot live would make a second destructor
            // call a double free.
            // SAFETY: the tag matched, so `init_callback` initialised the slot.
            unsafe {
                (*slot).tag = 0;
                (*slot).inner = core::ptr::null_mut();
            }
            if inner.is_null() {
                continue;
            }
            // SAFETY: `inner` was created by `Box::into_raw(Box::new(T::default()))`.
            // We are the only owner; dropping it here is correct. The drop runs
            // arbitrary user code, so it must not be allowed to unwind out of
            // this `extern "C"` function.
            drop(crate::callback::catch_ffi_panic(|| unsafe {
                drop(Box::from_raw(inner));
            }));
        }
    }

    /// Provides safe mutable access to the inner `T` value.
    ///
    /// Returns `None` if `inner` is null (which should not happen after a
    /// successful `init_callback`, but is checked defensively).
    ///
    /// # Pitfall L3: No panic across FFI
    ///
    /// This method returns `Option<&mut T>` rather than unwrapping, so callers
    /// can use `if let Some(state) = ...` patterns without panicking.
    ///
    /// # Safety
    ///
    /// - `state` must point to a valid `FfiState<T>` allocated by `DuckDB` and
    ///   initialized by [`init_callback`][Self::init_callback].
    /// - No other reference to the same `T` must exist simultaneously.
    /// - The returned reference must not outlive the callback that received
    ///   `state`. The lifetime `'a` is the caller's to choose and nothing ties
    ///   it to `state`; `DuckDB` destroys the state — and
    ///   [`destroy_callback`][Self::destroy_callback] frees the `T` — once it
    ///   is finalized or merged.
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
        // SAFETY: Caller guarantees `state` points to a valid `FfiState<T>`.
        let ffi = unsafe { &mut *state.cast::<Self>() };
        if ffi.inner.is_null() {
            return None;
        }
        // SAFETY: `inner` is non-null and was allocated by Box::into_raw.
        // Caller guarantees no other references exist.
        Some(unsafe { &mut *ffi.inner })
    }

    /// Provides safe immutable access to the inner `T` value.
    ///
    /// See [`with_state_mut`][Self::with_state_mut] for safety requirements.
    ///
    /// # Safety
    ///
    /// Same as `with_state_mut`, but only borrows immutably.
    pub unsafe fn with_state<'a>(state: duckdb_aggregate_state) -> Option<&'a T> {
        // SAFETY: Same invariants as with_state_mut.
        let ffi = unsafe { &*state.cast::<Self>() };
        if ffi.inner.is_null() {
            return None;
        }
        // SAFETY: inner is non-null.
        Some(unsafe { &*ffi.inner })
    }
}

// Note: The two-word layout is verified in unit tests using a concrete
// type that implements AggregateState (see tests::ffi_state_is_two_words).
// A const assertion is not possible here because const fn cannot use trait bounds.

impl<T: AggregateState> core::fmt::Debug for FfiState<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FfiState")
            .field("state", &core::any::type_name::<T>())
            .field("inner", &self.inner)
            .field("tag", &self.tag)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, Debug, PartialEq)]
    struct Counter {
        value: u64,
    }
    impl AggregateState for Counter {}

    #[test]
    fn ffi_state_is_two_words() {
        assert_eq!(
            core::mem::size_of::<FfiState<Counter>>(),
            2 * core::mem::size_of::<usize>()
        );
    }

    #[test]
    fn size_returns_two_words() {
        assert_eq!(
            FfiState::<Counter>::size(),
            2 * core::mem::size_of::<usize>()
        );
    }

    /// The fourth audit's F3. `DuckDB` destroys states whose `state_init`
    /// never ran when another init fails, so a slot can hold any bytes. A
    /// slot whose tag does not match — here a non-null `inner` that was never
    /// allocated — is skipped, not freed. Under Miri, freeing it would be
    /// reported as undefined behaviour.
    #[test]
    fn destroy_skips_a_slot_init_never_initialised() {
        let bogus = core::ptr::NonNull::<Counter>::dangling().as_ptr();
        let mut raw: FfiState<Counter> = FfiState {
            inner: bogus,
            tag: 0x5a5a_5a5a,
        };
        let state_ptr = std::ptr::addr_of_mut!(raw) as duckdb_aggregate_state;
        let mut state_arr: [duckdb_aggregate_state; 1] = [state_ptr];
        // SAFETY: the slot is a live `FfiState<Counter>`; its contents are
        // exactly what the destructor must refuse to trust.
        unsafe { FfiState::<Counter>::destroy_callback(state_arr.as_mut_ptr(), 1) };
        assert_eq!(raw.inner, bogus, "an untagged slot is left alone");
        // A tag copied from another slot does not match this one either.
        let other: FfiState<Counter> = FfiState {
            inner: core::ptr::null_mut(),
            tag: 0,
        };
        // Written through `state_ptr`, not `raw`: a direct write to `raw` would
        // invalidate the pointer the destructor is about to use (Miri, Stacked
        // Borrows).
        // SAFETY: `state_ptr` points at `raw`, a live `FfiState<Counter>`.
        unsafe {
            (*state_ptr.cast::<FfiState<Counter>>()).tag =
                FfiState::<Counter>::tag_for(std::ptr::addr_of!(other));
        }
        // SAFETY: as above.
        unsafe { FfiState::<Counter>::destroy_callback(state_arr.as_mut_ptr(), 1) };
        assert_eq!(raw.inner, bogus, "another slot's tag does not match");
    }

    /// `DuckDB` lays a group's states out contiguously, `size()` bytes apart,
    /// so a slot's neighbours are the likeliest source of a stale tag. Every
    /// slot's tag differs from every other's, and a neighbour's tag is not
    /// accepted. Eight adjacent slots (16 bytes each on 64-bit targets, 8 on
    /// 32-bit) always include two whose addresses differ only in the slot-size
    /// bit (bit 4, or bit 3), and `TAG_KEY` sets that bit on both: a tag built
    /// with `|` instead of `^` gives those two the same tag.
    #[test]
    fn adjacent_slots_never_share_a_tag() {
        let bogus = core::ptr::NonNull::<Counter>::dangling().as_ptr();
        let mut slots: [FfiState<Counter>; 8] = core::array::from_fn(|_| FfiState {
            inner: bogus,
            tag: 0,
        });
        let base = slots.as_mut_ptr();
        // SAFETY (all `add`s below): `i` and `j` are below `slots.len()`.
        let tags: Vec<usize> = (0..slots.len())
            .map(|i| FfiState::<Counter>::tag_for(unsafe { base.add(i) }))
            .collect();
        for i in 0..tags.len() {
            for j in i + 1..tags.len() {
                assert_ne!(tags[i], tags[j], "slots {i} and {j} share a tag");
            }
        }
        for i in 0..slots.len() {
            for j in (0..slots.len()).filter(|&j| j != i) {
                // SAFETY: `base.add(i)` is a live `FfiState<Counter>` in
                // `slots`, accessed only through `base` from here on.
                unsafe { (*base.add(i)).tag = tags[j] };
                let mut state_arr = [unsafe { base.add(i) } as duckdb_aggregate_state];
                // SAFETY: as above; the slot is either skipped or, if the tag
                // were wrongly accepted, `inner` would be freed (Miri reports it).
                unsafe { FfiState::<Counter>::destroy_callback(state_arr.as_mut_ptr(), 1) };
                // SAFETY: as above.
                let inner = unsafe { (*base.add(i)).inner };
                assert_eq!(inner, bogus, "slot {i} accepted slot {j}'s tag");
            }
        }
    }

    /// A destroyed slot's tag is cleared, so destroying it again is a no-op
    /// even though its bytes are otherwise unchanged.
    #[test]
    fn destroy_clears_the_tag() {
        let mut raw: FfiState<Counter> = FfiState {
            inner: core::ptr::null_mut(),
            tag: 0,
        };
        let state_ptr = std::ptr::addr_of_mut!(raw) as duckdb_aggregate_state;
        // SAFETY: a live, writable slot of the right size.
        unsafe { FfiState::<Counter>::init_callback(core::ptr::null_mut(), state_ptr) };
        assert_eq!(
            raw.tag,
            FfiState::<Counter>::tag_for(std::ptr::addr_of!(raw))
        );
        let mut state_arr: [duckdb_aggregate_state; 1] = [state_ptr];
        // SAFETY: initialised above.
        unsafe { FfiState::<Counter>::destroy_callback(state_arr.as_mut_ptr(), 1) };
        assert_eq!((raw.tag, raw.inner), (0, core::ptr::null_mut()));
    }

    #[test]
    fn init_and_destroy_lifecycle() {
        // Simulate what DuckDB does:
        // 1. Allocate state_size() bytes
        // 2. Call init_callback
        // 3. Use with_state_mut
        // 4. Call destroy_callback

        // Step 1: allocate
        let mut raw: FfiState<Counter> = FfiState {
            inner: core::ptr::null_mut(),
            tag: 0,
        };
        let state_ptr = std::ptr::addr_of_mut!(raw) as duckdb_aggregate_state;

        // Step 2: init
        unsafe { FfiState::<Counter>::init_callback(core::ptr::null_mut(), state_ptr) };
        assert!(!raw.inner.is_null());

        // Step 3: access
        // SAFETY: state_ptr is valid and inner is initialized.
        let s = unsafe { FfiState::<Counter>::with_state_mut(state_ptr) };
        assert!(s.is_some());
        if let Some(counter) = s {
            counter.value = 42;
        }

        // Verify the value was set
        let s2 = unsafe { FfiState::<Counter>::with_state(state_ptr) };
        assert_eq!(s2.map(|c| c.value), Some(42));

        // Step 4: destroy
        let mut state_arr: [duckdb_aggregate_state; 1] = [state_ptr];
        unsafe {
            FfiState::<Counter>::destroy_callback(state_arr.as_mut_ptr(), 1);
        }
        // After destroy, inner must be null (double-free prevention).
        assert!(raw.inner.is_null());
    }

    #[test]
    fn destroy_null_inner_is_noop() {
        let mut raw: FfiState<Counter> = FfiState {
            inner: core::ptr::null_mut(),
            tag: 0,
        };
        let state_ptr = std::ptr::addr_of_mut!(raw) as duckdb_aggregate_state;
        let mut state_arr: [duckdb_aggregate_state; 1] = [state_ptr];
        // Calling destroy on an uninitialized (null inner) state must not crash.
        unsafe {
            FfiState::<Counter>::destroy_callback(state_arr.as_mut_ptr(), 1);
        }
        assert!(raw.inner.is_null());
    }

    #[test]
    fn with_state_mut_null_inner_returns_none() {
        let mut raw: FfiState<Counter> = FfiState {
            inner: core::ptr::null_mut(),
            tag: 0,
        };
        let state_ptr = std::ptr::addr_of_mut!(raw) as duckdb_aggregate_state;
        // SAFETY: state_ptr is valid, inner is null.
        let result = unsafe { FfiState::<Counter>::with_state_mut(state_ptr) };
        assert!(result.is_none());
    }

    #[test]
    fn with_state_null_inner_returns_none() {
        let raw: FfiState<Counter> = FfiState {
            inner: core::ptr::null_mut(),
            tag: 0,
        };
        let state_ptr = std::ptr::addr_of!(raw) as duckdb_aggregate_state;
        // SAFETY: state_ptr is valid, inner is null.
        let result = unsafe { FfiState::<Counter>::with_state(state_ptr) };
        assert!(result.is_none());
    }

    #[test]
    fn size_callback_returns_two_words() {
        // SAFETY: size_callback takes a null-ok info pointer and only reads sizeof.
        let size = unsafe { FfiState::<Counter>::size_callback(core::ptr::null_mut()) };
        assert_eq!(
            usize::try_from(size).unwrap(),
            2 * core::mem::size_of::<usize>()
        );
    }

    #[test]
    fn multiple_state_destroy() {
        // Test destroy_callback with multiple states
        let mut states: Vec<FfiState<Counter>> = (0..4)
            .map(|_| FfiState {
                inner: core::ptr::null_mut(),
                tag: 0,
            })
            .collect();

        let mut ptrs: Vec<duckdb_aggregate_state> = states
            .iter_mut()
            .map(|s| std::ptr::from_mut::<FfiState<Counter>>(s) as duckdb_aggregate_state)
            .collect();

        // Initialize all
        for &ptr in &ptrs {
            unsafe { FfiState::<Counter>::init_callback(core::ptr::null_mut(), ptr) };
        }
        for s in &states {
            assert!(!s.inner.is_null());
        }

        // Destroy all
        unsafe {
            FfiState::<Counter>::destroy_callback(ptrs.as_mut_ptr(), 4);
        }

        // All should be null
        for s in &states {
            assert!(s.inner.is_null());
        }
    }
}
