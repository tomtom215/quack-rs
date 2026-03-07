// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

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
//! | [`entry_point`](mod@entry_point) | Helper for the correct `{name}_init_c_api` C entry point |
//! | [`aggregate`] | Builders for aggregate function registration |
//! | [`scalar`] | Builder for scalar function registration |
//! | [`sql_macro`] | SQL macro registration (`CREATE MACRO`) — no FFI callbacks |
//! | [`vector`] | Safe helpers for reading/writing `DuckDB` data vectors |
//! | [`types`] | `DuckDB` type system wrappers (`TypeId`, `LogicalType`) |
//! | [`interval`] | `INTERVAL` → microseconds conversion with overflow checking |
//! | [`error`] | `ExtensionError` for FFI error propagation |
//! | [`validate`] | Community extension compliance validators |
//! | [`validate::description_yml`] | Parse and validate `description.yml` metadata |
//! | [`scaffold`] | Project generator for new extensions (no C++ glue needed) |
//! | [`testing`] | Test harness for aggregate state logic |
//! | [`prelude`] | Convenience re-exports of the most commonly used items |
//!
//! ## Safety
//!
//! All unsafe code is confined to this SDK. Extension authors using the high-level
//! API write 100% safe Rust. Every `unsafe` block inside this crate has a
//! `// SAFETY:` comment explaining the invariants being upheld.
//!
//! ## Design Principles
//!
//! 1. **Thin wrapper**: every abstraction must pay for itself in reduced boilerplate
//!    or improved safety. When in doubt, prefer simplicity.
//! 2. **No panics across FFI**: `unwrap()` is forbidden in FFI callbacks and entry points.
//! 3. **Exact version pin**: `libduckdb-sys` is pinned with `=` because the `DuckDB` C API
//!    can change between minor versions.
//! 4. **Testable business logic**: state structs have zero FFI dependencies.
//!
//! ## Pitfalls
//!
//! See [`LESSONS.md`](https://github.com/tomtom215/quack-rs/blob/main/LESSONS.md)
//! for all 15 known `DuckDB` Rust FFI pitfalls, including symptoms, root causes, and fixes.
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

pub mod aggregate;
pub mod entry_point;
pub mod error;
pub mod interval;
pub mod prelude;
pub mod scaffold;
pub mod scalar;
pub mod sql_macro;
pub mod testing;
pub mod types;
pub mod validate;
pub mod vector;

/// The `DuckDB` C API version string required by [`duckdb_rs_extension_api_init`][libduckdb_sys::duckdb_rs_extension_api_init].
///
/// This constant corresponds to `DuckDB` release v1.4.x. If you are targeting a
/// different `DuckDB` release, consult the `DuckDB` changelog for the C API version.
///
/// # Pitfall P8: C API version ≠ `DuckDB` release version
///
/// The `-dv` flag passed to `append_extension_metadata.py` must be this value
/// (`"v1.2.0"`), **not** the `DuckDB` release version (`"v1.4.4"`). Using the wrong
/// value causes the metadata script to fail silently or produce incorrect metadata.
///
/// See `LESSONS.md` → Pitfall P8 for full details.
pub const DUCKDB_API_VERSION: &str = "v1.2.0";
