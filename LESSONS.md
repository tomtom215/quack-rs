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
(e.g., `"v1.2.0"`), NOT the DuckDB release version (e.g., `"v1.4.4"` / `"v1.5.0"` / `"v1.5.1"`).
DuckDB v1.4.x and v1.5.x (including v1.5.1) all use C API version v1.2.0 (confirmed by E2E tests).

**Fix**: Use `quack_rs::DUCKDB_API_VERSION` constant for the init call, and use the same
version string with `append_extension_metadata.py -dv v1.2.0`.

**Caveat — this only holds for the `C_STRUCT` ABI type.** `-dv` means the *C API* version for
`C_STRUCT`, but the *exact DuckDB release* for `C_STRUCT_UNSTABLE` and `CPP`. If you set
`USE_UNSTABLE_C_API=1` (which you must when using quack-rs's `duckdb-1-5` features — see P10),
`TARGET_DUCKDB_VERSION` must be a real release like `v1.5.5`, and `v1.2.0` would pin the binary
to DuckDB v1.2.0. `ScaffoldConfig` validates this pairing.

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

**Status**: Handled by `VectorReader::read_str`, `VectorReader::read_blob`,
`read_duck_string`, and `read_duck_blob`.

**Symptom**: VARCHAR reading produces garbage, empty strings, or crashes; BLOB
reading silently drops bytes that are not valid UTF-8.

**Root cause**: DuckDB stores strings in a 16-byte struct with two formats:
- **Inline** (≤ 12 bytes): `[ len: u32 | data: [u8; 12] ]`
- **Pointer** (> 12 bytes): `[ len: u32 | prefix: [u8; 4] | ptr: *const u8 | unused: u32 ]`

This is not documented in `libduckdb-sys`.

**Fix**: Use `VectorReader::read_str` or `read_duck_string` for UTF-8 text. Use
`VectorReader::read_blob` or `read_duck_blob` for arbitrary binary data. All four
handle both storage formats.

---

## P8: `INTERVAL` struct layout is undocumented

**Status**: Handled by `DuckInterval` and `read_interval_at`.

**Symptom**: Interval calculations produce wrong results or crashes.

**Root cause**: DuckDB's `INTERVAL` is a 16-byte struct: `{ months: i32, days: i32, micros: i64 }`.
This layout is not documented in the Rust bindings.
Month conversion uses the approximation: **1 month = 30 days** (matching DuckDB's behavior).

**Fix**: Use `DuckInterval`, `read_interval_at`, and `interval_to_micros` from the `interval` module.

---

## P9: `bundled-test` + `loadable-extension` dispatch-table conflict

**Status**: Fixed in quack-rs v0.6.0. `InMemoryDb::open()` now initialises the dispatch
table automatically. No user action required.

**Symptom**: Any test that calls `InMemoryDb::open()` (or otherwise exercises the DuckDB C
API through `libduckdb-sys`) panics with:

```
DuckDB API not initialized or DuckDB feature omitted
```

The panic occurs in the generated `bindgen.rs` file at a function-pointer assertion, not in
user code. All other tests (pure-Rust logic, `AggregateTestHarness`, mock utilities) pass.

**Affected configuration**: `cargo test --features bundled-test` (or `--all-features`) on
any platform. The panic was first observed on macOS only because the regular CI job used
`cargo test --all-targets` without `--features bundled-test`, so the `in_memory_db` module
was never compiled or exercised in CI runs.

**Root cause** (deep):

quack-rs requires `libduckdb-sys` with `features = ["loadable-extension"]` so that, at
runtime, DuckDB can load the compiled `.so`/`.dylib` and populate a lazy function-pointer
dispatch table.  In `loadable-extension` mode, *every* DuckDB C API call—including
`duckdb_open`, `duckdb_query`, etc.—is routed through a global `AtomicPtr` per function.
The atomics start as null (panic stubs) and are initialised when DuckDB calls the
extension's `init_c_api` entry-point.

The `bundled-test` feature adds `duckdb = { features = ["bundled"] }` as a dependency.
`duckdb` in turn depends on `libduckdb-sys`.  Cargo's **feature-unification** rule merges
features across all uses of the same crate, so the single `libduckdb-sys` that ends up in
the test binary carries *both* `loadable-extension` AND `bundled-full`.

In this combined configuration the bundled DuckDB static library is linked into the binary,
but `loadable-extension` still intercepts every call through the uninitialized dispatch
table.  Since `cargo test` never goes through DuckDB's extension-loading path, the atomics
are never populated, and any DuckDB API call panics.

**Why macOS looked different**: The regular CI job ran `cargo test --all-targets` (no
`--all-features`), so `bundled-test` was off and the `in_memory_db` module was never
compiled.  The release workflow ran `cargo test --all-targets --all-features`, which did
enable `bundled-test`.  With `fail-fast: true`, macOS happened to fail first; the Linux and
Windows jobs were also broken but were cancelled before reporting.

**The fix**: DuckDB's own C++ codebase contains an internal inline function `CreateAPIv1()`
(in `duckdb/main/capi/extension_api.hpp`) that constructs the complete `duckdb_ext_api_v1`
struct, setting every one of the ~573 function-pointer fields to the matching bundled DuckDB
C symbol.  This is exactly the same struct that DuckDB would send to an extension's
`init_c_api` callback.

quack-rs now compiles a tiny C++ shim (`src/testing/bundled_api_init.cpp`) that wraps
`CreateAPIv1()` as a C-linkage symbol `quack_rs_create_api_v1()`.  In `InMemoryDb::open()`,
a `std::sync::Once` guard calls this shim and feeds the result to
`libduckdb_sys::duckdb_rs_extension_api_init`, which populates all the atomics in one pass.
After that, the `duckdb` crate can open connections and run queries normally.

The header path is resolved at build time by `build.rs`, which searches for
`libduckdb-sys-*/out/duckdb/src/include` in Cargo's shared `build/` directory — the same
location where `libduckdb-sys` extracts the bundled DuckDB tarball.

**Risk and mitigation**: `extension_api.hpp` and `CreateAPIv1()` are **internal DuckDB
implementation details**, not part of the stable public C API.  They live in the DuckDB
source tree and can change between releases.  Known mitigations:

| Risk | Mitigation |
|------|-----------|
| `extension_api.hpp` is renamed or moved | `build.rs` will fail to compile with a clear error pointing at the missing header |
| `CreateAPIv1()` function is renamed | Same — C++ compile error |
| `duckdb_ext_api_v1` gains new fields in a future DuckDB version | `CreateAPIv1()` will fill all new fields too; the Rust struct (generated by `libduckdb-sys` bindgen from the same header set) will also have them; ABI stays consistent |
| `duckdb_ext_api_v1` field order changes | Within one DuckDB release both the C++ struct (`extension_api.hpp`) and the Rust struct (`libduckdb-sys` bindgen from `duckdb_extension.h`) are generated from the same source, so they stay in sync. **Across releases they do not** — see P10, which is about the loaded-extension case rather than this bundled-test one |
| libduckdb-sys switches away from `loadable-extension` dispatch | The problem disappears entirely; the `Once` guard becomes a cheap no-op |

The two header files that define `duckdb_ext_api_v1` are `duckdb_extension.h` (public,
used by extension authors and by `libduckdb-sys` bindgen) and `extension_api.hpp` (internal,
used by DuckDB's C++ extension-loader).  Both are maintained in the same DuckDB repository
release and always have identical field counts and field *order* — verified by
`scripts/check-abi-table.py`, which re-derives both from every release tag (459 entries for
DuckDB 1.4.x, 545 for 1.5.0–1.5.1, 546 for 1.5.2–1.5.5).

**This is a genuine DuckDB ecosystem discovery**: the combination of `loadable-extension`
dispatch and bundled DuckDB is not documented anywhere in the `duckdb-rs` or `libduckdb-sys`
repositories.  The fix relies on `CreateAPIv1()` — an implementation detail that the DuckDB
team could consider promoting to a stable C API (`duckdb_get_api_v1()` or similar) to make
this class of workaround unnecessary.

---

## P10: The C API struct has a stable prefix and an unstable tail

**Status**: Detected at load time by `quack_rs::abi` (default `AbiPolicy::Strict`).

**Symptom**: An extension loads without complaint and then corrupts memory —
`double free or corruption`, a segfault, or silently wrong results — on a DuckDB
release other than the one it was built against. Nothing in the build or the load
warns you.

**Root cause**: DuckDB hands a loadable extension a pointer to a
`duckdb_ext_api_v1` struct of function pointers, and the extension calls through
it at compiled-in offsets. The struct has two regions:

| Region | Slots | Guarantee |
|--------|-------|-----------|
| Stable | 0–356 | Frozen since v1.2.0 — identical names, identical order, in every release through v1.5.5 |
| Unstable | 357+ | DuckDB **inserts** entries in the middle, shifting every later slot |

| DuckDB | Total slots | What moved |
|--------|-------------|------------|
| v1.2.0 – v1.2.2 | 408 | baseline |
| v1.3.0 – v1.3.2 | 428 | appended |
| v1.4.0 – v1.4.4 | 459 | `duckdb_create_varint` renamed to `duckdb_create_bignum`; appended |
| v1.5.0 – v1.5.1 | 545 | `duckdb_appender_clear` **inserted** at slot 410 |
| v1.5.2 – v1.5.5 | 546 | `duckdb_geometry_type_get_crs` **inserted** at slot 493 |

Everything behind quack-rs's `duckdb-1-5` / `duckdb-1-5-3` features — 105 C API
functions covering scalar bind/init, copy functions, catalog access, `ErrorData`,
`FileSystem`, `Expression`, `SelectionVector`, config options, table descriptions
and the client context — sits in the unstable region.

DuckDB will not catch this for you. Its loader validates the ABI metadata in the
extension footer:

- `C_STRUCT` + a C API version (`v1.2.0`) — accepted by **any** DuckDB whose C API
  version is at least that, then handed the *whole* struct, unstable region
  included. There is no engine-version check.
- `C_STRUCT_UNSTABLE` + a DuckDB release version — accepted only by that exact
  release.

So a `C_STRUCT` binary that touches the unstable region loads happily into a
DuckDB with a different layout and calls the wrong function pointers. Built
against v1.5.0 and loaded into v1.5.2+, for example,
`duckdb_scalar_function_set_bind_data(info, data, destroy)` actually invokes
`duckdb_scalar_function_get_client_context(info)`, and
`duckdb_connection_get_client_context` invokes `duckdb_destroy_client_context`.

DuckDB's own `extension-template-c` says as much:

> WARNING: When set to 1, the `duckdb_extension.h` from the `TARGET_DUCKDB_VERSION`
> must be used, using any other version of the header is unsafe.

**Reproduction** (DuckDB 1.5.5, extension built against libduckdb-sys `=1.10500.0`
and stamped `C_STRUCT`/`v1.2.0`):

```text
double free or corruption (out)
Aborted (core dumped)
```

**Fix**:

1. If you use `duckdb-1-5` / `duckdb-1-5-3`, build with `USE_UNSTABLE_C_API=1` and
   a real `TARGET_DUCKDB_VERSION` (or `append_metadata --abi-type C_STRUCT_UNSTABLE
   --duckdb-version vX.Y.Z`). DuckDB then refuses to load the binary into any other
   release, at install time, with a clear message.
2. Leave `AbiPolicy::Strict` on. `quack_rs::abi::check()` compares the compiled-in
   slot count against the layout the running engine uses — resolved from
   `duckdb_library_version()`, which lives at stable slot 7 and is therefore always
   dispatched correctly — and the entry point turns a mismatch into a `LOAD` error
   naming both layouts and the remedy. Use `AbiPolicy::Trust` only when the binary
   is stamped `C_STRUCT_UNSTABLE`, where DuckDB already enforces the pinning.
3. If you stay off those features, nothing changes: the stable prefix is frozen, so
   `C_STRUCT` + `v1.2.0` remains portable across every DuckDB from v1.2.0 onwards.

`scripts/check-abi-table.py` re-derives the layout table from every upstream
release header and runs in CI, so the table cannot silently go stale.

---

## P11: `const char *` returns are borrowed — freeing one corrupts the heap

**Status**: Fixed in `CopyGlobalInitInfo::get_file_path`; every other
`duckdb_free` call site in the crate audited against DuckDB's implementation.

**Symptom**: `corrupted size vs. prev_size in fastbins`, `free(): invalid
pointer`, or a `SIGABRT` at an unrelated later allocation. Nothing points at the
call that caused it.

**Root cause**: The C API returns strings two ways, and only one of them
transfers ownership.

| Return type | Typical implementation | Caller must |
|-------------|------------------------|-------------|
| `char *` | `strdup(...)` or `duckdb_malloc` + `memcpy` | `duckdb_free` it |
| `const char *` | `some_std_string.c_str()` | **not** free it |

`duckdb_copy_function_global_init_get_file_path` is the second kind — it returns
`info_ref.file_path.c_str()`, the interior pointer of a C++ `std::string` that
DuckDB still owns and will destroy itself. quack-rs called `duckdb_free` on it,
handing the allocator a pointer it never issued.

**The trap in the rule**: the signature alone is not enough.
`duckdb_parameter_name` is declared `const char *` and yet returns
`strdup(identifier.c_str())` — it *is* owned, and *not* freeing it leaks. The
only reliable check is reading the implementation in DuckDB's `src/main/capi/`.

**Audit performed** (DuckDB 1.5.5, every `duckdb_free` site in quack-rs):

| Function | Implementation | quack-rs |
|----------|----------------|----------|
| `duckdb_logical_type_get_alias` | `strdup` | frees ✓ |
| `duckdb_enum_dictionary_value` | `strdup` | frees ✓ |
| `duckdb_struct_type_child_name` | `strdup` | frees ✓ |
| `duckdb_union_type_member_name` | `strdup` | frees ✓ |
| `duckdb_get_varchar` | `duckdb_malloc` + `memcpy` | frees ✓ |
| `duckdb_value_to_string` | `duckdb_malloc` + `memcpy` | frees ✓ |
| `duckdb_get_blob` | `malloc` + `memcpy` | frees ✓ |
| `duckdb_table_description_get_column_name` | `malloc` + `memcpy` | frees ✓ |
| `duckdb_parameter_name` | `strdup` (despite `const char *`) | frees ✓ |
| `duckdb_get_or_create_from_cache` (`out_error`) | `strdup` | frees ✓ |
| `duckdb_table_description_error` | `wrapper->error.c_str()` | does not free ✓ |
| `duckdb_appender_error` | `error_data.RawMessage().c_str()` | does not free ✓ |
| `duckdb_copy_function_global_init_get_file_path` | `file_path.c_str()` | **was freeing ✗** |

**How it was found**: by writing the first live test for copy functions. The
module had 16 unit tests and not one of them registered a copy function against
a real DuckDB, so the corruption had never had a chance to happen. Unit tests
over an FFI wrapper test the wrapper's arithmetic, not its contract with the
library.

**Rule**: before calling `duckdb_free` on anything the C API returned, read the
implementation. `const char *` is a strong hint that it is borrowed, but
`duckdb_parameter_name` proves it is only a hint.


## Community Extension Submission

### Build System Requirements

DuckDB's **C Extension API** now allows pure-Rust extensions **without any C++ glue**.
The official [extension-template-rs](https://github.com/duckdb/extension-template-rs) demonstrates
this approach. A pure-Rust extension needs:

1. **Cargo.toml**: `cdylib` crate type, pinned `duckdb` + `libduckdb-sys` deps, release profile
2. **Makefile**: Delegates to `cargo build` + metadata scripts from `extension-ci-tools`
3. **extension-ci-tools**: Git submodule for the DuckDB extension CI/CD pipeline
4. **src/lib.rs**: Entry point using `duckdb_entrypoint_c_api` macro + function registration
5. **description.yml**: Extension metadata (`language: Rust` and `build: cargo` for Rust extensions)
6. **test/sql/*.test**: SQLLogicTest format integration tests

Use `quack_rs::scaffold::generate_scaffold` to auto-generate all of these files from a
[`ScaffoldConfig`](https://docs.rs/quack-rs/latest/quack_rs/scaffold/struct.ScaffoldConfig.html).

> **Note**: The C Extension API has a stable and unstable part. The official template enables
> the unstable API via `USE_UNSTABLE_C_API=1` in the Makefile. See
> [extension-template-rs](https://github.com/duckdb/extension-template-rs) for details.

### description.yml

Required fields:
```yaml
extension:
  name: your_extension
  description: One-line description
  version: 0.1.0
  language: Rust
  build: cargo
  license: MIT
  requires_toolchains: rust;python3
  excluded_platforms: "wasm_mvp;wasm_eh;wasm_threads"  # optional
  maintainers:
    - Your Name

repo:
  github: yourorg/your_extension
  ref: main
```

Use `quack_rs::validate` to check name, version, and license before submission,
or use `quack_rs::scaffold::generate_scaffold` to auto-generate all project files.

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
| `windows_amd64_mingw` | Windows x86_64 (MinGW) |
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

### Extension Versioning

DuckDB core extensions use a three-tier versioning scheme. Community extensions should follow
the same convention:

| Level | Format | Example | Meaning |
|-------|--------|---------|---------|
| **Unstable** | Short git hash (7+ hex chars) | `690bfc5` | No stability guarantees |
| **Pre-release** | Semver `0.y.z` | `0.1.0` | Working toward stability |
| **Stable** | Semver `x.y.z` (x>0) | `1.0.0` | Full semver, stable API |

Key points:

- **Unstable** extensions may change or remove functionality at any time
- **Pre-release** extensions follow semver but the API may still have breaking changes in minor versions
- **Stable** extensions guarantee backwards-compatible APIs; breaking changes require a major version bump
- Use `quack_rs::validate::validate_extension_version` to accept all three formats
- Use `quack_rs::validate::semver::classify_extension_version` to determine the stability tier

### Extension Binary Compatibility

Extension binaries are tied to a specific DuckDB version and platform. Key implications:

- New binaries must be built for each DuckDB release
- Extensions compiled for one DuckDB version will not load in another
- DuckDB verifies binary compatibility before loading and will refuse mismatched binaries
- All official extensions are cryptographically signed by the DuckDB team
- Unsigned extensions require `allow_unsigned_extensions` to load (development only)
- The DuckDB extension template provides CI workflows for automated cross-platform builds

### CI Toolchain Notes

The community extension CI uses specific compiler versions and system libraries. Common issues:

- Rust toolchain must be available in CI (add `rustup` setup to your CI workflow)
- Cross-compilation for `linux_arm64` from `linux_amd64` requires the appropriate target
- WASM targets (`wasm_mvp`, `wasm_eh`, `wasm_threads`) may not work with all Rust crates
- Use `excluded_platforms` to skip targets that cannot be built

---

## Architecture Decision Records

> **Note**: ADR-1 through ADR-3 (design principles) are in
> [README.md](./README.md#architecture-decision-records). The ADRs below cover
> implementation-level decisions specific to FFI integration.

### ADR-4: `libduckdb-sys` only at runtime (no `duckdb` crate)

The `duckdb` crate provides a high-level Rust API but also includes a bundled DuckDB (via
the `bundled` feature). For loadable extensions, we must NOT bundle DuckDB — we link against
the DuckDB that loads us. The `libduckdb-sys` with `loadable-extension` feature provides
exactly this: lazy-initialized function pointers populated by DuckDB at load time.

### ADR-5: Function sets instead of varargs

`duckdb_aggregate_function_set_varargs` does not exist for aggregate functions. For variadic
signatures (e.g., `retention(c1, c2, ..., c32)`), you must register N overloads using a
`duckdb_aggregate_function_set`. `AggregateFunctionSetBuilder` handles this.

> **Note (DuckDB 1.5.0)**: Scalar functions now support varargs directly via
> `ScalarFunctionBuilder::varargs()` (requires the `duckdb-1-5` feature). This limitation
> still applies to aggregate functions, which must use function sets for variadic signatures.

### ADR-6: Custom C entry point instead of `duckdb-loadable-macros`

`duckdb-loadable-macros` relies on `extract_raw_connection` which uses the internal
`Rc<RefCell<InnerConnection>>` layout. This is fragile and causes SEGFAULTs when the layout
changes. The correct approach is a hand-written C entry point that calls
`duckdb_rs_extension_api_init`, `get_database`, and `duckdb_connect` directly.
`quack_rs::entry_point::init_extension` encapsulates this correctly.
