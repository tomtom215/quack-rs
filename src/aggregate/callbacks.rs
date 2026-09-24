// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

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
//! | [`StateSizeFn`] | Whenever an operator sizes its state buffers — at least once per query that uses the aggregate, never at registration | Returns `sizeof(FfiState)` in bytes; must return the same value every time |
//! | [`StateInitFn`] | Per state allocation, including every `combine` target | Initializes a fresh state |
//! | [`UpdateFn`] | Per batch of input rows | Accumulates data from a chunk into the states |
//! | [`CombineFn`] | Parallel merge, window segment trees | Merges source states into target states |
//! | [`FinalizeFn`] | Per batch of result rows, with a `count` and an `offset` | Writes results from `count` states to the output vector starting at `offset` |
//! | [`DestroyFn`] | For every state `DuckDB` created: after finalize, on the source states once `combine` has merged them — and, after a failed `state_init`, on states that were never initialised | Frees per-state memory |
//!
//! Sources (`DuckDB` 1.5.5): `CAPIAggregateStateSize`, `CAPIAggregateFinalize`
//! and `CAPIAggregateDestructor` in `src/main/capi/aggregate_function-c.cpp`;
//! the state-size calls in `aggregate_object.cpp`,
//! `physical_ungrouped_aggregate.cpp` and the window executors; and
//! `LocalUngroupedAggregateState`, whose states are destroyed after
//! `GlobalUngroupedAggregateState::Combine` has merged them
//! (`physical_ungrouped_aggregate.cpp`).
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
/// `states` is a pointer to an array of state pointers — one per **input row**
/// (rows of the same group carry the same pointer), so `states[row]` is the
/// state for row `row` of `input`.
///
/// The chunk holds **every** input row, NULL rows included, whatever
/// [`NullHandling`][crate::types::NullHandling] the aggregate was registered
/// with: check [`VectorReader::is_valid`][crate::vector::VectorReader::is_valid]
/// before reading a value.
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
/// A `target` state is not a copy of anything: it was set up by your
/// [`StateInitFn`] (for [`FfiState<T>`][crate::aggregate::FfiState], that is
/// `T::default()`) and may have seen no `update` at all. All configuration
/// fields must be copied from `source`, not just accumulated data values.
///
/// After `combine` returns, `DuckDB` may call [`DestroyFn`] on the `source`
/// states, so move out of them rather than keeping pointers into them.
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
/// Called for every state `DuckDB` created — after finalize, but also on the
/// source states of a [`CombineFn`] once they have been merged, and on states
/// that are never finalized. Must free all heap allocations made in
/// `StateInitFn`.
///
/// # Not every state it receives was initialised
///
/// When a [`StateInitFn`] call fails, `DuckDB` 1.4.4 to 1.5.5 still destroys
/// every state row it had created, including rows whose `state_init` never ran
/// — for this aggregate and for any other in the same query
/// (`RowOperations::InitializeStates` is not exception-safe). In one run
/// against 1.5.5, a failure at init call 3001 sent 4096 states to the
/// destructor, 1096 of them never initialised. A destructor that frees a
/// pointer stored by `state_init` must therefore recognise a slot `state_init`
/// did not write; [`FfiState`][crate::aggregate::FfiState] does so with a tag.
/// The same failure also leaves most initialised states undestroyed, so what
/// they own leaks; see `docs/upstream-duckdb-reports.md`.
pub type DestroyFn = unsafe extern "C" fn(states: *mut duckdb_aggregate_state, count: idx_t);

/// The destructor registered for an aggregate whose builder was given none.
///
/// It does nothing, and it exists for its presence: `DuckDB` evaluates an
/// aggregate with **no** state destructor as a *streaming* window
/// (`PhysicalStreamingWindow::IsStreamingFunction`) for
/// `agg(x) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)` and other
/// running frames, and that path gives every C API aggregate wrong answers:
/// it slices the argument chunk to one row per `update` call, and
/// `CAPIAggregateUpdate` flattens that slice in place on the first call, so
/// every later row of the chunk re-reads row 0 (Pitfall L13). Any destructor
/// takes the aggregate off that path. A no-op cannot fail, touches no state
/// and cannot unwind.
pub(crate) const unsafe extern "C" fn no_op_destroy(
    _states: *mut duckdb_aggregate_state,
    _count: idx_t,
) {
}

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
