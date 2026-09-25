// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;

#[test]
fn writer_write_and_read_i64() {
    let mut w = MockVectorWriter::new(3);
    w.write_i64(0, 42);
    w.write_i64(1, -100);
    w.set_null(2);
    assert_eq!(w.try_get_i64(0), Some(42));
    assert_eq!(w.try_get_i64(1), Some(-100));
    assert!(w.is_null(2));
}

/// A real vector has a fixed capacity; the mock used to grow silently, so
/// a test could pass for a callback that writes out of bounds.
#[test]
#[should_panic(expected = "out of bounds for a mock vector of capacity 1")]
fn writer_refuses_to_write_past_its_capacity() {
    let mut w = MockVectorWriter::new(1);
    w.write_i64(5, 99);
}

#[test]
#[should_panic(expected = "out of bounds")]
fn writer_set_null_past_capacity_panics() {
    MockVectorWriter::new(2).set_null(2);
}

#[test]
fn writer_set_null_hides_a_previous_value() {
    let mut w = MockVectorWriter::new(1);
    w.write_i64(0, 42);
    assert!(!w.is_null(0));
    w.set_null(0);
    assert!(w.is_null(0));
    assert_eq!(w.try_get_i64(0), None);
    assert!(w.is_written(0));
}

/// Validated against `DuckDB` 1.5.5: `set_null` followed by `write_i64` on a
/// real output vector still reads back NULL. The mock used to report the
/// written value, so it hid exactly that bug.
#[test]
fn writer_write_after_set_null_stays_null_like_duckdb() {
    let mut w = MockVectorWriter::new(1);
    w.set_null(0);
    w.write_i64(0, 7);
    assert!(w.is_null(0));
    assert_eq!(w.try_get_i64(0), None);
    w.set_valid(0);
    assert_eq!(w.try_get_i64(0), Some(7));
}

/// Validated against `DuckDB` 1.5.5: rows a callback never writes come back
/// valid (with stale data), not NULL. The mock used to report them NULL.
#[test]
fn writer_unwritten_rows_are_valid_but_unwritten() {
    let w = MockVectorWriter::new(2);
    assert!(!w.is_null(0));
    assert!(!w.is_written(0));
    assert_eq!(w.get(0), None);
}

#[test]
fn writer_varchar() {
    let mut w = MockVectorWriter::new(2);
    w.write_varchar(0, "hello");
    w.set_null(1);
    assert_eq!(w.try_get_str(0), Some("hello"));
    assert!(w.is_null(1));
}

#[test]
fn writer_all_types_round_trip() {
    let mut w = MockVectorWriter::new(10);
    w.write_i8(0, 127);
    w.write_i16(1, 1000);
    w.write_i32(2, 100_000);
    w.write_i64(3, 1_000_000_000);
    w.write_u8(4, 255);
    w.write_u32(5, 999);
    w.write_u64(6, u64::MAX);
    w.write_f32(7, std::f32::consts::PI);
    w.write_f64(8, std::f64::consts::PI);
    w.write_bool(9, true);

    assert!(matches!(w.get(0), Some(MockDuckValue::I8(127))));
    assert!(matches!(w.get(1), Some(MockDuckValue::I16(1000))));
    assert!(matches!(w.get(2), Some(MockDuckValue::I32(100_000))));
    assert_eq!(w.try_get_i64(3), Some(1_000_000_000));
    assert!(matches!(w.get(4), Some(MockDuckValue::U8(255))));
    assert_eq!(w.try_get_bool(9), Some(true));
}

#[test]
fn reader_from_i64s() {
    let r = MockVectorReader::from_i64s([Some(1), None, Some(3)]);
    assert_eq!(r.row_count(), 3);
    assert!(r.is_valid(0));
    assert!(!r.is_valid(1));
    assert!(r.is_valid(2));
    assert_eq!(r.try_get_i64(0), Some(1));
    assert_eq!(r.try_get_i64(1), None);
    assert_eq!(r.try_get_i64(2), Some(3));
}

