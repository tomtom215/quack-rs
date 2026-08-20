// WASM shim: re-exports lib.rs as a staticlib for emscripten compilation.
// The [[example]] target in Cargo.toml points here with crate-type = ["staticlib"].
// See extension-ci-tools/makefiles/c_api_extensions/rust.Makefile for details.
#[path = "lib.rs"]
mod lib;
