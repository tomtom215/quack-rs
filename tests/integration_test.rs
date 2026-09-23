// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Integration tests for `quack-rs`.
//!
//! # Why no `duckdb::Connection` here?
//!
//! This crate's `libduckdb-sys` dependency is built with `features = ["loadable-extension"]`,
//! which makes every `DuckDB` C API call go through lazy `AtomicPtr` dispatch. These
//! pointers are only initialized when `duckdb_rs_extension_api_init` is called from
//! within a real `DuckDB` extension load event. As a result, `Connection::open_in_memory()`
//! panics with "`DuckDB` API not initialized" in the test process.
//!
//! Tests that exercise a real `DuckDB` connection live in `examples/hello-ext/src/lib.rs`
//! and are exercised by loading the compiled extension into a live `DuckDB` process.
//!
//! All tests here are pure-Rust and do not require a `DuckDB` runtime.

use quack_rs::aggregate::AggregateState;
use quack_rs::interval::{interval_to_micros, interval_to_micros_saturating, DuckInterval};
use quack_rs::sql_macro::{MacroBody, SqlMacro};
use quack_rs::types::TypeId;

// ---------------------------------------------------------------------------
// interval tests
// ---------------------------------------------------------------------------

#[test]
fn interval_zero_is_zero_micros() {
    let iv = DuckInterval {
        months: 0,
        days: 0,
        micros: 0,
    };
    assert_eq!(interval_to_micros(iv), Some(0));
    assert_eq!(interval_to_micros_saturating(iv), 0);
}

#[test]
fn interval_one_day_in_micros() {
    let iv = DuckInterval {
        months: 0,
        days: 1,
        micros: 0,
    };
    // 1 day = 86_400 seconds = 86_400_000_000 microseconds
    assert_eq!(interval_to_micros(iv), Some(86_400_000_000));
}

#[test]
fn interval_one_month_in_micros() {
    let iv = DuckInterval {
        months: 1,
        days: 0,
        micros: 0,
    };
    // 1 month = 30 days × 86_400_000_000 µs/day
    assert_eq!(interval_to_micros(iv), Some(30 * 86_400_000_000_i64));
}

#[test]
fn interval_combined_fields() {
    let iv = DuckInterval {
        months: 1,
        days: 1,
        micros: 1_000_000,
    };
    // 30 days + 1 day + 1 second
    let expected = 31 * 86_400_000_000_i64 + 1_000_000;
    assert_eq!(interval_to_micros(iv), Some(expected));
}

#[test]
fn interval_overflow_returns_none() {
    let iv = DuckInterval {
        months: i32::MAX,
        days: i32::MAX,
        micros: i64::MAX,
    };
    assert_eq!(interval_to_micros(iv), None);
}

#[test]
fn interval_saturating_overflow_returns_max() {
    let iv = DuckInterval {
        months: i32::MAX,
        days: i32::MAX,
        micros: i64::MAX,
    };
    assert_eq!(interval_to_micros_saturating(iv), i64::MAX);
}

#[test]
fn interval_saturating_underflow_returns_min() {
    let iv = DuckInterval {
        months: i32::MIN,
        days: i32::MIN,
        micros: i64::MIN,
    };
    assert_eq!(interval_to_micros_saturating(iv), i64::MIN);
}

#[test]
fn interval_negative_values() {
    let iv = DuckInterval {
        months: -1,
        days: -1,
        micros: -1,
    };
    // -30 days - 1 day - 1 µs
    let expected = -31 * 86_400_000_000_i64 - 1;
    assert_eq!(interval_to_micros(iv), Some(expected));
}

// ---------------------------------------------------------------------------
// TypeId round-trip and display tests
// ---------------------------------------------------------------------------

#[test]
fn type_id_enum_sql_names_are_correct() {
    assert_eq!(TypeId::Boolean.sql_name(), "BOOLEAN");
    assert_eq!(TypeId::TinyInt.sql_name(), "TINYINT");
    assert_eq!(TypeId::SmallInt.sql_name(), "SMALLINT");
    assert_eq!(TypeId::Integer.sql_name(), "INTEGER");
    assert_eq!(TypeId::BigInt.sql_name(), "BIGINT");
    assert_eq!(TypeId::UTinyInt.sql_name(), "UTINYINT");
    assert_eq!(TypeId::USmallInt.sql_name(), "USMALLINT");
    assert_eq!(TypeId::UInteger.sql_name(), "UINTEGER");
    assert_eq!(TypeId::UBigInt.sql_name(), "UBIGINT");
    assert_eq!(TypeId::Float.sql_name(), "FLOAT");
    assert_eq!(TypeId::Double.sql_name(), "DOUBLE");
    assert_eq!(TypeId::Timestamp.sql_name(), "TIMESTAMP");
    assert_eq!(TypeId::TimestampTz.sql_name(), "TIMESTAMPTZ");
    assert_eq!(TypeId::Date.sql_name(), "DATE");
    assert_eq!(TypeId::Time.sql_name(), "TIME");
    assert_eq!(TypeId::Interval.sql_name(), "INTERVAL");
    assert_eq!(TypeId::Varchar.sql_name(), "VARCHAR");
    assert_eq!(TypeId::Blob.sql_name(), "BLOB");
    assert_eq!(TypeId::Uuid.sql_name(), "UUID");
}