#[test]
fn reader_from_strs() {
    let r = MockVectorReader::from_strs([Some("hello"), None, Some("world")]);
    assert_eq!(r.try_get_str(0), Some("hello"));
    assert_eq!(r.try_get_str(1), None);
    assert_eq!(r.try_get_str(2), Some("world"));
}

/// Reading past the end is refused, as the writer refuses writing past it: a
/// real reader has no bounds check, so an off-by-one loop must fail here
/// rather than pass.
#[test]
#[should_panic(expected = "row 99 is out of bounds for a mock reader of 1 row(s)")]
fn reader_out_of_bounds_is_valid_panics() {
    let r = MockVectorReader::from_i64s([Some(1)]);
    let _ = r.is_valid(99);
}

#[test]
#[should_panic(expected = "row 1 is out of bounds for a mock reader of 1 row(s)")]
fn reader_out_of_bounds_typed_getter_panics() {
    let r = MockVectorReader::from_i64s([Some(1)]);
    let _ = r.try_get_i64(1);
}

#[test]
fn reader_from_i32s() {
    let r = MockVectorReader::from_i32s([Some(10), None, Some(-5)]);
    assert_eq!(r.row_count(), 3);
    assert!(r.is_valid(0));
    assert!(!r.is_valid(1));
    assert_eq!(r.try_get_i32(0), Some(10));
    assert_eq!(r.try_get_i32(1), None);
    assert_eq!(r.try_get_i32(2), Some(-5));
}

#[test]
fn reader_from_f64s() {
    let r = MockVectorReader::from_f64s([Some(1.5), None, Some(-2.72)]);
    assert_eq!(r.row_count(), 3);
    assert!(r.is_valid(0));
    assert!(!r.is_valid(1));
    assert_eq!(r.try_get_f64(0), Some(1.5));
    assert_eq!(r.try_get_f64(1), None);
    assert_eq!(r.try_get_f64(2), Some(-2.72));
}

#[test]
fn reader_from_bools() {
    let r = MockVectorReader::from_bools([Some(true), None, Some(false)]);
    assert_eq!(r.row_count(), 3);
    assert!(r.is_valid(0));
    assert!(!r.is_valid(1));
    assert_eq!(r.try_get_bool(0), Some(true));
    assert_eq!(r.try_get_bool(1), None);
    assert_eq!(r.try_get_bool(2), Some(false));
}

#[test]
fn writer_typed_getters_i32() {
    let mut w = MockVectorWriter::new(2);
    w.write_i32(0, 42);
    w.set_null(1);
    assert_eq!(w.try_get_i32(0), Some(42));
    assert_eq!(w.try_get_i32(1), None);
    // Wrong type returns None
    assert_eq!(w.try_get_i64(0), None);
}

#[test]
fn writer_typed_getters_f64() {
    let mut w = MockVectorWriter::new(2);
    w.write_f64(0, 2.72);
    w.set_null(1);
    assert_eq!(w.try_get_f64(0), Some(2.72));
    assert_eq!(w.try_get_f64(1), None);
}

#[test]
fn writer_typed_getters_bool() {
    let mut w = MockVectorWriter::new(2);
    w.write_bool(0, true);
    w.write_bool(1, false);
    assert_eq!(w.try_get_bool(0), Some(true));
    assert_eq!(w.try_get_bool(1), Some(false));
}

#[test]
fn writer_u16_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_u16(0, 12345);
    assert!(matches!(w.get(0), Some(MockDuckValue::U16(12345))));
}

#[test]
fn writer_i128_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_i128(0, i128::MAX);
    assert!(matches!(w.get(0), Some(MockDuckValue::I128(v)) if *v == i128::MAX));
}

