// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
//
// Compiled only when the `bundled-test` Cargo feature is active, and only when
// the DuckDB headers have no `duckdb_extension.h` (the DuckDB 1.4 release
// zips). Everywhere else build.rs generates the equivalent C from that header
// instead; see build.rs.
//
// Fills the caller's buffer with DuckDB's internal CreateAPIv1(), an inline C++
// function defined inside DuckDB's amalgamated C++ header (`duckdb.hpp`). It
// constructs a duckdb_ext_api_v1 struct where every field is set to the
// corresponding DuckDB C function pointer — exactly what we need to initialise
// the Rust dispatch table via duckdb_rs_extension_api_init().

#include "duckdb.hpp"

#include <cstddef>

// `out` must point to at least quack_rs_api_v1_size() writable bytes; the Rust
// side checks that size against its bindings before calling.
extern "C" void quack_rs_fill_api_v1(void *out) {
    *static_cast<duckdb_ext_api_v1 *>(out) = CreateAPIv1();
}

extern "C" size_t quack_rs_api_v1_size() {
    return sizeof(duckdb_ext_api_v1);
}

extern "C" const char *quack_rs_header_duckdb_version() {
#ifdef DUCKDB_VERSION
    return "DuckDB " DUCKDB_VERSION;
#else
    return "DuckDB (unknown version)";
#endif
}
