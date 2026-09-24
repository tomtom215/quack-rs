# State Management

`FfiState<T>` manages the lifecycle of aggregate state — allocation, initialization, access,
and destruction — so you never write raw pointer code for state management.

> **Known DuckDB limitation.** C-API aggregates — which is to say every
> `FfiState<T>` user — read out of bounds under `agg(x) OVER ()` (whole-partition window frames)
> and `agg(x ORDER BY y)`. This is a DuckDB C API defect; see
> [Aggregate Functions](aggregate.md#known-duckdb-limitation) for the details and
> DuckDB source lines. Do not use C-API aggregates in those two query shapes.
>
> Reported upstream as [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109).

---

## `AggregateState` trait

Any type that is `Default + Send + 'static` can be used as aggregate state by implementing
the `AggregateState` marker trait:

```rust
use quack_rs::aggregate::AggregateState;

#[derive(Default, Debug)]
struct MyState {
    config: usize,    // set in update, must be propagated in combine
    total: i64,       // accumulated data
}

impl AggregateState for MyState {}
```

`AggregateState` has no required methods. The `Default` bound is used in `state_init` to
create fresh states.

---

## `FfiState<T>`

`FfiState<T>` names the layout of the bytes DuckDB allocates for each group's
state, and the callbacks that manage them. It is never constructed itself.

A small `T` — aligned no more strictly than `usize`, and at most 256 bytes —
is stored in those bytes directly. A larger or more strictly aligned `T` is
boxed, and the slot holds the pointer. Either way the slot starts with a tag:

### Memory layout

```text
DuckDB-allocated slot (state_size bytes, a multiple of sizeof(usize)):
  [ tag: usize ][ T, padded to whole words ]      T stored inline
  [ tag: usize ][ *mut T ]                        T boxed
                     │
                     └──→  Box<T>  (on the Rust heap)
```

Storing `T` inline matters because DuckDB 1.4.4 to 1.5.5 does not destroy
every state: when a grouped aggregate's result scan stops early (a `LIMIT`
above it, an error, an interrupt), the states it never reached are never
destroyed. An inline `T`'s bytes are DuckDB's, and DuckDB frees them with the
hash table; only what `T` itself owns on the heap, or a boxed `T`'s box,
leaks. See [Known Limitations](../reference/known-limitations.md).

The tag marks the slot initialised. When one `state_init` call fails — a
panicking `T::default()`, say — DuckDB 1.4.4 to 1.5.5 still runs the
destructor over every state it created, including states whose `state_init`
never ran, so a slot can hold arbitrary bytes. `destroy_callback` drops only a
slot carrying the tag `init_callback` wrote, and clears it first. The tag is
derived from `T` (and a boxed slot's pointer), not from the slot's address,
because DuckDB moves states by copying their bytes. That makes dropping
garbage a matter of chance — uninitialised bytes equal to the tag — rather
than a certainty; see `docs/upstream-duckdb-reports.md` for the DuckDB
defect.

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
// Returns: FfiState::<MyState>::size() (a tag word, then the i64 and usize inline)

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

Both methods return `Option<&T>` / `Option<&mut T>`. They return `None` if the slot's
tag does not match (which happens after `destroy_callback` or if initialization failed). Using `Option`
rather than panicking on null is what keeps the extension panic-free.

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
