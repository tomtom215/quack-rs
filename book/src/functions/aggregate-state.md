# Aggregate State

This page covers aggregate state in a DuckDB aggregate function written in Rust:
the `AggregateState` trait and `FfiState<T>`, which manages each state's lifecycle
(allocation, initialisation, access and destruction) so that you do not write
raw-pointer code for it.

> **Known DuckDB limitation.** Every C-API aggregate, and so every aggregate that
> uses `FfiState<T>`, reads out of bounds under `agg(x) OVER ()` (whole-partition
> window frames) and `agg(x ORDER BY y)`. This is a DuckDB C API defect; see
> [Aggregate Functions](aggregate.md#known-duckdb-limitation) for the details and
> DuckDB source lines. Do not use C-API aggregates in those two query shapes.
>
> Reported upstream as [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109).

---

## `AggregateState` trait

Any type that is `Default + Send + Sync + 'static` can be used as aggregate state by
implementing the `AggregateState` marker trait. The `Sync` bound is new in 0.18.0: a
window's segment tree lets several threads read the same state as a `combine` source at
once, so a state containing a `Cell` or `RefCell` would race. Use atomics or a `Mutex`
instead, or keep that data outside the state.

```rust
use quack_rs::aggregate::AggregateState;

#[derive(Default, Debug)]
struct MyState {
    config: usize,    // set in update, must be propagated in combine
    total: i64,       // accumulated data
}

impl AggregateState for MyState {}
```

`AggregateState` has no required methods. `state_init` uses `Default` to create each
fresh state.

---

## `FfiState<T>`

`FfiState<T>` names the layout of the bytes DuckDB allocates for each group's
state, and the callbacks that manage them. The type itself is never constructed.

A small `T`, aligned no more strictly than `usize` and at most 256 bytes, is
stored in those bytes directly. A larger or more strictly aligned `T` is boxed,
and the slot holds the pointer. On wasm32, where `usize` is 4 bytes but `u64`,
`i64` and `f64` are 8-byte aligned, a state containing one of them is therefore
boxed. Either way the slot starts with a tag.

### Memory layout

```text
DuckDB-allocated slot (state_size bytes, a multiple of sizeof(usize)):
  [ tag: usize ][ T, padded to whole words ]      T stored inline
  [ tag: usize ][ *mut T ]                        T boxed
                     │
                     └──→  Box<T>  (on the Rust heap)
```

Storing `T` inline matters because DuckDB 1.4.4 to 1.5.5 does not destroy
every state. When a grouped aggregate's result scan stops early (a `LIMIT`
above it, an error, an interrupt), the states it never reached are never
destroyed; so is one state per row of a window frame with `EXCLUDE`. An inline
`T`'s bytes belong to DuckDB, which frees them with the hash table; only what
`T` itself owns on the heap, or a boxed `T`'s box, leaks. See
[Known Limitations](../reference/known-limitations.md#grouped-aggregate-states-the-scan-never-reaches-are-never-destroyed-duckdb-defect).

The tag marks the slot initialised. When one `state_init` call fails (a
panicking `T::default()`, say), DuckDB 1.4.4 to 1.5.5 still runs the
destructor over every state it created, including states whose `state_init`
never ran, so a slot can hold arbitrary bytes. `destroy_callback` drops only a
slot carrying the tag `init_callback` wrote, and clears the tag first. The tag
is derived from a hash of `T`'s `TypeId` (and, for a boxed `T`, the box's
address), not from the slot's address, because DuckDB moves states by copying
their bytes. The check turns dropping garbage from a certainty into a matter
of chance: uninitialised bytes that happen to equal the tag. It mitigates the
DuckDB defect (described in the repository's `docs/upstream-duckdb-reports.md`);
it cannot guarantee against it.

### Lifecycle callbacks

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::aggregate::{AggregateState, FfiState};
# #[derive(Default, Debug)] struct MyState { config: usize, total: i64 }
# impl AggregateState for MyState {}
# unsafe fn demo(_info: duckdb_function_info, info: duckdb_function_info,
#     state: duckdb_aggregate_state, states: *mut duckdb_aggregate_state, count: idx_t) {
// state_size: DuckDB calls this whenever an operator sizes its state buffers
FfiState::<MyState>::size_callback(_info);
// Returns: FfiState::<MyState>::size() (on a 64-bit target, a tag word, then the
// usize and i64 inline)

// state_init: DuckDB calls this for every state slot it allocates, combine
// targets included
FfiState::<MyState>::init_callback(info, state);
// Effect: writes MyState::default() into the slot (or a box holding it), then the tag

// destructor: DuckDB calls this after finalize, on combine's source states once
// merged, and (after a failed state_init) on states never initialised, which
// the tag makes it skip; not on every state (see Known Limitations)
FfiState::<MyState>::destroy_callback(states, count);
// Effect: for each state whose tag matches: clear the tag, then drop the T
# }
```

### Wiring them up: `ffi_state::<T>()`

The recommended way to register those three callbacks is `ffi_state::<T>()`
(new in 0.18.0). It installs `size_callback`, `init_callback` and
`destroy_callback` for the same `T` in one call, so the size DuckDB allocates
and the state `init` writes cannot disagree. Wired one by one with the
`state_size`, `init` and `destructor` setters, a size callback for one type
paired with an init callback for a larger one writes past DuckDB's allocation.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_connection, duckdb_data_chunk,
#     duckdb_function_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# #[derive(Default, Debug)] struct MyState { config: usize, total: i64 }
# impl AggregateState for MyState {}
# unsafe extern "C" fn update(_: duckdb_function_info, _: duckdb_data_chunk, _: *mut duckdb_aggregate_state) {}
# unsafe extern "C" fn combine(_: duckdb_function_info, _: *mut duckdb_aggregate_state, _: *mut duckdb_aggregate_state, _: idx_t) {}
# unsafe extern "C" fn finalize(_: duckdb_function_info, _: *mut duckdb_aggregate_state, _: duckdb_vector, _: idx_t, _: idx_t) {}
unsafe fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        AggregateFunctionBuilder::new("my_agg")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .ffi_state::<MyState>()   // state_size + init + destructor
            .update(update)
            .combine(combine)
            .finalize(finalize)
            .register(con)?;
    }
    Ok(())
}
```

`AggregateOverloadBuilder` has the same method, for each overload of an
[`AggregateFunctionSetBuilder`](aggregate-sets.md); the set builder itself has
none. `update`, `combine` and `finalize` still read the state through
`FfiState::<T>::with_state` / `with_state_mut` with the same `T`.

### Accessing state in callbacks

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::aggregate::{AggregateState, FfiState};
# #[derive(Default, Debug)] struct MyState { config: usize, total: i64 }
# impl AggregateState for MyState {}
# unsafe fn demo(state_ptr: duckdb_aggregate_state, delta: i64) {
// Immutable access (in finalize, combine source):
if let Some(st) = FfiState::<MyState>::with_state(state_ptr) {
    let value = st.total;
}

// Mutable access (in update, combine target):
if let Some(st) = FfiState::<MyState>::with_state_mut(state_ptr) {
    st.total += delta;
}
# }
```

The methods return `Option<&T>` and `Option<&mut T>` respectively: `None` if the
slot's tag does not match, which happens after `destroy_callback` has run or when
`T::default()` panicked in `state_init`. Returning `Option` instead of panicking
keeps a panic from unwinding across the FFI boundary
([Pitfall L3](../reference/pitfalls.md#l3-no-panic-across-ffi-boundaries)).

---

## The double-free problem — solved

Without quack-rs, a naive destructor looks like:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, idx_t};
# struct MyState;
# // A hand-written boxed layout: this is the code *without* quack-rs.
# #[repr(C)] struct FfiState<T> { inner: *mut T }
// ❌ Naive — causes double-free if DuckDB calls destroy twice
unsafe extern "C" fn destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    for i in 0..count as usize {
        let ffi = &mut *(*states.add(i) as *mut FfiState<MyState>);
        drop(Box::from_raw(ffi.inner));   // inner is now dangling — crash on second call
    }
}
```

`FfiState::destroy_callback` clears the slot's tag *before* dropping the `T`,
and drops only a slot whose tag matches. If DuckDB calls destroy again, the tag
no longer matches, the slot is skipped, and `with_state` returns `None`.

---

## Testing state logic without DuckDB

`AggregateTestHarness<S>` simulates the DuckDB aggregate lifecycle in pure Rust:

```rust,test_harness
# use quack_rs::aggregate::AggregateState;
# #[derive(Default, Debug)] struct MyState { config: usize, total: i64 }
# impl AggregateState for MyState {}
use quack_rs::testing::AggregateTestHarness;

#[test]
fn combine_propagates_config() {
    let mut source = AggregateTestHarness::<MyState>::new();
    source.update(|s| {
        s.config = 5;    // config field set during update
        s.total += 100;
    });

    let mut target = AggregateTestHarness::<MyState>::new();
    target.combine(&source, |src, tgt| {
        tgt.config = src.config;   // must propagate config — Pitfall L1
        tgt.total  += src.total;
    });

    let result = target.finalize();
    assert_eq!(result.config, 5, "config must be propagated in combine");
    assert_eq!(result.total, 100);
}
```

See the [Testing Guide](../testing.md) for the full test strategy.
