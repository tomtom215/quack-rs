// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
//
// Compiled only when the `bundled-test` Cargo feature is active.
//
// Exposes DuckDB's internal CreateAPIv1() as a C-linkage function so that the
// Rust test-initialisation code can call it to populate the loadable-extension
// dispatch table from the linked DuckDB symbols.
//
// CreateAPIv1() is an inline C++ function defined inside DuckDB's amalgamated
// C++ header (`duckdb.hpp`). It constructs a duckdb_ext_api_v1 struct where
// every field is set to the corresponding DuckDB C function pointer — exactly
// what we need to initialise the Rust dispatch table via
// duckdb_rs_extension_api_init().
//
// Including the amalgamation rather than the internal `extension_api.hpp` lets
// the same shim compile in both modes:
//   - bundled: against libduckdb-sys's extracted source tree
//   - !bundled: against the official DuckDB release zip (downloaded by
//     libduckdb-sys via DUCKDB_DOWNLOAD_LIB=1, or user-supplied via
//     DUCKDB_LIB_DIR). The release zip ships `duckdb.hpp` but not
//     `extension_api.hpp`.

#include "duckdb.hpp"

extern "C" duckdb_ext_api_v1 quack_rs_create_api_v1() {
    return CreateAPIv1();
}
