# quack-rs — Pitfall Reference

All known DuckDB Rust FFI pitfalls, with symptoms, root causes, and fixes.

These were discovered building [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral),
a production DuckDB community extension. Every future developer who builds a Rust DuckDB extension
will hit every one of these problems. This SDK makes most of them impossible.

---

## L1: COMBINE must propagate ALL config fields

**Status**: Can be tested with `AggregateTestHarness`.

**Symptom**: Aggregate function returns wrong results. No error, no crash.

**Root cause**: DuckDB's segment tree creates fresh zero-initialized target states via `state_init`,
then calls `combine` to merge source states into them. If your `combine` only propagates data
fields (e.g., `count`, `sum`) but forgets configuration fields (e.g., `window_size`, `mode`),
the configuration will be zero at finalize time, silently corrupting results.

**Fix**:
```rust
unsafe extern "C" fn combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        if let (Some(src), Some(tgt)) = (
            FfiState::<MyState>::with_state(*source.add(i)),
            FfiState::<MyState>::with_state_mut(*target.add(i)),
        ) {
            // MUST copy ALL fields, including configuration fields
            tgt.window_size = src.window_size;  // config field
            tgt.mode = src.mode;                // config field
            tgt.count += src.count;             // data field
        }
    }
}
```

**SDK status**: `AggregateTestHarness::combine` lets you test this without DuckDB.
The combine-propagates-config bug passed 435 unit tests before being caught by E2E tests.

---

## L2: State destroy double-free

**Status**: Made impossible by `FfiState<T>`.

**Symptom**: Crash or memory corruption on extension unload.

**Root cause**: If `state_destroy` frees the inner Box but doesn't null out the pointer,
a second call to `state_destroy` (e.g., in error paths) will free already-freed memory.

**Fix**: `FfiState<T>::destroy_callback` nulls `inner` after freeing. Use it instead of
writing your own destructor.

---

## L3: No panic across FFI boundaries

**Status**: Made impossible by `init_extension` helper.

**Symptom**: Extension causes DuckDB to crash or behave unpredictably.

**Root cause**: `panic!()` and `.unwrap()` in `unsafe extern "C"` functions is undefined
behavior. Panics cannot unwind across FFI boundaries in Rust.

**Fix**: Use `Result` and `?` inside `init_extension`. Never use `unwrap()` in FFI callbacks.
`FfiState::with_state_mut` returns `Option`, not `Result`, so callers use `if let`.

---

## L4: `ensure_validity_writable` is required before NULL output

**Status**: Made impossible by `VectorWriter::set_null`.

**Symptom**: SEGFAULT when writing NULL values to the output vector.

**Root cause**: `duckdb_vector_get_validity` returns an uninitialized pointer if
`duckdb_vector_ensure_validity_writable` has not been called first. If you skip the first
call and then try to set a row invalid, you write to an uninitialized address.

**Fix**: Always call `duckdb_vector_ensure_validity_writable` before `duckdb_vector_get_validity`
when writing NULLs. `VectorWriter::set_null` does this automatically.

---

## L5: Boolean reading must use `u8 != 0`, not `*const bool`

**Status**: Made impossible by `VectorReader::read_bool`.

**Symptom**: Undefined behavior; Rust requires `bool` to be exactly 0 or 1.

**Root cause**: DuckDB's C API does not guarantee that boolean values in vectors are exactly
0 or 1. Casting a byte with value 2, 255, etc. to Rust `bool` is undefined behavior.

**Fix**: Read boolean data as `*const u8` and compare with `!= 0`.
`VectorReader::read_bool` always does this.

---

## L6: Function set name must be set on EACH member

**Status**: Made impossible by `AggregateFunctionSetBuilder`.

**Symptom**: Function is silently not registered. No error returned.

**Root cause**: When using `duckdb_register_aggregate_function_set`, the function name must
be set on EACH individual `duckdb_aggregate_function` added to the set using
`duckdb_aggregate_function_set_name`, not just on the set itself.

This is completely undocumented. Discovered by reading DuckDB's C++ test code at
`test/api/capi/test_capi_aggregate_functions.cpp`.

In duckdb-behavioral, 6 of 7 functions failed to register silently due to this bug.

**Fix**: `AggregateFunctionSetBuilder` calls `duckdb_aggregate_function_set_name` on every
individual function before adding it to the set.

---

## L7: LogicalType memory leak

**Status**: Made impossible by `LogicalType` RAII wrapper.

**Symptom**: Memory leak proportional to number of registered functions.

**Root cause**: `duckdb_create_logical_type` allocates memory that must be freed with
`duckdb_destroy_logical_type`. Forgetting to call the destructor leaks memory.

