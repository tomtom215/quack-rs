# quack-rs

[![CI](https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/quack-rs.svg)](https://crates.io/crates/quack-rs)
[![docs.rs](https://img.shields.io/docsrs/quack-rs)](https://docs.rs/quack-rs)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![MSRV: 1.84.1](https://img.shields.io/badge/MSRV-1.84.1-blue.svg)](https://blog.rust-lang.org/2025/01/30/Rust-1.84.1.html)

**A production-grade Rust SDK for building DuckDB loadable extensions.**

`quack-rs` solves the hard FFI problems that every DuckDB Rust extension author hits — so you can focus on your business logic, not on debugging silent segfaults.

## Why quack-rs?

Building a DuckDB extension in Rust requires solving 15+ undocumented FFI problems that are not covered anywhere in the DuckDB documentation or Rust ecosystem. Every developer who tries to build a community extension hits these problems from scratch. (Note: community extensions currently require a thin C++ glue layer for the build system — see [Important Caveats](#important-caveats) below.)

`quack-rs` was extracted from [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral), a production DuckDB community extension that implements 7 behavioral analytics aggregate functions. Building it required discovering these problems the hard way.

## Quick Start

Add to your extension's `Cargo.toml`:

```toml
[dependencies]
quack-rs = "0.1"
libduckdb-sys = { version = "=1.4.4", features = ["loadable-extension"] }

[lib]
name = "my_extension"  # MUST match extension name — see Pitfall P7
crate-type = ["cdylib", "rlib"]

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Write your entry point:

```rust
use quack_rs::entry_point::init_extension;
use quack_rs::DUCKDB_API_VERSION;

#[no_mangle]
pub unsafe extern "C" fn my_extension_init_c_api(
    info: libduckdb_sys::duckdb_extension_info,
    access: *const libduckdb_sys::duckdb_extension_access,
) -> bool {
    unsafe {
        init_extension(info, access, DUCKDB_API_VERSION, |con| {
            register_word_count(con)?;
            Ok(())
        })
    }
}
```

Register an aggregate function:

```rust
use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};
use quack_rs::types::TypeId;
use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_aggregate_state,
                    duckdb_data_chunk, duckdb_vector, idx_t};

#[derive(Default)]
struct WordCountState { count: u64 }
impl AggregateState for WordCountState {}

// ... implement your callbacks using FfiState<WordCountState> ...

fn register_word_count(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
    unsafe {
        AggregateFunctionBuilder::new("word_count")
            .param(TypeId::Varchar)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<WordCountState>::size_callback)
            .init(FfiState::<WordCountState>::init_callback)
            .update(my_update)
            .combine(my_combine)
            .finalize(my_finalize)
            .destructor(FfiState::<WordCountState>::destroy_callback)
            .register(con)
    }
}
```

Test your aggregate logic without DuckDB:

```rust
use quack_rs::testing::AggregateTestHarness;

let result = AggregateTestHarness::<WordCountState>::aggregate(
    ["hello world", "foo bar baz"],
    |state, s| state.count += s.split_whitespace().count() as u64,
);
assert_eq!(result.count, 5);
```

## What quack-rs Solves

| Problem | Without quack-rs | With quack-rs |
|---------|-----------------|---------------|
| Entry point SEGFAULT | `extract_raw_connection` crashes | `init_extension` helper, panic-free |
| Silent registration failure | Function set silently not registered | Builder enforces name on each member |
| combine config loss | Wrong results, silent | Harness test catches this |
| NULL output crash | Need to remember `ensure_validity_writable` | `VectorWriter::set_null` does it automatically |
| Boolean UB | `as bool` cast, undefined behavior | `read_bool` uses `u8 != 0` |
| Memory leak | `duckdb_logical_type` never freed | `LogicalType` RAII wrapper |
| `duckdb_string_t` format | Undocumented, confusing | `read_duck_string` handles both formats |
| Interval conversion | Undocumented struct layout | `DuckInterval` with overflow checking |

See [LESSONS.md](LESSONS.md) for all 15 pitfalls with symptoms, root causes, and fixes.

## SDK Modules

| Module | Purpose |
|--------|---------|
| `entry_point` | Panic-free `{name}_init_c_api` entry point helper + `entry_point!` macro |
| `aggregate` | Builders for registering aggregate functions |
| `aggregate::state` | Generic `FfiState<T>` — no raw pointer lifecycle code |
| `scalar` | Builder for registering scalar functions |
| `vector` | Safe readers/writers for DuckDB data vectors (including VARCHAR and INTERVAL) |
| `types` | `TypeId` enum, `LogicalType` RAII wrapper |
| `interval` | `INTERVAL` → microseconds with overflow checking |
| `error` | `ExtensionError` for `?`-based error propagation |
| `validate` | Community extension compliance validators (name, semver, SPDX, platform) |
| `testing` | `AggregateTestHarness` — test logic without DuckDB |

## Examples

See [examples/hello-ext](examples/hello-ext/) for a complete, community-extension-ready DuckDB Rust extension using this SDK.

## Community Extension Submission

Your extension will need:
- `extension-ci-tools` git submodule
- `Makefile` with `configure`/`debug`/`release`/`test` targets
- `description.yml`
- `test/sql/*.test` files in SQLLogicTest format

See [LESSONS.md](LESSONS.md) → Community Extension Submission for the complete guide.

**Critical**: The `[lib] name` in `Cargo.toml` must match the extension name in `description.yml`. See Pitfall P1.

## Important Caveats

### Rust Extensions Require C++ Glue Code

As of DuckDB v1.4.x, **Rust extensions cannot be submitted as pure Rust** to the DuckDB community extensions repository. The community extension build system expects a CMake-based C++ project. Rust extensions must include:

- A thin C++ glue layer that calls into the Rust `cdylib`
- A `CMakeLists.txt` that builds the C++ glue and links the Rust library
- The standard `extension-ci-tools` submodule and `Makefile`

`quack-rs` handles the Rust side of this boundary (entry point, FFI safety, function registration), but you still need the C++ scaffolding. See the [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) repository for a working example of this setup.

**The DuckDB team is developing a [C Extension API](https://github.com/duckdb/duckdb/discussions/14286) that will eventually allow pure Rust extensions without C++ glue.** When that API stabilizes, `quack-rs` will be updated to support it.

### Extension Naming

DuckDB community extensions must have **globally unique names**. Before submitting:

- Check existing extensions at <https://community-extensions.duckdb.org/>
- Use vendor prefixing if needed (e.g., `myorg_analytics` instead of `analytics`)
- Names must match `^[a-z][a-z0-9_-]*$` — use `quack_rs::validate::validate_extension_name` to check

### Security

Community extensions are **not vetted for security** by the DuckDB team. The community extensions repository is a distribution mechanism, not a security guarantee. Extension authors are responsible for the safety and correctness of their code. `quack-rs` enforces `panic = "abort"` and provides safe FFI wrappers, but the overall security of your extension is your responsibility.

## Design Constraints

- **Runtime dependency**: Only `libduckdb-sys = "=1.4.4"` with `loadable-extension` feature.
  The `=` pin is required — the DuckDB C API can change between minor versions.
- **No proc macros**: The SDK uses declarative macros only.
- **No panics in FFI**: All entry points and callbacks use `Result` propagation.
- **MSRV 1.84.1**: Required for `&raw mut` syntax and `const` extern fn support.

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting PRs. All contributions must pass:

```
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check && cargo doc --no-deps
```

## License

MIT — see [LICENSE](LICENSE).
