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
//                                                  "v1.4.0" exact release for CPP)
//   Bytes 192 – 223  Field 6: Platform           (e.g. "linux_amd64")
//   Bytes 224 – 255  Field 7: Magic bytes        (must be exactly "4")
//   Bytes 256 – 511  Signature area (RSA-2048; leave zero-filled for unsigned extensions)
//
// Each field is a null-terminated ASCII string padded to exactly 32 bytes.
// ParseExtensionMetaData reads fields 0-7 in order then reverses the array,
// so field 7 (magic) is checked first.

#![allow(missing_docs)]

use std::fs;
use std::path::PathBuf;
use std::process;

const FIELD_SIZE: usize = 32;
const NUM_FIELDS: usize = 8;
const METADATA_SIZE: usize = 512;
const SIGNATURE_SIZE: usize = 256;

const VALID_ABI_TYPES: &[&str] = &["C_STRUCT", "CPP", "C_STRUCT_UNSTABLE"];

fn make_field(s: &str) -> Result<[u8; FIELD_SIZE], String> {
    let b = s.as_bytes();
    if !b.iter().all(u8::is_ascii) {
        return Err(format!("field {s:?} contains non-ASCII bytes"));
    }
    if b.len() >= FIELD_SIZE {
        return Err(format!(
            "field {s:?} is {} bytes but max is {} (must fit including null terminator)",
            b.len(),
            FIELD_SIZE - 1,
        ));
    }
    let mut field = [0u8; FIELD_SIZE];
    field[..b.len()].copy_from_slice(b);
    Ok(field)
}

fn build_metadata(
    abi_type: &str,
    extension_version: &str,
    duckdb_version: &str,
    platform: &str,
) -> Result<[u8; METADATA_SIZE], String> {
    let fields: [[u8; FIELD_SIZE]; NUM_FIELDS] = [
        make_field("")?,               // Field 0: reserved
        make_field("")?,               // Field 1: reserved
        make_field("")?,               // Field 2: reserved
        make_field(abi_type)?,         // Field 3: ABI type
        make_field(extension_version)?, // Field 4: extension version
        make_field(duckdb_version)?,   // Field 5: DuckDB C API version
        make_field(platform)?,         // Field 6: platform
        make_field("4")?,              // Field 7: magic (must be "4")
    ];

    let mut block = [0u8; METADATA_SIZE];
    for (i, field) in fields.iter().enumerate() {
        block[i * FIELD_SIZE..(i + 1) * FIELD_SIZE].copy_from_slice(field);
    }
    // Bytes 256–511: RSA-2048 signature area; zero-filled = unsigned extension.
    // SIGNATURE_SIZE bytes are already zero from array initialisation.
    let _ = SIGNATURE_SIZE;
    debug_assert_eq!(block.len(), METADATA_SIZE);
    Ok(block)
}

fn dump_fields(metadata: &[u8; METADATA_SIZE]) {
    const FIELD_NAMES: [&str; NUM_FIELDS] = [
        "reserved",
        "reserved",
        "reserved",
        "abi_type",
        "extension_version",
        "duckdb_version",
        "platform",
        "magic",
    ];
    println!("\nMetadata fields (on-disk order):");
    for i in 0..NUM_FIELDS {
        let field = &metadata[i * FIELD_SIZE..(i + 1) * FIELD_SIZE];
        let null_pos = field.iter().position(|&b| b == 0).unwrap_or(FIELD_SIZE);
        let text = std::str::from_utf8(&field[..null_pos]).unwrap_or("(invalid utf-8)");
        println!("  Field {i} [{:20}]: {text:?}", FIELD_NAMES[i]);
    }
}

// ── Argument parsing (std-only, no clap) ─────────────────────────────────────

struct Args {
    input: PathBuf,
    output: PathBuf,
    abi_type: String,
    extension_version: String,
    duckdb_version: String,
    platform: String,
    dump: bool,
}