#[test]
fn type_id_display_matches_sql_name() {
    use std::fmt::Write as _;
    let types = [
        TypeId::Boolean,
        TypeId::BigInt,
        TypeId::Varchar,
        TypeId::Timestamp,
        TypeId::Interval,
    ];
    for t in types {
        let mut s = String::new();
        write!(s, "{t}").unwrap();
        assert_eq!(s, t.sql_name());
    }
}

#[test]
fn type_id_copy_and_eq() {
    let a = TypeId::BigInt;
    let b = a; // Copy
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// AggregateState / FfiState lifecycle tests
// ---------------------------------------------------------------------------

#[derive(Default, Debug, PartialEq, Clone)]
struct SumState {
    total: i64,
}
impl AggregateState for SumState {}

#[derive(Default, Debug, PartialEq, Clone)]
struct RetentionState {
    /// Configuration: number of condition columns (must be propagated in combine)
    n_conditions: usize,
    /// Counts per condition
    counts: [u64; 32],
}
impl AggregateState for RetentionState {}

#[test]
fn ffi_state_size_matches_pointer() {
    use quack_rs::aggregate::FfiState;

    #[derive(Default)]
    struct TestState {
        _value: i64,
    }
    impl AggregateState for TestState {}

    assert_eq!(
        FfiState::<TestState>::size(),
        std::mem::size_of::<*mut TestState>()
    );
}

// ---------------------------------------------------------------------------
// AggregateTestHarness tests
// ---------------------------------------------------------------------------

#[test]
fn harness_sum_correctness() {
    use quack_rs::testing::AggregateTestHarness;

    let result =
        AggregateTestHarness::<SumState>::aggregate([10_i64, 20, 30, 40, 50], |s, v| s.total += v);
    assert_eq!(result.total, 150);
}

#[test]
fn harness_empty_aggregate_is_default() {
    use quack_rs::testing::AggregateTestHarness;

    let result = AggregateTestHarness::<SumState>::aggregate(std::iter::empty::<i64>(), |s, v| {
        s.total += v;
    });
    assert_eq!(result.total, 0);
}

#[test]
fn harness_combine_propagates_config() {
    use quack_rs::testing::AggregateTestHarness;

    // Simulate DuckDB's segment-tree combine pattern:
    // DuckDB creates a new zero-initialized target state, then calls combine(source → target).
    // If combine doesn't copy the config field (n_conditions), the target retains 0 — Pitfall L1.
    let mut source = AggregateTestHarness::<RetentionState>::new();
    source.update(|s| {
        s.n_conditions = 3;
        s.counts[0] += 100;
        s.counts[1] += 50;
        s.counts[2] += 25;
    });

    // Target is fresh (zero-initialized), simulating DuckDB's behavior
    let mut target = AggregateTestHarness::<RetentionState>::new();

    // Correct combine: propagate ALL fields including config
    target.combine(&source, |src, tgt| {
        tgt.n_conditions = src.n_conditions; // critical: must copy config
        for i in 0..src.n_conditions {
            tgt.counts[i] += src.counts[i];
        }
    });

    let result = target.finalize();
    assert_eq!(result.n_conditions, 3, "config field must be propagated");
    assert_eq!(result.counts[0], 100);
    assert_eq!(result.counts[1], 50);
    assert_eq!(result.counts[2], 25);
}

#[test]
fn harness_combine_bug_demo_missing_config() {
    use quack_rs::testing::AggregateTestHarness;

    // This test demonstrates WHAT GOES WRONG without config propagation (Pitfall L1)
    let mut source = AggregateTestHarness::<RetentionState>::new();
    source.update(|s| {
        s.n_conditions = 3;
        s.counts[0] += 100;
    });

    let mut target = AggregateTestHarness::<RetentionState>::new();
    // BUG: only merge counts, forget to copy n_conditions
    target.combine(&source, |src, tgt| {
        for i in 0..32 {
            tgt.counts[i] += src.counts[i];
        }
        // FORGOT: tgt.n_conditions = src.n_conditions;
    });

    let result = target.finalize();
    assert_eq!(result.n_conditions, 0, "this is the bug: config is lost");
    assert_eq!(result.counts[0], 100, "data is preserved");
}

// ---------------------------------------------------------------------------
// ExtensionError integration tests
// ---------------------------------------------------------------------------

#[test]
fn extension_error_message_preserved() {
    use quack_rs::error::ExtensionError;

    let err = ExtensionError::new("something went wrong");
    assert_eq!(err.as_str(), "something went wrong");
    assert!(err.to_string().contains("something went wrong"));
}

#[test]
fn extension_error_from_string() {
    use quack_rs::error::ExtensionError;

    let err: ExtensionError = "test error".into();
    assert_eq!(err.as_str(), "test error");
}

#[test]
fn extension_error_to_c_string_no_null() {
    use quack_rs::error::ExtensionError;

    let err = ExtensionError::new("no null here");
    let c = err.to_c_string();
    assert_eq!(c.to_str().unwrap(), "no null here");
}

#[test]
fn extension_error_keeps_the_text_after_a_null_byte() {
    use quack_rs::error::ExtensionError;

    let err = ExtensionError::new("before\0after");
    let c = err.to_c_string();
    assert_eq!(c.to_str().unwrap(), "before?after");
}

#[test]
fn extension_error_implements_std_error() {
    use quack_rs::error::ExtensionError;
    use std::error::Error;

    let err = ExtensionError::new("std error test");
    // Verify it works as a Box<dyn std::error::Error>
    let boxed: Box<dyn Error> = Box::new(err.clone());
    assert!(boxed.to_string().contains("std error test"));
    // Verify source() is None (no cause chain)
    assert!(err.source().is_none());
}

#[test]
fn extension_error_question_mark_operator() {
    use quack_rs::error::ExtensionError;

    fn fallible(fail: bool) -> Result<i32, ExtensionError> {
        if fail {
            return Err(ExtensionError::new("forced failure"));
        }
        Ok(42)
    }

    assert_eq!(fallible(false).unwrap(), 42);
    assert_eq!(fallible(true).unwrap_err().as_str(), "forced failure");
}

// ---------------------------------------------------------------------------
// VectorReader / VectorWriter pure-Rust logic tests
// ---------------------------------------------------------------------------

#[test]
fn vector_reader_validity_null_means_all_valid() {
    // When validity pointer is null, is_valid should return true for any row.
    // This tests the underlying logic: a null validity bitmap means all rows are valid.
    // We test the invariant directly: a null *mut u64 means no NULL values.
    let validity_ptr: *mut u64 = std::ptr::null_mut();
    assert!(validity_ptr.is_null(), "null validity means all rows valid");
}

#[test]
fn vector_reader_boolean_as_u8_pattern() {
    // Verify the u8 != 0 boolean reading pattern (Pitfall L5)
    // DuckDB may store non-0/1 bytes for booleans; we must compare != 0
    let test_bytes: &[u8] = &[0x00, 0x01, 0x02, 0xFF];
    let results: Vec<bool> = test_bytes.iter().map(|&b| b != 0).collect();
    assert_eq!(results, [false, true, true, true]);
}

#[test]
fn vector_writer_holds_vector_data_and_cached_validity() {
    use quack_rs::vector::VectorWriter;
    use std::mem::size_of;
    // vector handle + cached data pointer + cached validity pointer. The
    // validity pointer is what turns "two FFI calls per NULL" into "two per
    // vector"; see `VectorWriter::set_null`.
    assert_eq!(size_of::<VectorWriter>(), 3 * size_of::<usize>());
}

// ---------------------------------------------------------------------------
// DuckStringView pure-Rust tests
// ---------------------------------------------------------------------------

#[test]
fn duck_string_view_inline_format() {
    use quack_rs::vector::DuckStringView;

    // Build a 16-byte buffer for an inline string ("hello" = 5 bytes)
    // Layout: [len: u32 LE][data: 12 bytes padding to 0]
    let mut bytes = [0u8; 16];
    let s = b"hello";
    bytes[0..4].copy_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_le_bytes());
    bytes[4..4 + s.len()].copy_from_slice(s);

    let view = DuckStringView::inline_from_bytes(&bytes).expect("inline value");
    assert_eq!(view.len(), 5);
    assert!(!view.is_empty());
    assert_eq!(view.as_str(), Some("hello"));
}

