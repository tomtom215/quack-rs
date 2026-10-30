// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
//
// Append a DuckDB extension metadata block to a compiled .so / .dylib / .dll file,
// producing a loadable .duckdb_extension file.
//
// Authoritative layout sourced from DuckDB 1.4.4 source:
//   src/main/extension/extension_load.cpp  ParseExtensionMetaData()
//   src/include/duckdb/main/extension.hpp  ParsedExtensionMetaData
//
// DuckDB reads the LAST 512 bytes of a .duckdb_extension file as the footer.
// The footer layout is:
//
//   Bytes   0 –  31  Field 0: reserved (zero-filled)
//   Bytes  32 –  63  Field 1: reserved (zero-filled)
//   Bytes  64 –  95  Field 2: reserved (zero-filled)
//   Bytes  96 – 127  Field 3: ABI type           ("C_STRUCT" | "C_STRUCT_UNSTABLE" | "CPP")
//   Bytes 128 – 159  Field 4: Extension version  (e.g. "v0.1.0")
//   Bytes 160 – 191  Field 5: DuckDB version     ("v1.2.0" C API min for C_STRUCT;
//                                                  exact release for CPP / C_STRUCT_UNSTABLE)
//   Bytes 192 – 223  Field 6: Platform           (e.g. "linux_amd64")
//   Bytes 224 – 255  Field 7: Magic bytes        (must be exactly "4")
//   Bytes 256 – 511  Signature area (RSA-2048; leave zero-filled for unsigned extensions)
//
// Each field is a null-terminated ASCII string padded to exactly 32 bytes.
// ParseExtensionMetaData reads fields 0-7 in order then reverses the array,
// so field 7 (magic) is checked first.
//
// WebAssembly: extension-ci-tools' append_extension_metadata.py also writes a
// 22-byte WebAssembly custom-section header (`duckdb_signature`) in front of
// the footer, for every platform. Native loaders read only the last 512 bytes
// and never see it, so this tool writes it only with `--wasm` — which a
// DuckDB-Wasm extension needs, because a module with raw bytes after its last
// section is not valid WebAssembly. See `footer::WASM_SECTION_HEADER`.

#![allow(missing_docs)]

mod cli;
mod entry;
mod footer;
mod output;

use std::process;

use cli::{Args, Command};
use footer::{build_metadata, dump_fields, existing_stamp_len, WASM_SECTION_HEADER};

/// Produces the stamped file from the library bytes.
///
/// # Errors
///
/// When a field does not fit, or `data` already ends in a footer and
/// `args.replace` is not set: appending a second footer would leave the first
/// one inside the file, and `DuckDB` would read only the new one — silently
/// discarding whatever the first stamp said.
fn stamp(mut data: Vec<u8>, args: &Args) -> Result<(Vec<u8>, [u8; footer::METADATA_SIZE]), String> {
    if let Some(len) = existing_stamp_len(&data) {
        if !args.replace {
            return Err(format!(
                "{} already ends in a DuckDB metadata footer; pass --replace to overwrite it",
                args.input.display()
            ));
        }
        data.truncate(data.len() - len);
    }
    let metadata = build_metadata(
        &args.abi_type,
        &args.extension_version,
        &args.duckdb_version,
        &args.platform,
    )?;
    if args.wasm {
        data.extend_from_slice(&WASM_SECTION_HEADER);
    }
    data.extend_from_slice(&metadata);
    Ok((data, metadata))
}

fn run() -> Result<(), String> {
    // `args_os`, not `args`: the latter panics on an argument that is not
    // UTF-8, and a file name on Unix can be any bytes but NUL (HARDEN-4).
    let raw: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let prog = raw.first().map_or_else(
        || "append_metadata".into(),
        |p| p.to_string_lossy().into_owned(),
    );
    let prog = prog.as_str();
    let args = match cli::parse(raw.get(1..).unwrap_or_default()) {
        Ok(Command::Help) => {
            eprintln!("{}", cli::help(prog));
            return Ok(());
        }
        Ok(Command::Run(args)) => args,
        Err(e) => return Err(format!("{e}\n\nRun `{prog} --help` for usage.")),
    };
    for warning in &args.warnings {
        eprintln!("warning: {warning}");
    }

    let so_data = output::read_input(&args.input)?;
    // A C API extension is found by `<name>_init_c_api`, `<name>` being the
    // output's file name. A WebAssembly module's export names are not
    // NUL-terminated strings, and a C++ extension has other entry points.
    if args.abi_type != "CPP" && !args.wasm {
        if let Some(note) = entry::entry_point_note(&so_data, &args.output) {
            eprintln!("warning: {note}");
        }
    }
    let (stamped, metadata) = stamp(so_data, &args)?;

    output::write_atomically(&args.output, &stamped)?;

    println!(
        "Written {} bytes → {} ({} for {})",
        stamped.len(),
        args.output.display(),
        args.abi_type,
        args.platform
    );

    if args.dump {
        dump_fields(&metadata);
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_output;
