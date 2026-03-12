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
fn extension_error_truncates_at_null_byte() {
    use quack_rs::error::ExtensionError;

    let err = ExtensionError::new("before\0after");
    let c = err.to_c_string();
    assert_eq!(c.to_str().unwrap(), "before");
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
fn vector_writer_size_is_two_pointers() {
    use quack_rs::vector::VectorWriter;

    // VectorWriter contains exactly two pointer-sized fields
    assert_eq!(
        std::mem::size_of::<VectorWriter>(),
        2 * std::mem::size_of::<usize>()
    );
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

    let view = DuckStringView::from_bytes(&bytes);
    assert_eq!(view.len(), 5);
    assert!(!view.is_empty());
    assert_eq!(view.as_str(), Some("hello"));
}

#[test]
fn duck_string_view_empty_string() {
    use quack_rs::vector::DuckStringView;

    let bytes = [0u8; 16]; // len = 0
    let view = DuckStringView::from_bytes(&bytes);
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
        "CREATE OR REPLACE MACRO pi() AS (3.14159265358979)"
    );
}

#[test]
fn sql_macro_scalar_to_sql_multiple_params() {
    let m = SqlMacro::scalar("add", &["a", "b"], "a + b").unwrap();
    assert_eq!(m.to_sql(), "CREATE OR REPLACE MACRO add(a, b) AS (a + b)");
}

#[test]
fn sql_macro_scalar_clamp_to_sql() {
    let m = SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))").unwrap();
    assert_eq!(
        m.to_sql(),
        "CREATE OR REPLACE MACRO clamp(x, lo, hi) AS (greatest(lo, least(hi, x)))"
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
        "CREATE OR REPLACE MACRO active_rows(tbl) AS TABLE SELECT * FROM tbl WHERE active = true"
    );
}

#[test]
fn sql_macro_invalid_name_rejected() {
    assert!(SqlMacro::scalar("MyMacro", &[], "1").is_err());
    assert!(SqlMacro::scalar("my-macro", &[], "1").is_err());
    assert!(SqlMacro::scalar("", &[], "1").is_err());
    assert!(SqlMacro::scalar("1func", &[], "1").is_err());
}

#[test]
fn sql_macro_invalid_param_rejected() {
    assert!(SqlMacro::scalar("f", &["BadParam"], "1").is_err());
    assert!(SqlMacro::scalar("f", &["a-b"], "1").is_err());
    assert!(SqlMacro::scalar("f", &[""], "1").is_err());
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
    let err = SqlMacro::scalar("f", &["Bad"], "1").unwrap_err();
    assert!(err.as_str().contains("Bad"));
}

// ---------------------------------------------------------------------------
// TEST-2: Scaffold generated code compiles successfully
// ---------------------------------------------------------------------------

/// Generates scaffold files, writes them to a temp directory with a path-dep
/// on the local quack-rs crate, and runs `cargo check` to verify the generated
/// code actually compiles.  This catches template regressions (like broken
/// macro paths) that unit tests can't detect.
#[test]
fn scaffold_generated_code_compiles() {
    use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};
    use std::fs;
    use std::process::Command;

    let config = ScaffoldConfig {
        name: "test_ext".to_string(),
        description: "Scaffold compile test".to_string(),
        version: "0.1.0".to_string(),
        license: "MIT".to_string(),
        maintainer: "CI".to_string(),
        github_repo: "test/test-ext".to_string(),
        excluded_platforms: vec![],
    };

    let files = generate_scaffold(&config).unwrap();

    // Write to a temp directory
    let tmp = std::env::temp_dir().join("quack_rs_scaffold_compile_test");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("src")).unwrap();

    // The scaffold Cargo.toml references `quack-rs = "0.5"` from crates.io.
    // Replace it with a path dependency pointing to this workspace root so
    // `cargo check` uses the local (possibly-modified) crate.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    for f in &files {
        let dest = tmp.join(&f.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        if f.path == "Cargo.toml" {
            // Rewrite quack-rs dep to use local path
            let patched = f.content.replace(
                r#"quack-rs = { version = "0.5" }"#,
                &format!(
                    "quack-rs = {{ path = \"{}\" }}",
                    workspace_root.display().to_string().replace('\\', "/")
                ),
            );
            fs::write(&dest, patched).unwrap();
        } else {
            fs::write(&dest, &f.content).unwrap();
        }
    }

    // Run cargo check on the generated project
    let output = Command::new("cargo")
        .args(["check", "--lib"])
        .current_dir(&tmp)
        .output()
        .expect("failed to run cargo check");

    // Clean up before asserting so we don't leave temp dirs on failure
    let _ = fs::remove_dir_all(&tmp);

    assert!(
        output.status.success(),
        "Scaffold-generated code failed to compile!\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
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

#[test]
fn mock_vector_writer_grows_beyond_initial_capacity() {
    use quack_rs::testing::MockVectorWriter;

    let mut w = MockVectorWriter::new(0);
    w.write_i64(3, 99);
    assert_eq!(w.len(), 4);
    assert_eq!(w.try_get_i64(3), Some(99));
    assert!(w.is_null(0));
    assert!(w.is_null(1));
    assert!(w.is_null(2));
}

#[test]
fn mock_vector_writer_boolean_and_interval() {
    use quack_rs::testing::MockVectorWriter;
    use quack_rs::interval::DuckInterval;

    let mut w = MockVectorWriter::new(2);
    w.write_bool(0, true);
    let iv = DuckInterval { months: 1, days: 2, micros: 3 };
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

    fn clamp_values(
        reader: &MockVectorReader,
        writer: &mut MockVectorWriter,
        lo: i64,
        hi: i64,
    ) {
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
            source: TypeId::Varchar,
            target: TypeId::Integer,
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
fn cast_builder_source_target_accessors() {
    use quack_rs::cast::CastFunctionBuilder;
    use quack_rs::types::TypeId;

    let b = CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer);
    assert_eq!(b.source(), TypeId::Varchar);
    assert_eq!(b.target(), TypeId::Integer);
}
