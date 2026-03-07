# Pitfall Catalog

All known DuckDB Rust FFI pitfalls, discovered while building
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral), a
production DuckDB community extension. Every future developer who builds a Rust
DuckDB extension will hit the majority of these. quack-rs makes most of them
impossible.

---

## L1: COMBINE must propagate ALL config fields

**Status**: Testable with `AggregateTestHarness`.

**Symptom**: Aggregate function returns wrong results. No error, no crash.

**Root cause**: DuckDB's segment tree creates fresh **zero-initialized** target
states via `state_init`, then calls `combine` to merge source states into them.
If your `combine` only propagates data fields (`count`, `sum`) but omits
configuration fields (`window_size`, `mode`), the configuration will be zero at
`finalize` time, silently corrupting results.

This bug passed 435 unit tests before being caught by E2E tests.

**Fix**:

```rust
unsafe extern "C" fn combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        if let (Some(src), Some(tgt)) = (
            FfiState::<MyState>::with_state(src_ptr),
            FfiState::<MyState>::with_state_mut(tgt_ptr),
        ) {
            tgt.window_size = src.window_size;  // config — MUST copy
            tgt.mode = src.mode;                // config — MUST copy
            tgt.count += src.count;             // data — accumulate
        }
    }
}
```

Test this with `AggregateTestHarness::combine` — see [Testing Guide](../testing.md).

---

## L2: State destroy double-free

**Status**: Made impossible by `FfiState<T>`.

**Symptom**: Crash or memory corruption on extension unload.

**Root cause**: If `state_destroy` frees the inner `Box` but does not null the
pointer, a second `state_destroy` call (common in error paths) frees
already-freed memory → undefined behavior.

**Fix**: `FfiState<T>::destroy_callback` nulls `inner` after freeing. Use it
instead of writing your own destructor:

```rust
unsafe extern "C" fn state_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<MyState>::destroy_callback(states, count) };
}
```

---

## L3: No panic across FFI boundaries

**Status**: Made impossible by `init_extension` and `panic = "abort"`.

**Symptom**: Extension causes DuckDB to crash or behave unpredictably.

**Root cause**: `panic!()` and `.unwrap()` in `unsafe extern "C"` functions is
undefined behavior. Panics cannot unwind across FFI boundaries in Rust.

**Fix**: Use `Result` and `?` inside `init_extension`. Never use `unwrap()` in
FFI callbacks. `FfiState::with_state_mut` returns `Option`, not `Result`, so
callers use `if let`:

```rust
// Safe pattern — no unwrap in FFI callback
if let Some(st) = unsafe { FfiState::<MyState>::with_state_mut(state_ptr) } {
    st.count += 1;
}

// Dangerous — never do this in an FFI callback
let st = unsafe { FfiState::<MyState>::with_state_mut(state_ptr) }.unwrap(); // UB if None
```

The scaffold-generated `Cargo.toml` sets `panic = "abort"` in the release
profile, which terminates the process instead of unwinding — still bad, but not
undefined behavior.

---

## L4: `ensure_validity_writable` is required before NULL output {#l4-ensure_validity_writable-is-required-before-null-output}

**Status**: Made impossible by `VectorWriter::set_null`.

**Symptom**: SEGFAULT when writing NULL values to the output vector.

**Root cause**: `duckdb_vector_get_validity` returns an uninitialized pointer if
`duckdb_vector_ensure_validity_writable` has not been called first. Writing to
an uninitialized address → SEGFAULT.

**Fix**: Always call `duckdb_vector_ensure_validity_writable` before accessing
the validity bitmap on the write path. `VectorWriter::set_null` does this
automatically:

```rust
// Correct — handled by set_null
unsafe { writer.set_null(row) };

// Wrong — validity bitmap may not be allocated yet
// let validity = duckdb_vector_get_validity(output);
// set_bit(validity, row, false);  // SEGFAULT
```

---

## L5: Boolean reading must use `u8 != 0`, not `*const bool`

**Status**: Made impossible by `VectorReader::read_bool`.

**Symptom**: Undefined behavior; Rust requires `bool` to be exactly 0 or 1.

**Root cause**: DuckDB's C API does not guarantee that boolean values in vectors
are exactly 0 or 1. Values of 2, 255, etc. cast to Rust `bool` is undefined
behavior.

**Fix**: Read as `u8` and compare with `!= 0`. `VectorReader::read_bool` always
does this:

```rust
let b: bool = unsafe { reader.read_bool(row) };  // safe: uses u8 != 0 internally
```

---

## L6: Function set name must be set on EACH member

**Status**: Made impossible by `AggregateFunctionSetBuilder`.

**Symptom**: Functions are silently not registered. No error returned.

**Root cause**: When using `duckdb_register_aggregate_function_set`, the function
name must be set on EACH individual `duckdb_aggregate_function` using
`duckdb_aggregate_function_set_name`, not just on the set.

This is completely undocumented. Discovered by reading DuckDB's C++ test code
at `test/api/capi/test_capi_aggregate_functions.cpp`.

In duckdb-behavioral, 6 of 7 functions failed to register silently due to this
bug.

**Fix**: `AggregateFunctionSetBuilder` calls `duckdb_aggregate_function_set_name`
on every individual function before adding it to the set. Use it instead of
managing the set manually.

---

## L7: LogicalType memory leak

**Status**: Made impossible by `LogicalType` RAII wrapper.