#[test]
fn duck_string_view_empty_string() {
    use quack_rs::vector::DuckStringView;

    let bytes = [0u8; 16]; // len = 0
    let view = DuckStringView::inline_from_bytes(&bytes).expect("inline value");
    assert_eq!(view.len(), 0);
    assert!(view.is_empty());
}

// ---------------------------------------------------------------------------
// SqlMacro pure-Rust tests (no DuckDB connection required)
// ---------------------------------------------------------------------------

#[test]
fn sql_macro_scalar_to_sql_no_params() {
    let m = SqlMacro::scalar("pi", &[], "3.14159265358979").unwrap();
    assert_eq!(
        m.to_sql(),
        r#"CREATE OR REPLACE MACRO "pi"() AS (3.14159265358979)"#
    );
}

#[test]
fn sql_macro_scalar_to_sql_multiple_params() {
    let m = SqlMacro::scalar("add", &["a", "b"], "a + b").unwrap();
    assert_eq!(
        m.to_sql(),
        r#"CREATE OR REPLACE MACRO "add"("a", "b") AS (a + b)"#
    );
}

#[test]
fn sql_macro_scalar_clamp_to_sql() {
    let m = SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))").unwrap();
    assert_eq!(
        m.to_sql(),
        r#"CREATE OR REPLACE MACRO "clamp"("x", "lo", "hi") AS (greatest(lo, least(hi, x)))"#
    );
}

#[test]
fn sql_macro_table_to_sql() {
    let m = SqlMacro::table(
        "active_rows",
        &["tbl"],
        "SELECT * FROM tbl WHERE active = true",
    )
    .unwrap();
    assert_eq!(
        m.to_sql(),
        r#"CREATE OR REPLACE MACRO "active_rows"("tbl") AS TABLE SELECT * FROM tbl WHERE active = true"#
    );
}

#[test]
fn sql_macro_invalid_name_rejected() {
    assert!(SqlMacro::scalar("my-macro", &[], "1").is_err());
    assert!(SqlMacro::scalar("", &[], "1").is_err());
    assert!(SqlMacro::scalar("1func", &[], "1").is_err());
    assert!(SqlMacro::scalar("my macro", &[], "1").is_err());
    // Mixed case is legal — DuckDB ships `formatReadableSize`.
    assert!(SqlMacro::scalar("MyMacro", &[], "1").is_ok());
}

#[test]
fn sql_macro_invalid_param_rejected() {
    assert!(SqlMacro::scalar("f", &["a-b"], "1").is_err());
    assert!(SqlMacro::scalar("f", &[""], "1").is_err());
    assert!(SqlMacro::scalar("f", &["a b"], "1").is_err());
    assert!(SqlMacro::scalar("f", &["MixedCase"], "1").is_ok());
}

#[test]
fn sql_macro_valid_underscore_param() {
    assert!(SqlMacro::scalar("f", &["_x", "_y"], "1").is_ok());
}

