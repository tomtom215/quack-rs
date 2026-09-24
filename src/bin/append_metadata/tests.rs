// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

use super::*;
use crate::footer::{field_text, make_field, METADATA_SIZE, VALID_ABI_TYPES};
use std::io::Write as _;

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

fn parse_ok(s: &str) -> Args {
    match cli::parse(&argv(s)) {
        Ok(Command::Run(args)) => args,
        other => panic!("{s}: {other:?}"),
    }
}

fn parse_err(s: &str) -> String {
    match cli::parse(&argv(s)) {
        Err(e) => e,
        other => panic!("{s} must be rejected, got {other:?}"),
    }
}

// ── make_field ────────────────────────────────────────────────────────────

#[test]
fn make_field_empty_is_all_zeros() {
    let f = make_field("").unwrap();
    assert_eq!(f, [0u8; footer::FIELD_SIZE]);
}

#[test]
fn make_field_null_terminates_and_pads() {
    let f = make_field("hi").unwrap();
    assert_eq!(&f[..2], b"hi");
    assert_eq!(f[2], 0); // null terminator
    assert!(f[3..].iter().all(|&b| b == 0)); // zero padding
    assert_eq!(f.len(), footer::FIELD_SIZE);
}

#[test]
fn make_field_max_length_is_31_chars() {
    let s = "a".repeat(31);
    let f = make_field(&s).unwrap();
    assert_eq!(&f[..31], s.as_bytes());
    assert_eq!(f[31], 0);
}

/// `DuckDB` reads each field as 32 bytes and strips trailing NULs
/// (`FilterZeroAtEnd`, `extension_load.cpp`), and extension-ci-tools pads to
/// exactly 32 — so a 32-byte value needs no terminator and is accepted there.
#[test]
fn make_field_accepts_exactly_32_chars_without_a_terminator() {
    let s = "a".repeat(32);
    let f = make_field(&s).unwrap();
    assert_eq!(&f[..], s.as_bytes());
    assert_eq!(field_text(&f, 0), s);
}