#[test]
fn writer_interval_round_trip() {
    let interval = DuckInterval {
        months: 1,
        days: 2,
        micros: 3_000_000,
    };
    let mut w = MockVectorWriter::new(1);
    w.write_interval(0, interval);
    assert_eq!(w.try_get_interval(0), Some(interval));
}

#[test]
fn reader_interval_round_trip() {
    let interval = DuckInterval {
        months: 6,
        days: 15,
        micros: 500_000,
    };
    let r = MockVectorReader::new([Some(MockDuckValue::Interval(interval))]);
    assert_eq!(r.try_get_interval(0), Some(interval));
}

#[test]
fn reader_wrong_type_returns_none() {
    let r = MockVectorReader::from_i64s([Some(42)]);
    assert_eq!(r.try_get_i32(0), None);
    assert_eq!(r.try_get_f64(0), None);
    assert_eq!(r.try_get_bool(0), None);
    assert_eq!(r.try_get_str(0), None);
    assert_eq!(r.try_get_interval(0), None);
}

#[test]
fn writer_is_empty() {
    let w = MockVectorWriter::new(0);
    assert!(w.is_empty());
    let w2 = MockVectorWriter::new(1);
    assert!(!w2.is_empty());
}

#[test]
fn writer_len_is_its_capacity() {
    assert_eq!(MockVectorWriter::new(0).len(), 0);
    assert_eq!(MockVectorWriter::new(5).len(), 5);
}

#[test]
fn writer_try_get_i8_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_i8(0, -42);
    assert_eq!(w.try_get_i8(0), Some(-42));
}

#[test]
fn writer_try_get_i16_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_i16(0, 1234);
    assert_eq!(w.try_get_i16(0), Some(1234));
}

#[test]
fn writer_try_get_u8_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_u8(0, 255);
    assert_eq!(w.try_get_u8(0), Some(255));
}

#[test]
fn writer_try_get_u16_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_u16(0, 60000);
    assert_eq!(w.try_get_u16(0), Some(60000));
}

#[test]
fn writer_try_get_u32_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_u32(0, 123_456);
    assert_eq!(w.try_get_u32(0), Some(123_456));
}

#[test]
fn writer_try_get_u64_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_u64(0, u64::MAX);
    assert_eq!(w.try_get_u64(0), Some(u64::MAX));
}

#[test]
fn writer_try_get_f32_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_f32(0, 2.5);
    assert_eq!(w.try_get_f32(0), Some(2.5));
}

#[test]
fn writer_try_get_i128_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_i128(0, i128::MIN);
    assert_eq!(w.try_get_i128(0), Some(i128::MIN));
}

#[test]
fn reader_from_i8s() {
    let r = MockVectorReader::from_i8s([Some(1), None, Some(-1)]);
    assert_eq!(r.row_count(), 3);
    assert_eq!(r.try_get_i8(0), Some(1));
    assert!(!r.is_valid(1));
    assert_eq!(r.try_get_i8(2), Some(-1));
}

#[test]
fn reader_from_i16s() {
    let r = MockVectorReader::from_i16s([Some(100), None]);
    assert_eq!(r.try_get_i16(0), Some(100));
    assert!(!r.is_valid(1));
}

#[test]
fn reader_from_u8s() {
    let r = MockVectorReader::from_u8s([Some(255), None]);
    assert_eq!(r.try_get_u8(0), Some(255));
}

#[test]
fn reader_from_u16s() {
    let r = MockVectorReader::from_u16s([Some(60000)]);
    assert_eq!(r.try_get_u16(0), Some(60000));
}

#[test]
fn reader_from_u32s() {
    let r = MockVectorReader::from_u32s([Some(999_999)]);
    assert_eq!(r.try_get_u32(0), Some(999_999));
}

#[test]
fn reader_from_u64s() {
    let r = MockVectorReader::from_u64s([Some(u64::MAX), None]);
    assert_eq!(r.try_get_u64(0), Some(u64::MAX));
    assert!(!r.is_valid(1));
}