#[test]
fn sql_macro_name_and_params_accessors() {
    let m = SqlMacro::scalar("my_fn", &["a", "b"], "a + b").unwrap();
    assert_eq!(m.name(), "my_fn");
    assert_eq!(m.params(), ["a", "b"]);
}

#[test]
fn sql_macro_body_accessor_scalar() {
    let m = SqlMacro::scalar("f", &["x"], "x * 2").unwrap();
    assert_eq!(m.body(), &MacroBody::Scalar("x * 2".to_string()));
}

#[test]
fn sql_macro_body_accessor_table() {
    let m = SqlMacro::table("t", &[], "SELECT 42 AS answer").unwrap();
    assert_eq!(
        m.body(),
        &MacroBody::Table("SELECT 42 AS answer".to_string())
    );
}

#[test]
fn sql_macro_clone_produces_equal_sql() {
    let m = SqlMacro::scalar("f", &["x"], "x + 1").unwrap();
    let m2 = m.clone();
    assert_eq!(m.to_sql(), m2.to_sql());
}

#[test]
fn sql_macro_error_mentions_bad_param_name() {
    let err = SqlMacro::scalar("f", &["bad param"], "1").unwrap_err();
    assert!(err.as_str().contains("bad param"));
}

// ---------------------------------------------------------------------------
// TEST-2: Scaffold generated code compiles successfully
// ---------------------------------------------------------------------------

