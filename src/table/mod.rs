// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` table functions.
//!
//! Table functions are the most powerful extension type — they bridge the gap
//! between "toy" and "real" extensions. Unlike scalar or aggregate functions that
//! transform existing data, table functions **generate** data from scratch: reading
//! files, querying remote APIs, parsing custom formats, or producing synthetic datasets.
//!
//! # Table function lifecycle
//!
//! ```text
//! bind  ─── declare output schema, read parameters, hint cardinality
//! init  ─── allocate global scan state (shared across threads)
//! local_init ─── allocate per-thread scan state (optional, enables parallelism)
//! scan  ─── fill output chunk; repeat until chunk size == 0
//! ```
//!
//! # Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`TableFunctionBuilder`] | Registers the table function with `DuckDB` |
//! | [`BindInfo`] | Ergonomic wrapper for `duckdb_bind_info` in bind callbacks |
//! | [`InitInfo`] | Ergonomic wrapper for `duckdb_init_info` in init callbacks |
//! | [`FunctionInfo`] | Ergonomic wrapper for `duckdb_function_info` in scan callbacks |
//! | [`FfiBindData<T>`] | Type-safe bind-phase data storage |
//! | [`FfiInitData<T>`] | Type-safe global init-phase data storage |
//! | [`FfiLocalInitData<T>`] | Type-safe per-thread init-phase data storage |
//!
//! # Example: Simple table function
//!
//! ```rust,no_run
//! use quack_rs::table::{TableFunctionBuilder, FfiBindData, FfiInitData, BindInfo, InitInfo};
//! use quack_rs::types::TypeId;
//! use libduckdb_sys::{duckdb_bind_info, duckdb_init_info, duckdb_function_info,
//!                     duckdb_data_chunk, duckdb_data_chunk_set_size};
//!
//! struct Config { count: u64 }
//! struct State  { row:   u64 }
//!
//! unsafe extern "C" fn bind(info: duckdb_bind_info) {
//!     unsafe {
//!         BindInfo::new(info)
//!             .add_result_column("n", TypeId::BigInt)
//!             .set_cardinality(3, true);
//!         FfiBindData::<Config>::set(info, Config { count: 3 });
//!     }
//! }
//!
//! unsafe extern "C" fn init(info: duckdb_init_info) {
//!     unsafe { FfiInitData::<State>::set(info, State { row: 0 }); }
//! }
//!
//! unsafe extern "C" fn scan(info: duckdb_function_info, output: duckdb_data_chunk) {
//!     unsafe {
//!         let cfg   = FfiBindData::<Config>::get_from_function(info);
//!         let state = FfiInitData::<State>::get_mut(info);
//!         if let (Some(cfg), Some(state)) = (cfg, state) {
//!             if state.row >= cfg.count {
//!                 duckdb_data_chunk_set_size(output, 0);
//!                 return;
//!             }
//!             // write data into `output`…
//!             state.row += 1;
//!             duckdb_data_chunk_set_size(output, 1);
//!         }
//!     }
//! }
//!
//! // fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
//! //     unsafe {
//! //         TableFunctionBuilder::new("generate_n")
//! //             .bind(bind)
//! //             .init(init)
//! //             .scan(scan)
//! //             .register(con)
//! //     }
//! // }
//! ```

pub mod bind_data;
pub mod builder;
pub mod init_data;

pub use bind_data::FfiBindData;
pub use builder::{BindFn, BindInfo, FunctionInfo, InitFn, InitInfo, ScanFn, TableFunctionBuilder};
pub use init_data::{FfiInitData, FfiLocalInitData};
