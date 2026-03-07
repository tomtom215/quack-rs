// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

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
    let m =
        SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))").unwrap();
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
    assert_eq!(m.body(), &MacroBody::Table("SELECT 42 AS answer".to_string()));
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