**Fix**: `LogicalType` implements `Drop` and calls `duckdb_destroy_logical_type` automatically.

---

## P1: Library name must match extension name

**Status**: Must be configured manually in `Cargo.toml`.

**Symptom**: Community build fails with `FileNotFoundError` when building the extension.

**Root cause**: The community extension Makefile expects `lib{extension_name}.so`. If your
Cargo crate is named `duckdb-my-ext` (producing `libduckdb_my_ext.so`) but `description.yml`
says `name: my-ext`, the build fails.

**Fix**: Add `name = "extension_name"` to `[lib]` in `Cargo.toml`:
```toml
[lib]
name = "my_extension"   # Must match description.yml's `name: my_extension`
crate-type = ["cdylib", "rlib"]
```

---

## P2: Extension metadata version is C API version, not DuckDB version

**Status**: Must be handled manually when using `append_extension_metadata.py`.

**Symptom**: Metadata script fails or produces incorrect metadata.

**Root cause**: The `-dv` flag to `append_extension_metadata.py` must be the C API version
(e.g., `"v1.2.0"`), NOT the DuckDB release version (e.g., `"v1.4.4"`).
DuckDB v1.4.4 uses C API version v1.2.0.

**Fix**: Use `quack_rs::DUCKDB_API_VERSION` constant for the init call, and use the same
version string with `append_extension_metadata.py -dv v1.2.0`.

---

## P3: E2E testing is mandatory — unit tests alone are insufficient

**Status**: Documented. See testing guide.

**Symptom**: All unit tests pass but the extension is completely broken when loaded.

**Root cause**: Unit tests test Rust logic in isolation. They cannot detect:
- SEGFAULTs on extension load
- Functions failing to register silently
- Wrong results due to combine not propagating config

In duckdb-behavioral, 435 unit tests passed while the extension had three critical bugs:
1. SEGFAULT on load (wrong entry point)
2. 6 of 7 functions not registered (function set name bug)
3. Wrong results from window_funnel (combine not propagating config)

**Fix**: Always run E2E tests using the actual DuckDB CLI:
```sql
LOAD './libmy_extension.so';
SELECT my_function(col) FROM ...;
```

---

## P4: extension-ci-tools submodule must be initialized

**Status**: Build-time check, no SDK fix needed.

**Symptom**: `make configure` or `make release` fails.

**Root cause**: The community extension CI uses `extension-ci-tools` as a git submodule.
If not initialized, the Makefile cannot find the build scripts.

**Fix**:
```bash
git submodule update --init --recursive
```

---

## P5: SQLLogicTest expected values must match actual DuckDB output

**Status**: Test-authoring care required.

**Symptom**: Tests fail in CI but pass locally (or vice versa) due to output format differences.

**Root cause**: SQLLogicTest format is exact-match. Output formatting (decimal places, NULL
representation, etc.) must match exactly.

**Fix**: Generate expected values by running the actual SQL in DuckDB CLI and copying the output.

---

## P6: `duckdb_register_aggregate_function_set` silently fails

**Status**: Made impossible by builder (returns `Err`). Also see L6.

**Symptom**: Function appears to be registered but is not found when called in SQL.

**Root cause**: `duckdb_register_aggregate_function_set` returns `DuckDBError` silently when
the function set name is not set on individual members (see L6). The return value is often
ignored by extension authors.

**Fix**: The builder checks the return value and returns `Err` on failure.
Additionally, use `duckdb_get_function` to verify registration in development.

---

## P7: `duckdb_string_t` format is undocumented in Rust bindings

**Status**: Handled by `VectorReader::read_str` and `read_duck_string`.

**Symptom**: VARCHAR reading produces garbage, empty strings, or crashes.

**Root cause**: DuckDB stores strings in a 16-byte struct with two formats:
- **Inline** (≤ 12 bytes): `[ len: u32 | data: [u8; 12] ]`
- **Pointer** (> 12 bytes): `[ len: u32 | prefix: [u8; 4] | ptr: *const u8 | unused: u32 ]`

This is not documented in `libduckdb-sys`.

**Fix**: Use `VectorReader::read_str` or `read_duck_string` which handle both formats.

---

## P8: `INTERVAL` struct layout is undocumented

**Status**: Handled by `DuckInterval` and `read_interval_at`.

**Symptom**: Interval calculations produce wrong results or crashes.

