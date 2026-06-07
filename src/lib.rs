// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! # quack-rs
//!
//! A production-grade Rust SDK for building `DuckDB` loadable extensions.
//!
//! ## Overview
//!
//! `quack-rs` encapsulates the hard-won FFI knowledge required to build `DuckDB`
//! community extensions in Rust. It provides:
//!
//! - A correct, panic-free entry point helper via the [`entry_point`](mod@entry_point) module
//! - Type-safe builders for registering aggregate functions ([`aggregate`])
//! - Safe vector reading and writing helpers ([`vector`])
//! - A generic [`FfiState<T>`][aggregate::state::FfiState] that eliminates raw pointer management
//! - Documented solutions to every known `DuckDB` Rust FFI pitfall
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! // In your extension's src/lib.rs, write the entry point manually:
//! use quack_rs::entry_point::init_extension;
//!
//! #[no_mangle]
//! pub unsafe extern "C" fn my_extension_init_c_api(
//!     info: libduckdb_sys::duckdb_extension_info,
//!     access: *const libduckdb_sys::duckdb_extension_access,
//! ) -> bool {
//!     unsafe {
//!         init_extension(info, access, quack_rs::DUCKDB_API_VERSION, |con| {
//!             // register_my_function(con)?;
//!             Ok(())
//!         })
//!     }
//! }
//! ```
//!
//! ## Architecture
//!
//! The SDK is organized into focused modules:
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`callback`] | Safe `extern "C"` callback wrapper macros (`scalar_callback!`, `table_scan_callback!`) |
//! | [`chunk_writer`] | Auto-sizing chunk writer for table scan callbacks (auto `set_size` on drop) |
//! | [`data_chunk`] | Ergonomic wrapper for `DuckDB` data chunks |
//! | [`entry_point`](mod@entry_point) | Helper for the correct `{name}_init_c_api` C entry point |
//! | [`connection`] | `Connection` facade + `Registrar` trait for version-agnostic registration |
//! | [`aggregate`] | Builders for aggregate function registration |
//! | [`scalar`] | Builder for scalar function registration |
//! | [`cast`] | Builder for custom type cast functions |
//! | [`table`] | Builders for table function registration (raw `TableFunctionBuilder` + closure-based `TypedTableFunctionBuilder<S>`) |
//! | [`replacement_scan`] | `SELECT * FROM 'file.xyz'` replacement scan registration |
//! | [`sql_macro`] | SQL macro registration (`CREATE MACRO`) — no FFI callbacks |
//! | [`vector`] | Safe helpers for reading/writing `DuckDB` data vectors |
//! | [`vector::complex`] | STRUCT / LIST / MAP / ARRAY vector access (child vectors, offsets) |
//! | [`vector::struct_reader`] | Batched [`StructReader`][vector::StructReader] for STRUCT input vectors |
//! | [`vector::struct_writer`] | Batched [`StructWriter`][vector::StructWriter] for STRUCT output vectors |
//! | [`types`] | `DuckDB` type system wrappers (`TypeId`, `LogicalType`) |
//! | [`interval`] | `INTERVAL` → microseconds conversion with overflow checking |
//! | [`error`] | `ExtensionError` for FFI error propagation |
//! | [`config`] | RAII wrapper for `DuckDB` database configuration |
//! | [`value`] | RAII wrapper for `DuckDB` values with typed extraction |
//! | [`tls`] | Type-erased TLS configuration provider for HTTP-capable extensions |
//! | [`warning`] | Structured security warning API (`ExtensionWarning`, `WarningCollector`) |
//! | [`secrets`] | Secrets manager bridge trait (`SecretsManager`, `SecretEntry`) |
//! | [`validate`] | Community extension compliance validators |
//! | [`validate::description_yml`] | Parse and validate `description.yml` metadata |
//! | [`scaffold`] | Project generator for new extensions (no C++ glue needed) |
//! | [`testing`] | Test harness for aggregate state logic |
//! | [`prelude`] | Convenience re-exports of the most commonly used items |
//! | `appender` | Bulk row appender (`duckdb-1-5` feature) |
//! | `catalog` | Catalog entry lookup (`duckdb-1-5` feature) |
//! | `client_context` | Client context access (`duckdb-1-5` feature) |
//! | `config_option` | Extension-defined configuration options (`duckdb-1-5` feature) |
//! | `copy_function` | Custom `COPY TO` handlers (`duckdb-1-5` feature) |
//! | `error_data` | Structured error type + UTF-8 validation (`duckdb-1-5` feature) |
//! | `expression` | Bound expression inspection / constant folding (`duckdb-1-5` feature) |
//! | `file_system` | `DuckDB` virtual file system access (`duckdb-1-5` feature) |
//! | `instance_cache` | Shared database instance cache (`duckdb-1-5` feature) |
//! | `selection_vector` | Zero-copy row-index selection vectors (`duckdb-1-5` feature) |
//! | `table_description` | Table metadata queries (`duckdb-1-5` feature) |
//!
//! ## Safety
//!
//! All `unsafe` code within this SDK is sound and documented. Extension authors
//! must write `unsafe extern "C"` callback functions (required by `DuckDB`'s C API),
//! but the SDK's helpers minimize the surface area of unsafe code within those
//! callbacks. Every `unsafe` block inside this crate has a `// SAFETY:` comment
//! explaining the invariants being upheld.
//!
//! ## Design Principles
//!
//! 1. **Thin wrapper**: every abstraction must pay for itself in reduced boilerplate
//!    or improved safety. When in doubt, prefer simplicity.
//! 2. **No panics across FFI**: `unwrap()` is forbidden in FFI callbacks and entry points.
//! 3. **Bounded version range**: `libduckdb-sys` uses `>=1.4.4, <2` to support `DuckDB` 1.4.x
//!    and 1.5.x (through v1.5.3) while preventing silent adoption of breaking changes in
//!    future major releases.
//! 4. **Testable business logic**: state structs have zero FFI dependencies.
//!
//! ## Pitfalls
//!
//! See [`LESSONS.md`](https://github.com/tomtom215/quack-rs/blob/main/LESSONS.md)
//! for all 16 known `DuckDB` Rust FFI pitfalls, including symptoms, root causes, and fixes.
//!
//! ## Pitfall L1: COMBINE must propagate config fields
//!
//! `DuckDB`'s segment tree creates fresh zero-initialized target states via
//! `state_init`, then calls `combine` to merge source into them. This means
//! your `combine` callback MUST copy ALL configuration fields from source to
//! target — not just accumulated data. Any field that defaults to zero will
//! be wrong at finalize time, producing silently incorrect results.
//!
//! See [`aggregate::callbacks::CombineFn`] for details.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

