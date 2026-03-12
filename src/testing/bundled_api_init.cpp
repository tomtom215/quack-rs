// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
//
// Compiled only when the `bundled-test` Cargo feature is active.
//
// Exposes DuckDB's internal CreateAPIv1() as a C-linkage function so that the
// Rust test-initialisation code can call it to populate the loadable-extension
// dispatch table from the bundled DuckDB symbols.
//
// CreateAPIv1() is defined as an inline C++ function in
// duckdb/main/capi/extension_api.hpp.  It constructs a duckdb_ext_api_v1
// struct where every field is set to the corresponding bundled DuckDB C
// function pointer — exactly what we need to initialise the Rust dispatch
// table via duckdb_rs_extension_api_init().

#include "duckdb/main/capi/extension_api.hpp"

extern "C" duckdb_ext_api_v1 quack_rs_create_api_v1() {
    return CreateAPIv1();
}