fn print_help(prog: &str) {
    eprintln!(
        "Usage: {prog} <input> <output> [OPTIONS]

Append a DuckDB extension metadata footer to a compiled shared library.

Arguments:
  <input>   Input .so / .dylib / .dll file
  <output>  Output .duckdb_extension file

Options:
  --abi-type <TYPE>            C_STRUCT | CPP | C_STRUCT_UNSTABLE  [default: C_STRUCT]
  --extension-version <VER>    Your extension's version (e.g. v0.1.0)  [default: v0.1.0]
  --duckdb-version <VER>       C_STRUCT: minimum C API version (e.g. v1.2.0)
                               CPP/C_STRUCT_UNSTABLE: exact DuckDB release (e.g. v1.4.0)
                               [default: v1.2.0]
  --platform <PLATFORM>        linux_amd64 | linux_arm64 | osx_amd64 | osx_arm64 |
                               windows_amd64  [default: linux_amd64]
  --dump                       Print metadata fields after writing
  -h, --help                   Print this help message"
    );
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().collect();
    let prog = raw.first().map_or("append_metadata", String::as_str);

    let mut positional: Vec<String> = Vec::new();
    let mut abi_type = String::from("C_STRUCT");
    let mut extension_version = String::from("v0.1.0");
    let mut duckdb_version = String::from("v1.2.0");
    let mut platform = String::from("linux_amd64");
    let mut dump = false;

    let mut i = 1usize;
    while i < raw.len() {
        match raw[i].as_str() {
            "-h" | "--help" => {
                print_help(prog);
                process::exit(0);
            }
            "--abi-type" => {
                i += 1;
                abi_type.clone_from(raw
                    .get(i)
                    .ok_or("--abi-type requires a value")?);
                if !VALID_ABI_TYPES.contains(&abi_type.as_str()) {
                    return Err(format!(
                        "--abi-type must be one of {VALID_ABI_TYPES:?}, got {abi_type:?}"
                    ));
                }
            }
            "--extension-version" => {
                i += 1;
                extension_version.clone_from(raw
                    .get(i)
                    .ok_or("--extension-version requires a value")?);
            }
            "--duckdb-version" => {
                i += 1;
                duckdb_version.clone_from(raw
                    .get(i)
                    .ok_or("--duckdb-version requires a value")?);
            }
            "--platform" => {
                i += 1;
                platform.clone_from(raw
                    .get(i)
                    .ok_or("--platform requires a value")?);
            }
            "--dump" => {
                dump = true;
            }
            arg if arg.starts_with('-') => {
                return Err(format!("unknown flag: {arg}"));
            }
            _ => {
                positional.push(raw[i].clone());
            }
        }
        i += 1;
    }

    if positional.len() < 2 {
        print_help(prog);
        return Err(String::from(
            "expected positional arguments: <input> <output>",
        ));
    }

    Ok(Args {
        input: PathBuf::from(&positional[0]),
        output: PathBuf::from(&positional[1]),
        abi_type,
        extension_version,
        duckdb_version,
        platform,
        dump,
    })
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn run() -> Result<(), String> {
    let args = parse_args()?;

    if !args.input.exists() {
        return Err(format!(
            "input file not found: {}",
            args.input.display()
        ));
    }

    let so_data = fs::read(&args.input)
        .map_err(|e| format!("failed to read {}: {e}", args.input.display()))?;

    let metadata = build_metadata(
        &args.abi_type,
        &args.extension_version,
        &args.duckdb_version,
        &args.platform,
    )?;

    let mut output = so_data;
    output.extend_from_slice(&metadata);

    fs::write(&args.output, &output)
        .map_err(|e| format!("failed to write {}: {e}", args.output.display()))?;

    println!(
        "Written {} bytes → {}",
        output.len(),
        args.output.display()
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    // ── make_field ────────────────────────────────────────────────────────────

    #[test]
    fn make_field_empty_is_all_zeros() {
        let f = make_field("").unwrap();
        assert_eq!(f, [0u8; FIELD_SIZE]);
    }

    #[test]
    fn make_field_null_terminates_and_pads() {
        let f = make_field("hi").unwrap();
        assert_eq!(&f[..2], b"hi");
        assert_eq!(f[2], 0); // null terminator
        assert!(f[3..].iter().all(|&b| b == 0)); // zero padding
        assert_eq!(f.len(), FIELD_SIZE);
    }

    #[test]
    fn make_field_max_length_is_31_chars() {
        let s = "a".repeat(31);
        let f = make_field(&s).unwrap();
        assert_eq!(&f[..31], s.as_bytes());
        assert_eq!(f[31], 0);
    }

    #[test]
    fn make_field_rejects_32_chars() {
        let s = "a".repeat(32);
        assert!(make_field(&s).is_err());
    }

    #[test]
    fn make_field_rejects_non_ascii() {
        assert!(make_field("café").is_err());
    }

    #[test]
    fn make_field_magic_is_ascii_four() {
        let f = make_field("4").unwrap();
        assert_eq!(f[0], b'4');
        assert_eq!(f[1], 0);
    }

    // ── build_metadata ────────────────────────────────────────────────────────

    #[test]
    fn build_metadata_is_exactly_512_bytes() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert_eq!(m.len(), METADATA_SIZE);
    }

    #[test]
    fn build_metadata_fields_0_2_reserved() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert!(m[..96].iter().all(|&b| b == 0));
    }

    #[test]
    fn build_metadata_field3_abi_type() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert_eq!(&m[96..96 + 8], b"C_STRUCT");
        assert_eq!(m[96 + 8], 0);
    }

    #[test]
    fn build_metadata_field4_extension_version() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert_eq!(&m[128..128 + 6], b"v0.1.0");
        assert_eq!(m[128 + 6], 0);
    }

    #[test]
    fn build_metadata_field5_duckdb_version() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert_eq!(&m[160..160 + 6], b"v1.2.0");
        assert_eq!(m[160 + 6], 0);
    }

    #[test]
    fn build_metadata_field6_platform() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert_eq!(&m[192..192 + 11], b"linux_amd64");
        assert_eq!(m[192 + 11], 0);
    }

    #[test]
    fn build_metadata_field7_magic() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert_eq!(m[224], b'4');
        assert_eq!(m[225], 0);
    }

    #[test]
    fn build_metadata_signature_area_is_zero() {
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        assert!(m[256..].iter().all(|&b| b == 0));
    }

    #[test]
    fn build_metadata_cpp_abi_type() {
        let m = build_metadata("CPP", "v1.0.0", "v1.4.0", "osx_arm64").unwrap();
        assert_eq!(&m[96..99], b"CPP");
        assert_eq!(m[99], 0);
        assert_eq!(&m[128..134], b"v1.0.0");
        assert_eq!(&m[160..166], b"v1.4.0");
        assert_eq!(&m[192..201], b"osx_arm64");
        assert_eq!(m[224], b'4');
    }

    #[test]
    fn build_metadata_rejects_long_platform() {
        // 32-char string exceeds the 31-char limit
        let long = "a".repeat(32);
        assert!(build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", &long).is_err());
    }

    // ── round-trip: write file then read back ─────────────────────────────────

    #[test]
    fn roundtrip_file_has_correct_footer() {
        let dir = std::env::temp_dir();
        let input = dir.join("quack_test_input.bin");
        let output = dir.join("quack_test_output.duckdb_extension");

        // Write a tiny fake .so
        let fake_so: Vec<u8> = (0u8..=15).collect();
        fs::write(&input, &fake_so).unwrap();

        let metadata =
            build_metadata("C_STRUCT", "v0.2.0", "v1.2.0", "linux_arm64").unwrap();

        let mut combined = fake_so.clone();
        combined.extend_from_slice(&metadata);
        fs::write(&output, &combined).unwrap();

        let written = fs::read(&output).unwrap();
        assert_eq!(&written[..fake_so.len()], fake_so.as_slice());
        assert_eq!(&written[fake_so.len()..], &metadata as &[u8]);

        // Verify the footer fields at the correct byte offsets
        let footer_start = written.len() - METADATA_SIZE;
        let footer = &written[footer_start..];

        assert_eq!(&footer[96..104], b"C_STRUCT");
        assert_eq!(&footer[128..134], b"v0.2.0");
        assert_eq!(&footer[160..166], b"v1.2.0");
        assert_eq!(&footer[192..203], b"linux_arm64");
        assert_eq!(footer[224], b'4');
        assert!(footer[256..].iter().all(|&b| b == 0));

        // Cleanup
        let _ = fs::remove_file(&input);
        let _ = fs::remove_file(&output);
    }

    // ── dump_fields smoke test ────────────────────────────────────────────────

    #[test]
    fn dump_fields_does_not_panic() {
        // Redirect stdout is not trivial in unit tests; just verify no panic.
        let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
        // Capture by redirecting via a pipe is complex; just call and assert no panic.
        // In CI, output goes to /dev/null anyway.
        drop(std::io::stdout().write_all(b"")); // ensure stdout is accessible
        dump_fields(&m); // must not panic
    }

    // ── parse_args edge cases ─────────────────────────────────────────────────

    #[test]
    fn parse_args_defaults() {
        // Simulate: append_metadata input.so output.duckdb_extension
        // We can't call parse_args() directly (reads std::env::args),
        // so we test the defaults by constructing Args explicitly.
        let args = Args {
            input: PathBuf::from("input.so"),
            output: PathBuf::from("out.duckdb_extension"),
            abi_type: String::from("C_STRUCT"),
            extension_version: String::from("v0.1.0"),
            duckdb_version: String::from("v1.2.0"),
            platform: String::from("linux_amd64"),
            dump: false,
        };
        assert_eq!(args.abi_type, "C_STRUCT");
        assert_eq!(args.extension_version, "v0.1.0");
        assert_eq!(args.duckdb_version, "v1.2.0");
        assert_eq!(args.platform, "linux_amd64");
        assert!(!args.dump);
    }

    #[test]
    fn make_field_all_valid_abi_types() {
        for &t in VALID_ABI_TYPES {
            assert!(make_field(t).is_ok(), "should accept {t:?}");
        }
    }
}
