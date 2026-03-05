//! Type aliases for the five required `DuckDB` aggregate callback signatures.
//!
//! Import these type aliases when defining your callback functions to ensure
//! you use the correct signature. A mismatch in any parameter type is a silent
//! bug that is extremely difficult to debug.
//!
//! # The five required callbacks
//!
//! Every `DuckDB` aggregate function requires exactly these five callbacks
//! (plus an optional `Destructor`):
//!
//! | Callback | When called | Purpose |
//! |----------|-------------|---------|
//! | [`StateSizeFn`] | Once at registration | Returns `sizeof(FfiState)` in bytes |
//! | [`StateInitFn`] | Per state allocation | Initializes a fresh state |
//! | [`UpdateFn`] | Per batch of rows | Accumulates data from a chunk into the state |
//! | [`CombineFn`] | Parallel merge | Merges source state into target state |
//! | [`FinalizeFn`] | Once at the end | Writes results from states to output vector |
//! | [`DestroyFn`] | After finalize | Frees per-state memory |
//!
//! # Important: Array-of-pointers calling convention
//!
//! The `update`, `combine`, and `finalize` callbacks receive
//! `*mut duckdb_aggregate_state` — a pointer to an **array** of state pointers.
//!
//! # Pitfall: COMBINE must propagate ALL config fields
//!
//! See the crate-level documentation for a worked example of this critical bug.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::aggregate::callbacks::{StateSizeFn, StateInitFn, UpdateFn, CombineFn, FinalizeFn, DestroyFn};
//! use libduckdb_sys::{duckdb_function_info, duckdb_aggregate_state, duckdb_data_chunk, duckdb_vector, idx_t};
//!
//! unsafe extern "C" fn my_state_size(_: duckdb_function_info) -> idx_t { 8 }
//! unsafe extern "C" fn my_init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
//! unsafe extern "C" fn my_update(_: duckdb_function_info, _: duckdb_data_chunk, _: *mut duckdb_aggregate_state) {}
//! unsafe extern "C" fn my_combine(_: duckdb_function_info, _: *mut duckdb_aggregate_state, _: *mut duckdb_aggregate_state, _: idx_t) {}
//! unsafe extern "C" fn my_finalize(_: duckdb_function_info, _: *mut duckdb_aggregate_state, _: duckdb_vector, _: idx_t, _: idx_t) {}
//! unsafe extern "C" fn my_destroy(_: *mut duckdb_aggregate_state, _: idx_t) {}
//!
//! let _: StateSizeFn = my_state_size;
//! let _: StateInitFn = my_init;
//! let _: UpdateFn = my_update;
//! let _: CombineFn = my_combine;
//! let _: FinalizeFn = my_finalize;
//! let _: DestroyFn = my_destroy;
//! ```

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};

/// Returns the size of the aggregate state struct in bytes.
///
/// # Example
///
/// ```rust
/// use quack_rs::aggregate::callbacks::StateSizeFn;
/// use libduckdb_sys::{duckdb_function_info, idx_t};
///
/// unsafe extern "C" fn state_size(_: duckdb_function_info) -> idx_t {
///     std::mem::size_of::<u64>() as idx_t
/// }
/// let _: StateSizeFn = state_size;
/// ```
pub type StateSizeFn = unsafe extern "C" fn(info: duckdb_function_info) -> idx_t;

/// Initializes a freshly allocated aggregate state.
///
/// The `state` parameter is a single `duckdb_aggregate_state` for this group.
///
/// # Example
///
/// ```rust
/// use quack_rs::aggregate::callbacks::StateInitFn;
/// use libduckdb_sys::{duckdb_function_info, duckdb_aggregate_state};
///
/// unsafe extern "C" fn state_init(_: duckdb_function_info, _state: duckdb_aggregate_state) {}
/// let _: StateInitFn = state_init;
/// ```
pub type StateInitFn =
    unsafe extern "C" fn(info: duckdb_function_info, state: duckdb_aggregate_state);

/// Accumulates data from a chunk into the aggregate states.
///
/// `states` is a pointer to an array of state pointers — one per group.
pub type UpdateFn = unsafe extern "C" fn(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
);

/// Merges source states into target states.
///
/// Both `source` and `target` point to arrays of `count` state pointers.
///
/// # Pitfall L1: Combine must propagate ALL config fields
///
/// The `target` states are freshly zero-initialized. All configuration fields
/// must be copied from `source`, not just accumulated data values.
pub type CombineFn = unsafe extern "C" fn(
    info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
);

/// Writes final results from aggregate states to the output vector.
///
/// `source` points to an array of `count` state pointers.
/// Write `count` results starting at `offset` in the output vector.
///
/// # NULL output — Pitfall L4
///
/// Call `duckdb_vector_ensure_validity_writable` before `duckdb_vector_get_validity`
/// when writing NULL values, or use [`VectorWriter::set_null`][crate::vector::VectorWriter::set_null].
pub type FinalizeFn = unsafe extern "C" fn(
    info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
);

/// Frees memory allocated by [`StateInitFn`].
///
/// Called after finalize. Must free all heap allocations made in `StateInitFn`.
pub type DestroyFn = unsafe extern "C" fn(states: *mut duckdb_aggregate_state, count: idx_t);

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn _state_size(_: duckdb_function_info) -> idx_t {
        0
    }
    unsafe extern "C" fn _state_init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
    unsafe extern "C" fn _update(
        _: duckdb_function_info,
        _: duckdb_data_chunk,
        _: *mut duckdb_aggregate_state,
    ) {
    }
    unsafe extern "C" fn _combine(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: *mut duckdb_aggregate_state,
        _: idx_t,
    ) {
    }
    unsafe extern "C" fn _finalize(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: duckdb_vector,
        _: idx_t,
        _: idx_t,
    ) {
    }
    unsafe extern "C" fn _destroy(_: *mut duckdb_aggregate_state, _: idx_t) {}

    #[test]
    fn all_callback_types_compile() {
        let _: StateSizeFn = _state_size;
        let _: StateInitFn = _state_init;
        let _: UpdateFn = _update;
        let _: CombineFn = _combine;
        let _: FinalizeFn = _finalize;
        let _: DestroyFn = _destroy;
    }
}