// quack-rs supports 64-bit targets and `wasm32-unknown-emscripten` (the
// DuckDB-WASM target). The `duckdb_string_t` layout reserves 8 bytes for the
// heap pointer regardless of pointer width — see `vector::string` for details.
#[cfg(not(any(target_pointer_width = "64", target_arch = "wasm32")))]
compile_error!("quack-rs supports 64-bit targets and wasm32-unknown-emscripten.");

pub mod aggregate;
pub mod callback;
pub mod cast;
pub mod chunk_writer;
pub mod config;
pub mod connection;
pub mod data_chunk;
pub mod entry_point;
pub mod error;
pub mod interval;
pub mod prelude;
pub mod replacement_scan;
pub mod scaffold;
pub mod scalar;
pub mod secrets;
pub mod sql_macro;
pub mod table;
pub mod testing;
pub mod tls;
pub mod types;
pub mod validate;
pub mod value;
pub mod vector;
pub mod warning;

// DuckDB 1.5.0+ modules — gated behind the `duckdb-1-5` feature flag.
#[cfg(feature = "duckdb-1-5")]
pub mod appender;
#[cfg(feature = "duckdb-1-5")]
pub mod catalog;
#[cfg(feature = "duckdb-1-5")]
pub mod client_context;
#[cfg(feature = "duckdb-1-5")]
pub mod config_option;
#[cfg(feature = "duckdb-1-5")]
pub mod copy_function;
#[cfg(feature = "duckdb-1-5")]
pub mod error_data;
#[cfg(feature = "duckdb-1-5")]
pub mod expression;
#[cfg(feature = "duckdb-1-5")]
pub mod file_system;
#[cfg(feature = "duckdb-1-5")]
pub mod instance_cache;
#[cfg(feature = "duckdb-1-5")]
pub mod selection_vector;
#[cfg(feature = "duckdb-1-5")]
pub mod table_description;

/// The `DuckDB` C API version string required by [`duckdb_rs_extension_api_init`][libduckdb_sys::duckdb_rs_extension_api_init].
///
/// This constant corresponds to every `DuckDB` release from v1.4.x through
/// v1.5.3: the C extension API version has remained `v1.2.0` across all of them
/// (it did **not** change in the v1.5.1, v1.5.2, or v1.5.3 patch releases). If
/// you are targeting a different `DuckDB` release, consult the `DuckDB` changelog
/// for the C API version.
///
/// # Pitfall P2: C API version ≠ `DuckDB` release version
///
/// The `-dv` flag passed to `append_extension_metadata.py` must be this value
/// (`"v1.2.0"`), **not** the `DuckDB` release version (`"v1.4.4"` / `"v1.5.0"` /
/// `"v1.5.3"`). Using the wrong value causes the metadata script to fail silently
/// or produce incorrect metadata.
///
/// See `LESSONS.md` → Pitfall P2 for full details.
pub const DUCKDB_API_VERSION: &str = "v1.2.0";