/// Generates scaffold files, writes them to a temp directory with a path-dep
/// on the local quack-rs crate, and runs the generated project's own
/// `cargo test --lib`: the code must compile, and the unit test the scaffold
/// ships must run and pass. This catches template regressions (like broken
/// macro paths) that unit tests can't detect — and a scaffold whose tests are
/// vacuous (it used to generate zero).
#[test]
fn scaffold_generated_code_compiles() {
    use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};
    use std::fs;
    use std::process::Command;

    let config = ScaffoldConfig {
        name: "test_ext".to_string(),
        // Deliberately awkward: a multi-line description used to leave every
        // line after the first outside the `//!` comment, where it failed to
        // compile. Quotes, a backslash and non-ASCII text ride along.
        description: "Scaffold compile test: \"quoted\", C:\\path \u{2014} caf\u{e9}\n\
                      second line # not a comment"
            .to_string(),
        version: "0.1.0".to_string(),
        license: "MIT".to_string(),
        maintainer: "CI".to_string(),
        github_repo: "test/test-ext".to_string(),
        excluded_platforms: vec![],
        ..ScaffoldConfig::default()
    };

    let files = generate_scaffold(&config).unwrap();

    // Write to a unique temp directory to avoid race conditions when running
    // multiple test suites concurrently (e.g. with and without feature flags).
    let tmp = tempfile::Builder::new()
        .prefix("quack_rs_scaffold_compile_test_")
        .tempdir()
        .unwrap();
    let tmp = tmp.path().to_path_buf();
    fs::create_dir_all(tmp.join("src")).unwrap();

    // The scaffold's Cargo.toml depends on quack-rs from crates.io, at whatever
    // version this crate currently is. Repoint it at this working copy so
    // `cargo check` tests the code in front of us — and so a version bump does
    // not fail this test in the window before that version is published.
    //
    // This was previously a `.replace()` of the literal `version = "0.13"`,
    // which stopped matching the moment the crate moved past 0.13. It then
    // silently did nothing, and the test spent several releases checking the
    // last *published* quack-rs instead of the working copy. The assertion
    // below makes that failure mode impossible.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path_dep = format!(
        "quack-rs = {{ path = \"{}\" }}",
        workspace_root.display().to_string().replace('\\', "/")
    );

    for f in &files {
        let dest = tmp.join(&f.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        if f.path == "Cargo.toml" {
            let patched: String = f
                .content
                .lines()
                .map(|line| {
                    if line.trim_start().starts_with("quack-rs = {") {
                        path_dep.as_str()
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                patched.contains(&path_dep),
                "the quack-rs dependency line was not rewritten to a path dep; \
                 this test would have checked the published crate instead of \
                 this working copy"
            );
            fs::write(&dest, patched).unwrap();
        } else {
            fs::write(&dest, &f.content).unwrap();
        }
    }

    // Build and run the generated project's unit tests.
    let output = Command::new("cargo")
        .args(["test", "--lib"])
        .current_dir(&tmp)
        .output()
        .expect("failed to run cargo test");

    // Clean up before asserting so we don't leave temp dirs on failure
    let _ = fs::remove_dir_all(&tmp);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "Scaffold-generated code failed to compile or its tests failed!\nstdout:\n{}\nstderr:\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr),
    );
    let passed: usize = stdout
        .lines()
        .find_map(|line| line.strip_prefix("test result: ok. "))
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    assert!(
        passed > 0,
        "the generated project's `cargo test` ran no tests:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// MockVectorWriter / MockVectorReader tests
// ---------------------------------------------------------------------------

#[test]
fn mock_vector_writer_basic_write_and_read() {
    use quack_rs::testing::{MockDuckValue, MockVectorWriter};

    let mut w = MockVectorWriter::new(4);
    w.write_i64(0, 42);
    w.write_i64(1, -7);
    w.set_null(2);
    w.write_varchar(3, "hello");

    assert_eq!(w.try_get_i64(0), Some(42));
    assert_eq!(w.try_get_i64(1), Some(-7));
    assert!(w.is_null(2));
    assert_eq!(w.try_get_str(3), Some("hello"));
    // Wrong type returns None
    assert_eq!(w.try_get_i64(3), None);
    assert!(matches!(w.get(0), Some(MockDuckValue::I64(42))));
}

#[test]
fn mock_vector_writer_set_null_after_write() {
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(1);
    w.write_i64(0, 100);
    assert!(!w.is_null(0));
    w.set_null(0);
    assert!(w.is_null(0));
    assert_eq!(w.try_get_i64(0), None);
}

/// A real `DuckDB` vector has a fixed capacity; writing past it is
/// out-of-bounds memory. The mock used to grow silently instead, so a test
/// could pass for a callback that overruns its output. It must refuse.
#[test]
#[should_panic(expected = "out of bounds for a mock vector of capacity 0")]
fn mock_vector_writer_refuses_to_grow_beyond_its_capacity() {
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(0);
    w.write_i64(3, 99);
}

#[test]
fn mock_vector_writer_boolean_and_interval() {
    use quack_rs::interval::DuckInterval;
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(2);
    w.write_bool(0, true);
    let iv = DuckInterval {
        months: 1,
        days: 2,
        micros: 3,
    };
    w.write_interval(1, iv);

    assert_eq!(w.try_get_bool(0), Some(true));
    assert_eq!(w.try_get_interval(1), Some(iv));
}

#[test]
fn mock_vector_reader_from_i64s() {
    use quack_rs::testing::MockVectorReader;

    let r = MockVectorReader::from_i64s([Some(10), None, Some(-5)]);
    assert_eq!(r.row_count(), 3);
    assert!(r.is_valid(0));
    assert!(!r.is_valid(1));
    assert!(r.is_valid(2));
    assert_eq!(r.try_get_i64(0), Some(10));
    assert_eq!(r.try_get_i64(1), None);
    assert_eq!(r.try_get_i64(2), Some(-5));
}

#[test]
fn mock_vector_reader_from_strs() {
    use quack_rs::testing::MockVectorReader;

    let r = MockVectorReader::from_strs([Some("alpha"), None, Some("beta")]);
    assert_eq!(r.try_get_str(0), Some("alpha"));
    assert_eq!(r.try_get_str(1), None);
    assert_eq!(r.try_get_str(2), Some("beta"));
    assert!(!r.is_valid(1));
}

#[test]
fn mock_vector_reader_out_of_bounds() {
    use quack_rs::testing::MockVectorReader;

    let r = MockVectorReader::from_i64s([Some(1)]);
    assert!(!r.is_valid(100));
    assert_eq!(r.try_get_i64(100), None);
}

#[test]
fn mock_vector_pattern_extract_and_test_logic() {
    // Demonstrates the recommended pattern: extract callback logic into a
    // pure-Rust function that can be called with MockVectorReader/Writer.
    use quack_rs::testing::{MockVectorReader, MockVectorWriter};

    fn clamp_values(reader: &MockVectorReader, writer: &mut MockVectorWriter, lo: i64, hi: i64) {
        for i in 0..reader.row_count() {
            if reader.is_valid(i) {
                let v = reader.try_get_i64(i).unwrap_or(0);
                writer.write_i64(i, v.clamp(lo, hi));
            } else {
                writer.set_null(i);
            }
        }
    }

    let reader = MockVectorReader::from_i64s([Some(-5), Some(3), None, Some(15)]);
    let mut writer = MockVectorWriter::new(4);
    clamp_values(&reader, &mut writer, 0, 10);

    assert_eq!(writer.try_get_i64(0), Some(0));
    assert_eq!(writer.try_get_i64(1), Some(3));
    assert!(writer.is_null(2));
    assert_eq!(writer.try_get_i64(3), Some(10));
}

#[test]
fn mock_vector_writer_u32_round_trip() {
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(1);
    w.write_u32(0, 123_456);
    assert!(matches!(
        w.get(0),
        Some(quack_rs::testing::MockDuckValue::U32(123_456))
    ));
}

#[test]
fn mock_vector_writer_u64_round_trip() {
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(1);
    w.write_u64(0, 9_876_543_210);
    assert!(matches!(
        w.get(0),
        Some(quack_rs::testing::MockDuckValue::U64(9_876_543_210))
    ));
}

#[test]
fn mock_vector_writer_f32_round_trip() {
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(1);
    w.write_f32(0, 2.5);
    match w.get(0) {
        Some(quack_rs::testing::MockDuckValue::F32(v)) => {
            assert!((v - 2.5_f32).abs() < f32::EPSILON);
        }
        other => panic!("expected F32, got {other:?}"),
    }
}

#[test]
fn mock_vector_writer_write_str_alias() {
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(1);
    w.write_str(0, "via_alias");
    assert_eq!(w.try_get_str(0), Some("via_alias"));
}

// ---------------------------------------------------------------------------
// MockRegistrar tests
// ---------------------------------------------------------------------------

#[test]
fn mock_registrar_records_scalar_function() {
    use quack_rs::connection::Registrar;
    use quack_rs::scalar::ScalarFunctionBuilder;
    use quack_rs::testing::MockRegistrar;
    use quack_rs::types::TypeId;

    let mock = MockRegistrar::new();
    let b = ScalarFunctionBuilder::new("word_count")
        .param(TypeId::Varchar)
        .returns(TypeId::BigInt);
    unsafe { mock.register_scalar(b).unwrap() };

    assert!(mock.has_scalar("word_count"));
    assert_eq!(mock.scalar_names(), vec!["word_count"]);
    assert_eq!(mock.total_registrations(), 1);
}

#[test]
fn mock_registrar_records_aggregate_function() {
    use quack_rs::aggregate::AggregateFunctionBuilder;
    use quack_rs::connection::Registrar;
    use quack_rs::testing::MockRegistrar;
    use quack_rs::types::TypeId;

    let mock = MockRegistrar::new();
    let b = AggregateFunctionBuilder::new("my_agg")
        .param(TypeId::BigInt)
        .returns(TypeId::BigInt);
    unsafe { mock.register_aggregate(b).unwrap() };

    assert!(mock.has_aggregate("my_agg"));
    assert_eq!(mock.total_registrations(), 1);
}

#[test]
fn mock_registrar_records_table_function() {
    use quack_rs::connection::Registrar;
    use quack_rs::table::TableFunctionBuilder;
    use quack_rs::testing::MockRegistrar;

    let mock = MockRegistrar::new();
    let b = TableFunctionBuilder::new("my_table_fn");
    unsafe { mock.register_table(b).unwrap() };

    assert!(mock.has_table("my_table_fn"));
    assert_eq!(mock.total_registrations(), 1);
}

#[test]
fn mock_registrar_records_sql_macro() {
    use quack_rs::connection::Registrar;
    use quack_rs::sql_macro::SqlMacro;
    use quack_rs::testing::MockRegistrar;

    let mock = MockRegistrar::new();
    let m = SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))").unwrap();
    unsafe { mock.register_sql_macro(m).unwrap() };

    assert!(mock.has_sql_macro("clamp"));
    assert_eq!(mock.total_registrations(), 1);
}

#[test]
fn mock_registrar_records_cast() {
    use quack_rs::cast::CastFunctionBuilder;
    use quack_rs::connection::Registrar;
    use quack_rs::testing::{CastRecord, MockRegistrar};
    use quack_rs::types::TypeId;

    let mock = MockRegistrar::new();
    let b = CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer);
    unsafe { mock.register_cast(b).unwrap() };

    let casts = mock.casts();
    assert_eq!(casts.len(), 1);
    assert_eq!(
        casts[0],
        CastRecord {
            source: Some(TypeId::Varchar),
            target: Some(TypeId::Integer),
        }
    );
}

