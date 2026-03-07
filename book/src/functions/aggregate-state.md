# State Management

`FfiState<T>` manages the lifecycle of aggregate state — allocation, initialization, access,
and destruction — so you never write raw pointer code for state management.

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

`FfiState<T>` is a `#[repr(C)]` struct containing a single raw pointer:

```rust
#[repr(C)]
pub struct FfiState<T> {
    inner: *mut T,
}
```

This matches DuckDB's expectation: DuckDB allocates `state_size()` bytes per group,
and your state lives in a `Box<T>` heap allocation whose pointer is stored in that space.

### Memory layout

```
DuckDB-allocated slot (state_size bytes = sizeof(*mut T)):
  [ inner: *mut T ]  ──→  Box<T>  (on the Rust heap)
```

### Lifecycle callbacks

```rust
// state_size: DuckDB calls this once to know how many bytes to allocate per group
FfiState::<MyState>::size_callback(_info)
// Returns: size_of::<*mut MyState>()

// state_init: DuckDB calls this once per group after allocating the slot
FfiState::<MyState>::init_callback(info, state)
// Effect: writes Box::into_raw(Box::new(MyState::default())) into the slot

// state_destroy: DuckDB calls this after finalize for every group
FfiState::<MyState>::destroy_callback(states, count)
// Effect: for each state: drop(Box::from_raw(inner)); inner = null
```

### Accessing state in callbacks

```rust
// Immutable access (in finalize, combine source):
if let Some(st) = FfiState::<MyState>::with_state(state_ptr) {
    let value = st.total;
}

// Mutable access (in update, combine target):
if let Some(st) = FfiState::<MyState>::with_state_mut(state_ptr) {
    st.total += delta;
}
```

Both methods return `Option<&T>` / `Option<&mut T>`. They return `None` if `inner` is
null (which happens after `destroy_callback` or if initialization failed). Using `Option`
rather than panicking on null is what keeps the extension panic-free.

---

## The double-free problem — solved

Without quack-rs, a naive destructor looks like:

```rust
// ❌ Naive — causes double-free if DuckDB calls destroy twice
unsafe extern "C" fn destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    for i in 0..count as usize {
        let ffi = &mut *(*states.add(i) as *mut FfiState<MyState>);
        drop(Box::from_raw(ffi.inner));   // inner is now dangling — crash on second call
    }
}
```

`FfiState::destroy_callback` does:

```rust
// After drop(Box::from_raw(ffi.inner)):
ffi.inner = std::ptr::null_mut();   // ← prevents double-free
```

If DuckDB calls destroy again, `with_state` returns `None` and the loop body is a no-op.

---

## Testing state logic without DuckDB

`AggregateTestHarness<S>` simulates the DuckDB aggregate lifecycle in pure Rust:

```rust
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
