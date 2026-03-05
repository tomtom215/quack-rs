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
//! - **L13**: No panic across FFI — `with_state_mut` returns an `Option`, not a panic.

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
/// `FfiState<T>` is a `#[repr(C)]` struct containing a single raw pointer to a
/// heap-allocated `T`. `DuckDB` allocates `size_of::<FfiState<T>>()` bytes per
/// aggregate group via [`size_callback`][FfiState::size_callback], then calls
/// [`init_callback`][FfiState::init_callback] to initialize each allocation.
///
/// # Memory layout
///
/// ```text
/// FfiState<T> = { inner: *mut T }  // exactly pointer-sized, repr(C)
/// ```
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
}

impl<T: AggregateState> FfiState<T> {
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
    /// # Safety
    ///
    /// - `state` must point to `size_of::<FfiState<T>>()` bytes of writable memory
    ///   allocated by `DuckDB`.
    /// - This function must only be called once per state allocation.
    pub unsafe extern "C" fn init_callback(
        _info: duckdb_function_info,
        state: duckdb_aggregate_state,
    ) {
        // SAFETY: DuckDB allocated `size_of::<FfiState<T>>()` bytes at `state`.
        // We cast it to `*mut FfiState<T>` and write the inner pointer.
        let ffi = unsafe { &mut *(state.cast::<Self>()) };
        ffi.inner = Box::into_raw(Box::<T>::default());
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
    /// # Safety
    ///
    /// - `states` must point to an array of `count` valid `duckdb_aggregate_state`
    ///   pointers, each previously initialized by [`init_callback`][Self::init_callback].
    /// - Each state must not have been freed already (or have `inner == null`).
    pub unsafe extern "C" fn destroy_callback(states: *mut duckdb_aggregate_state, count: idx_t) {
        for i in 0..usize::try_from(count).unwrap_or(0) {
            // SAFETY: `states` is a valid array of `count` pointers.
            let state_ptr = unsafe { *states.add(i) };
            // SAFETY: Each element was initialized by `init_callback` as `*mut Self`.
            let ffi = unsafe { &mut *(state_ptr.cast::<Self>()) };
            if !ffi.inner.is_null() {
                // SAFETY: `inner` was created by `Box::into_raw(Box::new(T::default()))`.
                // We are the only owner; dropping it here is correct.
                unsafe { drop(Box::from_raw(ffi.inner)) };
                // Null out the pointer to prevent double-free if called again.
                ffi.inner = core::ptr::null_mut();
            }
        }
    }

    /// Provides safe mutable access to the inner `T` value.
    ///
    /// Returns `None` if `inner` is null (which should not happen after a
    /// successful `init_callback`, but is checked defensively).
    ///
    /// # Pitfall L13: No panic across FFI
    ///
    /// This method returns `Option<&mut T>` rather than unwrapping, so callers
    /// can use `if let Some(state) = ...` patterns without panicking.
    ///
    /// # Safety
    ///
    /// - `state` must point to a valid `FfiState<T>` allocated by `DuckDB` and
    ///   initialized by [`init_callback`][Self::init_callback].
    /// - No other reference to the same `T` must exist simultaneously.
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

// Note: The pointer-size invariant is verified in unit tests using a concrete
// type that implements AggregateState (see tests::ffi_state_is_pointer_sized).
// A const assertion is not possible here because const fn cannot use trait bounds.

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, Debug, PartialEq)]
    struct Counter {
        value: u64,
    }
    impl AggregateState for Counter {}

    #[test]
    fn ffi_state_is_pointer_sized() {
        assert_eq!(
            core::mem::size_of::<FfiState<Counter>>(),
            core::mem::size_of::<*mut Counter>()
        );
    }

    #[test]
    fn size_returns_pointer_size() {
        assert_eq!(FfiState::<Counter>::size(), core::mem::size_of::<usize>());
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
        };
        let state_ptr = std::ptr::addr_of!(raw) as duckdb_aggregate_state;
        // SAFETY: state_ptr is valid, inner is null.
        let result = unsafe { FfiState::<Counter>::with_state(state_ptr) };
        assert!(result.is_none());
    }

    #[test]
    fn size_callback_returns_pointer_size() {
        // SAFETY: size_callback takes a null-ok info pointer and only reads sizeof.
        let size = unsafe { FfiState::<Counter>::size_callback(core::ptr::null_mut()) };
        assert_eq!(
            usize::try_from(size).unwrap(),
            core::mem::size_of::<usize>()
        );
    }

    #[test]
    fn multiple_state_destroy() {
        // Test destroy_callback with multiple states
        let mut states: Vec<FfiState<Counter>> = (0..4)
            .map(|_| FfiState {
                inner: core::ptr::null_mut(),
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