#[test]
fn mock_registrar_used_as_generic_registrar() {
    // Demonstrates the core use case: passing MockRegistrar where &impl Registrar
    // is expected, so registration functions can be unit-tested.
    use quack_rs::connection::Registrar;
    use quack_rs::error::ExtensionError;
    use quack_rs::scalar::ScalarFunctionBuilder;
    use quack_rs::sql_macro::SqlMacro;
    use quack_rs::testing::MockRegistrar;
    use quack_rs::types::TypeId;

    fn register_all(reg: &impl Registrar) -> Result<(), ExtensionError> {
        let upper = ScalarFunctionBuilder::new("upper_ext")
            .param(TypeId::Varchar)
            .returns(TypeId::Varchar);
        let m = SqlMacro::scalar("pi", &[], "3.14159265358979").unwrap();
        unsafe {
            reg.register_scalar(upper)?;
            reg.register_sql_macro(m)?;
        }
        Ok(())
    }

    let mock = MockRegistrar::new();
    register_all(&mock).unwrap();

    assert_eq!(mock.total_registrations(), 2);
    assert!(mock.has_scalar("upper_ext"));
    assert!(mock.has_sql_macro("pi"));
    assert!(!mock.has_scalar("pi")); // pi is a macro, not a scalar
}

// ---------------------------------------------------------------------------
// Builder name() accessor tests
// ---------------------------------------------------------------------------

#[test]
fn scalar_builder_name_accessor() {
    use quack_rs::scalar::ScalarFunctionBuilder;
    use quack_rs::types::TypeId;

    let b = ScalarFunctionBuilder::new("my_scalar").returns(TypeId::BigInt);
    assert_eq!(b.name(), "my_scalar");
}

#[test]
fn aggregate_builder_name_accessor() {
    use quack_rs::aggregate::AggregateFunctionBuilder;
    use quack_rs::types::TypeId;

    let b = AggregateFunctionBuilder::new("my_agg").returns(TypeId::BigInt);
    assert_eq!(b.name(), "my_agg");
}

#[test]
fn table_builder_name_accessor() {
    use quack_rs::table::TableFunctionBuilder;

    let b = TableFunctionBuilder::new("my_table");
    assert_eq!(b.name(), "my_table");
}

#[test]
fn typed_table_function_builds_through_mock_registrar() {
    use quack_rs::connection::Registrar;
    use quack_rs::table::TableFunctionBuilder;
    use quack_rs::testing::MockRegistrar;

    #[derive(Clone)]
    struct State {
        #[allow(dead_code)]
        remaining: u64,
    }

    let typed = TableFunctionBuilder::new("count_down")
        .param(TypeId::BigInt)
        .with_state::<State, _>(|_bind| Ok(State { remaining: 3 }))
        .scan(|_state, _chunk| Ok(()));

    assert_eq!(typed.name(), "count_down");

    let builder = typed.build().expect("build succeeds");
    let mock = MockRegistrar::new();
    unsafe { mock.register_table(builder).unwrap() };
    assert!(mock.has_table("count_down"));
}

#[test]
fn typed_table_function_requires_scan_closure() {
    use quack_rs::table::TableFunctionBuilder;

    #[derive(Clone)]
    struct State;
    let typed = TableFunctionBuilder::new("needs_scan").with_state::<State, _>(|_bind| Ok(State));

    match typed.build() {
        Err(e) => assert!(e.as_str().contains("scan closure not set")),
        Ok(_) => panic!("expected an error when scan is missing"),
    }
}