#[test]
fn make_field_rejects_33_chars() {
    let s = "a".repeat(33);
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
    // 33 bytes do not fit in a 32-byte field
    let long = "a".repeat(33);
    assert!(build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", &long).is_err());
}

#[test]
fn build_metadata_keeps_a_32_byte_field_intact() {
    let version = "v1.2.3-".to_string() + &"x".repeat(25);
    assert_eq!(version.len(), 32);
    let m = build_metadata("C_STRUCT", &version, "v1.2.0", "linux_amd64").unwrap();
    assert_eq!(field_text(&m, 4), version);
    // The neighbouring fields are untouched.
    assert_eq!(field_text(&m, 3), "C_STRUCT");
    assert_eq!(field_text(&m, 5), "v1.2.0");
}

/// `--flag=value` is the other spelling every std-style CLI accepts; it used
/// to be "unknown flag".
#[test]
fn flags_accept_the_equals_form() {
    let a = parse_ok(
        "in.so out.ext --abi-type=C_STRUCT_UNSTABLE --extension-version=v2.0.0 \
         --duckdb-version=v1.5.5 --platform=linux_arm64",
    );
    assert_eq!(a.abi_type, "C_STRUCT_UNSTABLE");
    assert_eq!(a.extension_version, "v2.0.0");
    assert_eq!(a.duckdb_version, "v1.5.5");
    assert_eq!(a.platform, "linux_arm64");
    // Mixed forms work, and repetition is still caught across them.
    let a = parse_ok("in.so out.ext --platform linux_amd64 --extension-version=v1.0.0");
    assert_eq!(a.platform, "linux_amd64");
    assert!(
        parse_err("in.so out.ext --platform=linux_amd64 --platform linux_arm64")
            .contains("more than once")
    );
    // An empty value is an error, not a default.
    assert!(parse_err("in.so out.ext --platform=").contains("requires a value"));
    // Switches take no value.
    assert!(parse_err("in.so out.ext --dump=yes").contains("takes no value"));
    assert!(parse_err("in.so out.ext --bogus=1").contains("unknown flag"));
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

    let metadata = build_metadata("C_STRUCT", "v0.2.0", "v1.2.0", "linux_arm64").unwrap();

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
    let m = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", "linux_amd64").unwrap();
    drop(std::io::stdout().write_all(b"")); // ensure stdout is accessible
    dump_fields(&m); // must not panic
}

#[test]
fn make_field_all_valid_abi_types() {
    for &t in VALID_ABI_TYPES {
        assert!(make_field(t).is_ok(), "should accept {t:?}");
    }
}

// ── argument parsing ─────────────────────────────────────────────────────

#[test]
fn parse_args_defaults() {
    let args = parse_ok("in.so out.duckdb_extension");
    assert_eq!(args.input, std::path::PathBuf::from("in.so"));
    assert_eq!(
        args.output,
        std::path::PathBuf::from("out.duckdb_extension")
    );
    assert_eq!(args.abi_type, "C_STRUCT");
    assert_eq!(args.extension_version, "v0.1.0");
    assert_eq!(args.duckdb_version, quack_rs::DUCKDB_API_VERSION);
    // The platform defaults to the host's rather than to a hard-coded
    // linux_amd64, which stamped every macOS/Windows/arm64 build wrong.
    assert_eq!(Some(args.platform.as_str()), cli::host_platform());
    assert!(!args.dump && !args.replace && !args.wasm);
    assert!(args.warnings.is_empty(), "{:?}", args.warnings);
}

#[test]
fn a_flag_is_not_taken_as_the_previous_flags_value() {
    // `--platform --dump` used to stamp the platform "--dump".
    let err = parse_err("in out --platform --dump");
    assert!(err.contains("--platform requires a value"), "{err}");
    let err = parse_err("in out --duckdb-version");
    assert!(err.contains("requires a value"), "{err}");
}

#[test]
fn extra_positional_arguments_are_rejected() {
    // A third path used to be ignored, so a mistyped command stamped the
    // wrong file without a word.
    let err = parse_err("in out stray");
    assert!(err.contains("exactly two positional"), "{err}");
    assert!(parse_err("in").contains("<input> <output>"));
}

#[test]
fn repeated_and_unknown_flags_are_rejected() {
    assert!(
        parse_err("in out --platform linux_amd64 --platform osx_arm64").contains("more than once")
    );
    assert!(parse_err("in out --plattform linux_amd64").contains("unknown flag"));
}

#[test]
fn empty_versions_are_rejected() {
    assert!(cli::parse(&[
        "in".into(),
        "out".into(),
        "--extension-version".into(),
        String::new()
    ])
    .is_err());
    assert!(cli::parse(&[
        "in".into(),
        "out".into(),
        "--duckdb-version".into(),
        String::new()
    ])
    .is_err());
}

/// `LESSONS.md` P2: for `C_STRUCT` the field is the C API version. `DuckDB` 1.5.5
/// refuses a file claiming C API v1.5.5 ("requires a newer C API"), so the
/// stamp was useless — and the tool used to write it without complaint.
#[test]
fn a_duckdb_release_as_the_c_struct_version_is_rejected() {
    let err = parse_err("in out --abi-type C_STRUCT --duckdb-version v1.5.5");
    assert!(err.contains("P2"), "{err}");
    assert!(err.contains("C_STRUCT_UNSTABLE"), "{err}");
    for bad in ["1.2.0", "v1.2", "v2.0.0", "v1.3.0", "latest"] {
        parse_err(&format!("in out --duckdb-version {bad}"));
    }
    for good in ["v1.2.0", "v1.1.0", "v1.0.0"] {
        parse_ok(&format!("in out --duckdb-version {good}"));
    }
}

#[test]
fn unstable_and_cpp_take_a_release_or_a_commit_hash() {
    let args = parse_ok("in out --abi-type C_STRUCT_UNSTABLE --duckdb-version v1.5.5");
    assert_eq!(args.duckdb_version, "v1.5.5");
    parse_ok("in out --abi-type CPP --duckdb-version 1f0067f1a5");
    parse_err("in out --abi-type C_STRUCT_UNSTABLE --duckdb-version 1.5.5");
    // Legal, but almost certainly the C API version pasted by mistake.
    let args = parse_ok("in out --abi-type C_STRUCT_UNSTABLE --duckdb-version v1.2.0");
    assert_eq!(args.warnings.len(), 1, "{:?}", args.warnings);
}

#[test]
fn platform_is_validated() {
    parse_ok("in out --platform osx_arm64");
    parse_ok("in out --platform wasm_eh --wasm");
    assert!(parse_err("in out --platform Linux_AMD64").contains("--platform"));
    // Syntactically fine but not a community platform: accepted, with a warning.
    let args = parse_ok("in out --platform freebsd_amd64");
    assert_eq!(args.warnings.len(), 1, "{:?}", args.warnings);
    // A name only older DuckDB reported: legal, but not built any more.
    let args = parse_ok("in out --platform windows_amd64_rtools");
    assert_eq!(args.warnings.len(), 1, "{:?}", args.warnings);
}

/// `--platform linux` stamped silently, and `DuckDB` 1.5.5 then refused the
/// file ("built for the platform 'linux', but we can only load extensions
/// built for platform `linux_amd64`"): group names are refused now.
#[test]
fn a_platform_group_is_refused() {
    for group in ["linux", "osx", "wasm", "windows"] {
        let err = parse_err(&format!("in out --platform {group} --wasm"));
        assert!(err.contains("platform group"), "{group}: {err}");
    }
}

/// `--platform wasm_eh` without `--wasm` produced a module that
/// `WebAssembly.validate` rejects; it is refused now. `--wasm` on another
/// platform is legal but warned about.
#[test]
fn a_wasm_platform_needs_the_wasm_section_header() {
    for platform in ["wasm_eh", "wasm_mvp", "wasm_threads"] {
        let err = parse_err(&format!("in out --platform {platform}"));
        assert!(err.contains("pass --wasm"), "{platform}: {err}");
        assert_eq!(
            parse_ok(&format!("in out --platform {platform} --wasm")).warnings,
            Vec::<String>::new(),
            "{platform}"
        );
    }
    let args = parse_ok("in out --platform linux_amd64 --wasm");
    assert_eq!(args.warnings.len(), 1, "{:?}", args.warnings);
    assert!(
        args.warnings[0].contains("not a wasm platform"),
        "{:?}",
        args.warnings
    );
}

#[test]
fn help_is_a_command_not_an_exit() {
    assert!(matches!(cli::parse(&argv("--help")), Ok(Command::Help)));
    assert!(cli::help("append_metadata").contains("--wasm"));
}

// ── stamping ─────────────────────────────────────────────────────────────

#[test]
fn stamping_appends_exactly_one_footer() {
    let args = parse_ok("in out --platform linux_amd64");
    let (out, footer) = stamp(vec![1, 2, 3], &args).unwrap();
    assert_eq!(out.len(), 3 + METADATA_SIZE);
    assert_eq!(&out[3..], &footer[..]);
    assert_eq!(field_text(&footer, 6), "linux_amd64");
}

/// Re-stamping used to append a second footer after the first, silently.
#[test]
fn restamping_is_refused_unless_replace_is_given() {
    let first = parse_ok("in out --platform linux_amd64 --extension-version v1");
    let (once, _) = stamp(vec![7; 100], &first).unwrap();

    let err = stamp(once.clone(), &first).unwrap_err();
    assert!(err.contains("--replace"), "{err}");

    let second = parse_ok("in out --platform linux_arm64 --extension-version v2 --replace");
    let (twice, footer) = stamp(once, &second).unwrap();
    assert_eq!(
        twice.len(),
        100 + METADATA_SIZE,
        "the old footer must be removed"
    );
    assert_eq!(&twice[..100], &[7; 100][..]);
    assert_eq!(field_text(&footer, 4), "v2");
    assert_eq!(field_text(&footer, 6), "linux_arm64");
}

/// A footer whose ABI field is empty — which `DuckDB` reads as `CPP`, and
/// which `append_extension_metadata.py --abi-type ""` writes — was not
/// recognised, so re-stamping it appended a second footer. It is now refused
/// without `--replace`, and replaced with it.
#[test]
fn a_footer_with_an_empty_abi_field_is_recognised() {
    let mut data = vec![7; 100];
    data.extend_from_slice(&build_metadata("", "v1", "v1.4.0", "linux_amd64").unwrap());
    let args = parse_ok("in out --platform linux_amd64");
    let err = stamp(data.clone(), &args).unwrap_err();
    assert!(err.contains("--replace"), "{err}");
    let replace = parse_ok("in out --platform linux_amd64 --replace");
    let (out, _) = stamp(data, &replace).unwrap();
    assert_eq!(out.len(), 100 + METADATA_SIZE);
}

/// `--wasm` writes the header `append_extension_metadata.py` writes, and
/// `--replace` removes it along with the footer.
#[test]
fn wasm_header_matches_extension_ci_tools() {
    let mut expected = vec![0x00, 0x93, 0x04, 0x10];
    expected.extend_from_slice(b"duckdb_signature");
    expected.extend_from_slice(&[0x80, 0x04]);
    assert_eq!(WASM_SECTION_HEADER.to_vec(), expected);
    // LEB128 0x93 0x04 = 531 = 1 + 16 + 2 + 512; 0x80 0x04 = 512.
    assert_eq!((0x93 & 0x7f) + (0x04 << 7), 1 + 16 + 2 + METADATA_SIZE);
    assert_eq!(0x04 << 7, METADATA_SIZE); // 0x80 carries no payload bits

    let wasm = parse_ok("in out --platform wasm_eh --wasm");
    let (out, _) = stamp(vec![0; 8], &wasm).unwrap();
    assert_eq!(out.len(), 8 + 22 + METADATA_SIZE);
    assert_eq!(&out[8..30], &WASM_SECTION_HEADER);

    let again = parse_ok("in out --platform wasm_eh --wasm --replace");
    let (out, _) = stamp(out, &again).unwrap();
    assert_eq!(
        out.len(),
        8 + 22 + METADATA_SIZE,
        "header and footer replaced, not stacked"
    );
}