**Root cause**: DuckDB's `INTERVAL` is a 16-byte struct: `{ months: i32, days: i32, micros: i64 }`.
This layout is not documented in the Rust bindings.
Month conversion uses the approximation: **1 month = 30 days** (matching DuckDB's behavior).

**Fix**: Use `DuckInterval`, `read_interval_at`, and `interval_to_micros` from the `interval` module.

---

## Community Extension Submission

### Build System Requirements

DuckDB community extensions use a CMake-based build system. As of DuckDB v1.4.x, Rust extensions
cannot be submitted as pure Rust — they require:

1. **C++ glue layer**: A thin `.cpp` file that calls into your Rust `cdylib` shared library
2. **CMakeLists.txt**: Builds the C++ glue and links the Rust library
3. **extension-ci-tools**: Git submodule for the DuckDB extension CI/CD pipeline
4. **Makefile**: Standard targets: `configure`, `debug`, `release`, `test`
5. **description.yml**: Extension metadata (name, version, language, build, licence, maintainers, repo)
6. **test/sql/*.test**: SQLLogicTest format integration tests

The DuckDB team is developing a C Extension API that will eventually allow pure Rust extensions
without C++ glue. Track progress at: https://github.com/duckdb/duckdb/discussions/14286

### description.yml

Required fields:
```yaml
extension:
  name: your_extension
  description: One-line description
  version: 0.1.0
  language: C++        # Currently required even for Rust extensions (C++ glue)
  build: cmake
  license: MIT
  maintainers:
    - Your Name

repo:
  github: yourorg/your_extension
  ref: main
```

Use `quack_rs::validate` to check name, version, and license before submission.

### Naming Rules

- Extension names must be globally unique across the entire DuckDB community extensions ecosystem
- Check existing names at https://community-extensions.duckdb.org/ before choosing
- Use vendor prefixing to avoid collisions (e.g., `myorg_analytics` instead of `analytics`)
- Names must match `^[a-z][a-z0-9_-]*$` and not exceed 64 characters
- The `[lib] name` in `Cargo.toml` MUST match the extension name (Pitfall P1)

### Platform Targets

Community extensions are built for these platform targets:

| Platform | Description |
|----------|-------------|
| `linux_amd64` | Linux x86_64 |
| `linux_amd64_gcc4` | Linux x86_64 (GCC 4 compatible) |
| `linux_arm64` | Linux AArch64 |
| `osx_amd64` | macOS x86_64 |
| `osx_arm64` | macOS Apple Silicon |
| `windows_amd64` | Windows x86_64 |
| `windows_arm64` | Windows AArch64 |
| `wasm_mvp` | WebAssembly (MVP) |
| `wasm_eh` | WebAssembly (exception handling) |
| `wasm_threads` | WebAssembly (threads) |

Use `excluded_platforms` in `description.yml` to skip platforms your extension cannot support.
Validate with `quack_rs::validate::validate_platform` and `validate_excluded_platforms`.

### Security Disclaimer

Community extensions are NOT vetted for security by the DuckDB team. The community extensions
repository is a distribution mechanism, not a security guarantee. As an extension author:

- Never panic across FFI boundaries (`quack-rs` enforces `panic = "abort"`)
- Validate all user inputs at system boundaries
- Do not include secrets, credentials, or API keys in your extension binary
- Follow the OWASP top 10 where applicable (SQL injection via dynamic SQL, etc.)

### CI Toolchain Notes

The community extension CI uses specific compiler versions and system libraries. Common issues:

- Rust toolchain must be available in CI (add `rustup` setup to your CI workflow)
- Cross-compilation for `linux_arm64` from `linux_amd64` requires the appropriate target
- WASM targets (`wasm_mvp`, `wasm_eh`, `wasm_threads`) may not work with all Rust crates
- Use `excluded_platforms` to skip targets that cannot be built

---

## Architecture Decision Records

### ADR-1: `libduckdb-sys` only at runtime (no `duckdb` crate)

The `duckdb` crate provides a high-level Rust API but also includes a bundled DuckDB (via
the `bundled` feature). For loadable extensions, we must NOT bundle DuckDB — we link against
the DuckDB that loads us. The `libduckdb-sys` with `loadable-extension` feature provides
exactly this: lazy-initialized function pointers populated by DuckDB at load time.

### ADR-2: Function sets instead of varargs

`duckdb_aggregate_function_set_varargs` does not exist for aggregate functions. For variadic
signatures (e.g., `retention(c1, c2, ..., c32)`), you must register N overloads using a
`duckdb_aggregate_function_set`. `AggregateFunctionSetBuilder` handles this.

### ADR-3: Custom C entry point instead of `duckdb-loadable-macros`

`duckdb-loadable-macros` relies on `extract_raw_connection` which uses the internal
`Rc<RefCell<InnerConnection>>` layout. This is fragile and causes SEGFAULTs when the layout
changes. The correct approach is a hand-written C entry point that calls
`duckdb_rs_extension_api_init`, `get_database`, and `duckdb_connect` directly.
`quack_rs::entry_point::init_extension` encapsulates this correctly.