#[test]
fn cast_builder_source_target_accessors() {
    use quack_rs::cast::CastFunctionBuilder;
    use quack_rs::types::TypeId;

    let b = CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer);
    assert_eq!(b.source(), Some(TypeId::Varchar));
    assert_eq!(b.target(), Some(TypeId::Integer));
}

// ─── The crate's own artefacts obey its own validators ───────────────────────

/// Extracts a `key = value` from a `[profile.release]` section, ignoring
/// comments and any other section.
fn release_profile_value(manifest: &str, key: &str) -> Option<String> {
    let mut in_profile = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_profile = line == "[profile.release]";
            continue;
        }
        if !in_profile || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() == key {
            return Some(v.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// `validate_release_profile` rejects `panic = "abort"` because it makes every
/// `catch_unwind` in quack-rs inert — a panic then aborts the whole `DuckDB`
/// process instead of becoming a SQL error.
///
/// The reference example shipped with `panic = "abort"` regardless, so CI built
/// it, loaded it into `DuckDB`, and held it up as the way to do this, while every
/// panic guarantee in it was switched off. Nothing caught the contradiction
/// because nothing pointed the validator at the crate's own files.
#[test]
fn the_example_obeys_the_crates_own_release_profile_rules() {
    use quack_rs::validate::validate_release_profile;

    // Read at run time, not with `include_str!`: `examples/` is excluded from
    // the published package, so a compile-time include made `cargo test` of
    // the downloaded crate fail to build at all. The skip is taken only when
    // the whole directory is missing — in the repository, a moved or renamed
    // manifest is still a failure.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join("examples").is_dir() {
        eprintln!(
            "SKIPPED the_example_obeys_the_crates_own_release_profile_rules: examples/ is not \
             part of the packaged crate"
        );
        return;
    }
    let path = root.join("examples/hello-ext/Cargo.toml");
    let manifest =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let manifest = manifest.as_str();
    let panic = release_profile_value(manifest, "panic")
        .expect("the example must state its panic strategy explicitly");
    let lto = release_profile_value(manifest, "lto").unwrap_or_default();
    let opt = release_profile_value(manifest, "opt-level").unwrap_or_default();
    let cgu = release_profile_value(manifest, "codegen-units").unwrap_or_default();

    let check = validate_release_profile(&panic, &lto, &opt, &cgu)
        .unwrap_or_else(|e| panic!("examples/hello-ext/Cargo.toml: {e}"));
    assert!(
        check.is_fully_optimized(),
        "the example is what people copy; it should also be the profile quack-rs \
         recommends for a shipped extension"
    );
}

/// The repository-structure trees in `CONTRIBUTING.md` and the book list every
/// source file under `src/` and `tests/`, and nothing that does not exist.
///
/// The trees were accurate when this test was written except for the one file
/// most recently added, which is how such a tree goes stale: nothing fails.
#[test]
fn documented_source_trees_match_the_filesystem() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join("book").is_dir() {
        eprintln!("SKIPPED documented_source_trees_match_the_filesystem: book/ is not packaged");
        return;
    }
    let mut actual = std::collections::BTreeSet::new();
    let mut dirs = vec![root.join("src"), root.join("tests")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs" || e == "cpp") {
                let rel = path.strip_prefix(root).expect("under root");
                actual.insert(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    for doc in ["CONTRIBUTING.md", "book/src/contributing.md"] {
        let text = std::fs::read_to_string(root.join(doc)).expect(doc);
        let block = text
            .split("```")
            .find(|b| b.contains("quack-rs/") && b.contains("├── src/"))
            .unwrap_or_else(|| panic!("{doc}: no repository tree found"));
        // Each level is four columns ("│   " or "    ") before "├── " / "└── ".
        let mut stack: Vec<String> = Vec::new();
        let mut listed = std::collections::BTreeSet::new();
        for line in block.lines() {
            let chars: Vec<char> = line.chars().collect();
            let Some(pos) = line.find("├── ").or_else(|| line.find("└── ")) else {
                continue;
            };
            let depth = line[..pos].chars().count() / 4;
            let name: String = chars[line[..pos].chars().count() + 4..]
                .iter()
                .take_while(|c| !c.is_whitespace())
                .collect();
            stack.truncate(depth);
            let full = format!("{}{name}", stack.concat());
            if name.ends_with('/') {
                stack.push(name);
            } else if full.starts_with("src/") || full.starts_with("tests/") {
                listed.insert(full);
            }
        }
        let unlisted: Vec<_> = actual.difference(&listed).collect();
        let phantom: Vec<_> = listed
            .difference(&actual)
            .filter(|p| {
                std::path::Path::new(p.as_str())
                    .extension()
                    .is_some_and(|e| e == "rs" || e == "cpp")
            })
            .collect();
        assert!(
            unlisted.is_empty() && phantom.is_empty(),
            "{doc}'s tree is stale.\nMissing from the tree: {unlisted:?}\nListed but absent: {phantom:?}"
        );
    }
}

/// Every "N pitfalls" claim in the documentation matches the number of
/// pitfalls `LESSONS.md` actually documents.
///
/// The count had drifted three ways at once — 21 in the crate docs and the
/// FAQ, 23 in the README and the book introduction — because each copy was
/// edited by hand.
#[test]
fn documented_pitfall_counts_match_lessons_md() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join("book").is_dir() {
        eprintln!("SKIPPED documented_pitfall_counts_match_lessons_md: book/ is not packaged");
        return;
    }
    let lessons = std::fs::read_to_string(root.join("LESSONS.md")).expect("LESSONS.md");
    let actual = lessons
        .lines()
        .filter(|l| {
            l.strip_prefix("## ")
                .and_then(|h| h.split_once(':'))
                .is_some_and(|(id, _)| {
                    id.len() >= 2
                        && matches!(id.as_bytes()[0], b'L' | b'P')
                        && id[1..].bytes().all(|b| b.is_ascii_digit())
                })
        })
        .count();
    assert!(
        actual > 0,
        "no `## L<n>:` / `## P<n>:` headings found in LESSONS.md"
    );

    let mut files = vec![root.join("README.md"), root.join("src/lib.rs")];
    let mut dirs = vec![root.join("book/src")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "md")
                && path.file_name().is_some_and(|n| n != "changelog.md")
            {
                files.push(path);
            }
        }
    }
    let mut wrong = Vec::new();
    for file in &files {
        let text =
            std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let words: Vec<&str> = text.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let digits = w.trim_matches(|c: char| !c.is_ascii_digit());
            let Ok(n) = digits.parse::<usize>() else {
                continue;
            };
            // "<n> ... pitfall(s)" within the next four words, e.g. "23 documented
            // FFI pitfalls", "all 21 known pitfalls", "23 pitfalls documented".
            // Historical counts ("revealed 16 undocumented pitfalls", "the
            // first 16 of the pitfalls") are about the past and stay as written.
            let window = &words[i + 1..words.len().min(i + 5)];
            let near = window.iter().any(|w| w.starts_with("pitfall"));
            let historical = window.iter().any(|w| w.contains("undocumented"))
                || i.checked_sub(1).is_some_and(|p| words[p] == "first");
            if near && !historical && n != actual {
                wrong.push(format!("{}: says {n}", file.display()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "LESSONS.md documents {actual} pitfalls, but:\n{}",
        wrong.join("\n")
    );
}

/// Every paragraph of user-facing documentation that mentions
/// `panic = "abort"` must be warning against it.
///
/// The Cargo manifests are held to `validate_release_profile` above, but the
/// docs are prose: the book once said `panic = "abort"` was *required*, and
/// after that was fixed the hello-ext README's checklist still told readers to
/// verify their profile "has `panic = "abort"`". This test fails on either.
/// A paragraph passes when it also contains one of the words below.
#[test]
fn documentation_never_recommends_panic_abort() {
    // Stems, matched as substrings — except "not", matched as a whole word
    // below so that "note" or "annotation" do not count as a warning.
    const NEGATIONS: &[&str] = &[
        "never",
        "reject",
        "inert",
        "disable",
        "terminat",
        "dies",
        "kill",
        "abort the",
        "aborts",
        "crash",
        "nothing can be caught",
        "cannot catch",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join("book").is_dir() {
        eprintln!("SKIPPED documentation_never_recommends_panic_abort: book/ is not packaged");
        return;
    }
    let mut files = vec![
        root.join("README.md"),
        root.join("LESSONS.md"),
        root.join("CONTRIBUTING.md"),
        root.join("SECURITY.md"),
        root.join("examples/hello-ext/README.md"),
    ];
    let mut dirs = vec![root.join("book/src"), root.join("docs")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "md")
                // The changelog mirror records what old releases said.
                && path.file_name().is_some_and(|n| n != "changelog.md")
            {
                files.push(path);
            }
        }
    }
    let mut offenders = Vec::new();
    for file in &files {
        let text =
            std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for paragraph in text.split("\n\n") {
            let lower = paragraph.to_lowercase();
            let warns = NEGATIONS.iter().any(|n| lower.contains(n))
                || lower
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|w| w == "not");
            if lower.contains("panic = \"abort\"") && !warns {
                offenders.push(format!("{}:\n{paragraph}", file.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these paragraphs mention panic = \"abort\" without warning against it:\n\n{}",
        offenders.join("\n\n")
    );
}

/// The same check for the profile `generate_scaffold` writes, so the generator
/// and the validator cannot drift apart either.
#[test]
fn the_scaffold_generates_a_profile_its_own_validator_accepts() {
    use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};
    use quack_rs::validate::validate_release_profile;

    let files = generate_scaffold(&ScaffoldConfig {
        name: "profile_probe".to_string(),
        description: "Checks the generated release profile".to_string(),
        version: "0.1.0".to_string(),
        license: "MIT".to_string(),
        maintainer: "tomtom215".to_string(),
        github_repo: "tomtom215/duckdb-profile-probe".to_string(),
        excluded_platforms: vec![],
        ..Default::default()
    })
    .expect("scaffold");

    let manifest = &files
        .iter()
        .find(|f| f.path == "Cargo.toml")
        .expect("the scaffold writes a Cargo.toml")
        .content;

    let panic = release_profile_value(manifest, "panic")
        .expect("the generated profile must state its panic strategy explicitly");
    let lto = release_profile_value(manifest, "lto").unwrap_or_default();
    let opt = release_profile_value(manifest, "opt-level").unwrap_or_default();
    let cgu = release_profile_value(manifest, "codegen-units").unwrap_or_default();

    let check = validate_release_profile(&panic, &lto, &opt, &cgu)
        .unwrap_or_else(|e| panic!("generated Cargo.toml: {e}"));
    assert!(check.is_fully_optimized());
}