#[test]
fn reader_from_f32s() {
    let r = MockVectorReader::from_f32s([Some(1.5), None]);
    assert_eq!(r.try_get_f32(0), Some(1.5));
}

#[test]
fn reader_from_i128s() {
    let r = MockVectorReader::from_i128s([Some(i128::MAX), None]);
    assert_eq!(r.try_get_i128(0), Some(i128::MAX));
}

#[test]
fn reader_from_intervals() {
    let iv = DuckInterval {
        months: 1,
        days: 2,
        micros: 3,
    };
    let r = MockVectorReader::from_intervals([Some(iv), None]);
    assert_eq!(r.try_get_interval(0), Some(iv));
    assert!(!r.is_valid(1));
}

#[test]
fn mock_double_pattern() {
    // Demonstrates extracting callback logic into a testable pure-Rust function.
    fn double_values(reader: &MockVectorReader, writer: &mut MockVectorWriter) {
        for i in 0..reader.row_count() {
            if reader.is_valid(i) {
                let v = reader.try_get_i64(i).unwrap_or(0);
                writer.write_i64(i, v * 2);
            } else {
                writer.set_null(i);
            }
        }
    }

    let reader = MockVectorReader::from_i64s([Some(1), Some(5), None, Some(-3)]);
    let mut writer = MockVectorWriter::new(4);
    double_values(&reader, &mut writer);

    assert_eq!(writer.try_get_i64(0), Some(2));
    assert_eq!(writer.try_get_i64(1), Some(10));
    assert!(writer.is_null(2));
    assert_eq!(writer.try_get_i64(3), Some(-6));
}

#[test]
fn writer_blob_round_trip() {
    let mut w = MockVectorWriter::new(2);
    w.write_blob(0, b"hello bytes");
    w.set_null(1);
    assert_eq!(w.try_get_blob(0), Some(b"hello bytes".as_slice()));
    assert_eq!(w.try_get_blob(1), None);
    // Wrong type returns None
    assert_eq!(w.try_get_i64(0), None);
}

#[test]
fn writer_uuid_round_trip() {
    let mut w = MockVectorWriter::new(1);
    let uuid_val: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    w.write_uuid(0, uuid_val);
    assert_eq!(w.try_get_uuid(0), Some(uuid_val));
}

#[test]
fn writer_date_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_date(0, 19815); // days since epoch
    assert_eq!(w.try_get_i32(0), Some(19815));
}

#[test]
fn writer_timestamp_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_timestamp(0, 1_711_756_800_000_000); // micros since epoch
    assert_eq!(w.try_get_i64(0), Some(1_711_756_800_000_000));
}

#[test]
fn writer_time_round_trip() {
    let mut w = MockVectorWriter::new(1);
    w.write_time(0, 43_200_000_000); // noon in micros
    assert_eq!(w.try_get_i64(0), Some(43_200_000_000));
}

#[test]
fn reader_blob_round_trip() {
    let r = MockVectorReader::from_blobs([Some(b"data".as_slice()), None]);
    assert_eq!(r.try_get_blob(0), Some(b"data".as_slice()));
    assert_eq!(r.try_get_blob(1), None);
    assert!(!r.is_valid(1));
}

#[test]
fn reader_uuid_round_trip() {
    let bits: u128 = 0x0ead_beef_cafe_babe_1234_5678_9abc_def0;
    let storage = crate::vector::uuid_to_storage(bits);
    let r = MockVectorReader::from_i128s([Some(storage)]);
    // The UUID accessor undoes DuckDB's top-bit flip; the raw i128
    // accessor does not. A mock that conflated the two would let a
    // callback pass its tests and still write the wrong UUID.
    assert_eq!(r.try_get_uuid(0), Some(bits));
    assert_eq!(r.try_get_i128(0), Some(storage));
    #[allow(clippy::cast_sign_loss)]
    {
        assert_ne!(storage as u128, bits);
    }
}
