# quack-rs

[![CI](https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/quack-rs.svg)](https://crates.io/crates/quack-rs)
[![docs.rs](https://img.shields.io/docsrs/quack-rs)](https://docs.rs/quack-rs)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![MSRV: 1.84.1](https://img.shields.io/badge/MSRV-1.84.1-blue.svg)](https://blog.rust-lang.org/2025/01/30/Rust-1.84.1.html)

**A Rust SDK for building DuckDB loadable extensions.**

Provides safe wrappers for the DuckDB C API, builders for aggregate and scalar function registration, and a project scaffold generator for community extension submission.

## Why quack-rs?

DuckDB's Rust FFI surface has undocumented pitfalls — struct layouts, callback contracts, and initialization sequences that are not covered in the DuckDB documentation or the `libduckdb-sys` crate docs. `quack-rs` was extracted from [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral), a production DuckDB community extension, where each of these problems was encountered and solved.

### What quack-rs Solves

Each row below corresponds to a documented pitfall in [LESSONS.md](LESSONS.md) with symptoms, root cause, and fix.

| Pitfall | Without quack-rs | With quack-rs |
|---------|-----------------|---------------|
| P3: Entry point SEGFAULT | `extract_raw_connection` dereferences invalid pointer | `init_extension` helper with correct init sequence |
| L6: Silent registration failure | Function set members missing names → silently ignored | `AggregateFunctionSetBuilder` sets name on each member |
| L1: combine config loss | Zero-initialized target state → wrong results | `AggregateTestHarness` catches missing field propagation |
| L4: NULL output crash | Forgetting `ensure_validity_writable` → SEGFAULT | `VectorWriter::set_null` calls it automatically |
| L5: Boolean UB | `as bool` cast on non-0/1 value → undefined behavior | `VectorReader::read_bool` reads `u8 != 0` |
| L7: Memory leak | `duckdb_logical_type` not freed after registration | `LogicalType` RAII wrapper with `Drop` |
| P7: `duckdb_string_t` format | Two undocumented inline/pointer layouts | `read_duck_string` handles both formats |
| P8: C API version confusion | Metadata script fails with DuckDB release version | `DUCKDB_API_VERSION` constant documents the distinction |

The SDK also generates complete, submission-ready extension projects via [`scaffold::generate_scaffold`](#pure-rust-extensions-via-the-c-extension-api) — no C++ glue required.

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
| `scaffold` | Project generator for new extensions (no C++ glue needed) |
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

## Pure Rust Extensions via the C Extension API

DuckDB's [C Extension API](https://github.com/duckdb/duckdb/pull/12682) now allows **pure Rust extensions without any C++ glue**. The official [extension-template-rs](https://github.com/duckdb/extension-template-rs) demonstrates this approach.

`quack-rs` includes a **scaffold generator** that produces all the files you need:

```rust
use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};

let config = ScaffoldConfig {
    name: "my_analytics".to_string(),
    description: "Fast analytics for DuckDB".to_string(),
    version: "0.1.0".to_string(),
    license: "MIT".to_string(),
    maintainer: "Your Name".to_string(),
    github_repo: "yourorg/duckdb-my-analytics".to_string(),
    excluded_platforms: vec![],
};

let files = generate_scaffold(&config).unwrap();
// Generates: Cargo.toml, Makefile, src/lib.rs, description.yml, .gitmodules, .gitignore
```

The generated project uses the C Extension API directly — no CMakeLists.txt, no C++ files. It includes the `extension-ci-tools` submodule for CI/CD and a `Makefile` that delegates to `cargo build`.

## Extension Naming

DuckDB community extensions must have **globally unique names**. Before submitting:

- Check existing extensions at <https://community-extensions.duckdb.org/>
- Use vendor prefixing if needed (e.g., `myorg_analytics` instead of `analytics`)
- Names must match `^[a-z][a-z0-9_-]*$` — use `quack_rs::validate::validate_extension_name` to check

## Security

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

## AI-Assisted Development

This project was developed with assistance from [Claude Code](https://claude.ai/code) (Anthropic). AI was used for code generation, documentation drafting, test authoring, and research into DuckDB's C Extension API and community extension infrastructure. All AI-generated code was reviewed, tested, and validated by the project maintainer.

## License

MIT — see [LICENSE](LICENSE).