**Symptom**: Memory leak proportional to number of registered functions.

**Root cause**: `duckdb_create_logical_type` allocates memory that must be freed
with `duckdb_destroy_logical_type`. Forgetting leaks memory.

**Fix**: `LogicalType` implements `Drop` and calls `duckdb_destroy_logical_type`
automatically when it goes out of scope.

---

## P1: Library name must match extension name

**Status**: Must be configured in `Cargo.toml`. Scaffold handles this.

**Symptom**: Community build fails with `FileNotFoundError`.

**Root cause**: The community build expects `lib{extension_name}.so`. If the
Cargo crate name produces a different `.so` filename, the build fails.

**Fix**: Set `name` explicitly in `[lib]`:

```toml
[lib]
name = "my_extension"   # Must match description.yml `name: my_extension`
crate-type = ["cdylib", "rlib"]
```

---

## P2: Metadata version is C API version, not DuckDB version

**Status**: `DUCKDB_API_VERSION` constant encodes the correct value.

**Symptom**: Metadata script fails or produces incorrect metadata.

**Root cause**: The `-dv` flag to `append_extension_metadata.py` must be the
C API version (`v1.2.0`), not the DuckDB release version (`v1.4.4`). These are
different strings.

**Fix**: Use `quack_rs::DUCKDB_API_VERSION` (`"v1.2.0"`) in `init_extension`,
and use the same version with `append_extension_metadata.py -dv v1.2.0`.

---

## P3: E2E testing is mandatory

**Status**: Documented. See [Testing Guide](../testing.md).

**Symptom**: All unit tests pass but the extension is completely broken.

**Root cause**: Unit tests cannot detect SEGFAULTs on load, silent registration
failures, or wrong results from combine bugs.

**Fix**: Always run E2E tests using an actual DuckDB binary. The scaffold
generates a complete SQLLogicTest skeleton.

---

## P4: `extension-ci-tools` submodule must be initialized

**Status**: Build-time check.

**Symptom**: `make configure` or `make release` fails.

**Fix**:

```bash
git submodule update --init --recursive
```

---

## P5: SQLLogicTest expected values must match exactly

**Status**: Test-authoring care required.

**Symptom**: Tests fail in CI but pass locally (or vice versa).

**Root cause**: SQLLogicTest does exact string matching. Output format (decimal
places, NULL representation, column separators) must match character-for-character.

**Fix**: Generate expected values by running the SQL in DuckDB CLI and copying
the output. NULL is `NULL` (uppercase). Integers have no decimal places.

---

## P6: `duckdb_register_aggregate_function_set` silently fails

**Status**: Builder returns `Err`. Also see L6.

**Symptom**: Function appears registered but is not found in SQL.

**Root cause**: The return value of `duckdb_register_aggregate_function_set` is
often ignored. When it returns `DuckDBError`, the function set is not registered.

**Fix**: The builder checks the return value and propagates it as `Err`.

---

## P7: `duckdb_string_t` format is undocumented

**Status**: Handled by `VectorReader::read_str` and `DuckStringView`.

**Symptom**: VARCHAR reading produces garbage, empty strings, or crashes.

**Root cause**: DuckDB stores strings in a 16-byte struct with two formats
(inline ≤ 12 bytes, pointer > 12 bytes) that are not documented in
`libduckdb-sys`.

**Fix**: Use `VectorReader::read_str(row)`. See
[NULL Handling & Strings](../data/nulls-and-strings.md).

---

## P8: `INTERVAL` struct layout is undocumented

**Status**: Handled by `DuckInterval` and `read_interval_at`.

**Symptom**: Interval calculations produce wrong results or crashes.

**Root cause**: DuckDB's `INTERVAL` is `{ months: i32, days: i32, micros: i64 }`
(16 bytes total). This is not documented in `libduckdb-sys`. Month conversion
uses 1 month = 30 days (DuckDB's approximation).

**Fix**: Use `VectorReader::read_interval(row)` and `DuckInterval`. See
[INTERVAL Type](../data/intervals.md).

---

## Summary

| Pitfall | SDK status | Your action |
|---------|------------|-------------|
| L1: combine config fields | Testable | Test with `AggregateTestHarness::combine` |
| L2: state double-free | Prevented | Use `FfiState::destroy_callback` |
| L3: panic across FFI | Prevented | Use `init_extension`, no `unwrap` in callbacks |
| L4: validity bitmap SEGFAULT | Prevented | Use `VectorWriter::set_null` |
| L5: bool UB | Prevented | Use `VectorReader::read_bool` |
| L6: function set name | Prevented | Use `AggregateFunctionSetBuilder` |
| L7: LogicalType leak | Prevented | Use `LogicalType` (RAII) |
| P1: lib name mismatch | Scaffold | Set `[lib] name` in `Cargo.toml` |
| P2: API version string | Constant | Use `DUCKDB_API_VERSION` |
| P3: unit tests insufficient | Documented | Write SQLLogicTest E2E tests |
| P4: submodule not initialized | Build-time | `git submodule update --init` |
| P5: SQLLogicTest exact match | Documented | Copy output from DuckDB CLI |
| P6: register set silent fail | Prevented | Builder returns `Err` |
| P7: VARCHAR format undocumented | Prevented | Use `VectorReader::read_str` |
| P8: INTERVAL layout undocumented | Prevented | Use `DuckInterval` |
