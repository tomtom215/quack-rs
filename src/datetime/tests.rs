// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Tests for [`super`]; the input checks have their own in `checks.rs`.

mod unit {
    use super::super::*;

    #[test]
    fn date_struct_round_trips_through_ffi_types() {
        let date = Date {
            year: 2026,
            month: 8,
            day: 18,
        };
        let raw: duckdb_date_struct = date.into();
        assert_eq!(raw.year, 2026);
        assert_eq!(raw.month, 8);
        assert_eq!(raw.day, 18);
        assert_eq!(Date::from(raw), date);
    }

    #[test]
    fn time_struct_round_trips_through_ffi_types() {
        let time = Time {
            hour: 23,
            min: 59,
            sec: 58,
            micros: 123_456,
        };
        let raw: duckdb_time_struct = time.into();
        assert_eq!(Time::from(raw), time);
    }

    #[test]
    fn decimal_is_ordered_and_hashable() {
        use std::collections::HashSet;
        let a = Decimal {
            width: 18,
            scale: 3,
            value: 1_500,
        };
        let b = Decimal {
            width: 18,
            scale: 3,
            value: 2_500,
        };
        assert!(a < b);
        let set: HashSet<Decimal> = [a, b, a].into_iter().collect();
        assert_eq!(set.len(), 2);
    }
}

/// Conversions checked against a live `DuckDB`.
#[cfg(feature = "_duckdb-testing")]
mod live_tests {
    use super::super::*;
    use crate::testing::InMemoryDb;

    #[test]
    fn epoch_day_zero_is_1970_01_01() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        let date = unsafe { date_from_days(0) };
        assert_eq!(
            date,
            Date {
                year: 1970,
                month: 1,
                day: 1
            }
        );
    }

    #[test]
    fn date_round_trips_across_leap_years_and_bce() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        for days in [
            -1_000_000_i32,
            -719_162, // 0001-01-01
            -1,
            0,
            1,
            59,     // 1970-03-01
            10_957, // 2000-01-01
            11_017, // 2000-03-01, just past a leap day
            20_685, // 2026-08-18
            1_000_000,
        ] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let date = unsafe { date_from_days(days) };
            assert_eq!(
                unsafe { date_to_days(date) },
                Some(days),
                "round trip for {days}"
            );
        }
    }

    #[test]
    fn duckdb_agrees_with_our_conversion() {
        // Cross-check against DuckDB's SQL layer rather than trusting the C API
        // wrapper in isolation.
        let db = InMemoryDb::open().expect("open in-memory DuckDB");
        for days in [0_i32, 20_685, -719_162] {
            // `INTERVAL {n} DAY` will not parse a negative literal, so add the
            // interval as an expression instead.
            let sql =
                format!("SELECT strftime(DATE '1970-01-01' + INTERVAL ({days}) DAY, '%Y-%m-%d')");
            let expected: String = db.query_one(&sql).expect("query");
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let date = unsafe { date_from_days(days) };
            let actual = format!("{:04}-{:02}-{:02}", date.year, date.month, date.day);
            assert_eq!(actual, expected, "for {days} days since the epoch");
        }
    }

    #[test]
    fn infinity_sentinels_match_the_documented_constants() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            assert!(is_finite_date(0));
            assert!(!is_finite_date(DATE_INFINITY_DAYS));
            assert!(!is_finite_date(DATE_NEGATIVE_INFINITY_DAYS));
            // -infinity is -i32::MAX, so i32::MIN is one step beyond it and is a
            // finite (if nonsensical) date. Getting this backwards would make a
            // caller treat a real date as infinity.
            assert!(is_finite_date(i32::MIN));

            assert!(is_finite_timestamp(0));
            assert!(!is_finite_timestamp(TIMESTAMP_INFINITY_MICROS));
            assert!(!is_finite_timestamp(TIMESTAMP_NEGATIVE_INFINITY_MICROS));
            assert!(is_finite_timestamp(i64::MIN));

            assert!(is_finite_timestamp_s(0));
            assert!(is_finite_timestamp_ms(0));
            assert!(is_finite_timestamp_ns(0));
            assert!(!is_finite_timestamp_s(TIMESTAMP_INFINITY_MICROS));
            assert!(!is_finite_timestamp_ms(TIMESTAMP_INFINITY_MICROS));
            assert!(!is_finite_timestamp_ns(TIMESTAMP_INFINITY_MICROS));
        }
    }

    #[test]
    fn duckdb_sql_agrees_that_the_sentinels_are_infinite() {
        let db = InMemoryDb::open().expect("open in-memory DuckDB");
        let rendered: String = db
            .query_one("SELECT ('infinity'::DATE)::VARCHAR")
            .expect("query");
        assert_eq!(rendered, "infinity");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        assert!(!unsafe { is_finite_date(DATE_INFINITY_DAYS) });
    }

    #[test]
    fn time_round_trips_including_microsecond_precision() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        for micros in [0_i64, 1, 999_999, 1_000_000, 86_399_999_999] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let time = unsafe { time_from_micros(micros) };
            assert_eq!(unsafe { time_to_micros(time) }, micros, "for {micros} us");
        }
        // SAFETY: dispatch table initialised above.
        let end_of_day = unsafe { time_from_micros(86_399_999_999) };
        assert_eq!(
            end_of_day,
            Time {
                hour: 23,
                min: 59,
                sec: 59,
                micros: 999_999
            }
        );
    }

    #[test]
    fn timestamp_round_trips() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        for micros in [0_i64, 1, -1, 1_700_000_000_000_000, -1_700_000_000_000_000] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let ts = unsafe { timestamp_from_micros(micros) }.expect("finite");
            assert_eq!(
                unsafe { timestamp_to_micros(ts) },
                Some(micros),
                "for {micros} us"
            );
        }
    }

    #[test]
    fn time_tz_round_trips_with_offset() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            let bits = time_tz_bits(12 * 3_600 * 1_000_000, -5 * 3_600).expect("in range");
            let decoded = time_tz_from_bits(bits);
            assert_eq!(decoded.time.hour, 12);
            assert_eq!(decoded.offset_seconds, -5 * 3_600);
        }
    }

    #[test]
    fn hugeint_conversions_match_duckdb() {
        let db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            assert!((hugeint_to_f64(0) - 0.0).abs() < f64::EPSILON);
            assert!((hugeint_to_f64(1) - 1.0).abs() < f64::EPSILON);
            assert!((hugeint_to_f64(-1) + 1.0).abs() < f64::EPSILON);
            assert_eq!(f64_to_hugeint(42.0), 42);
            assert_eq!(f64_to_hugeint(-42.0), -42);
            assert_eq!(f64_to_uhugeint(42.0), 42);
            assert!((uhugeint_to_f64(u128::from(u64::MAX)) - 1.844_674_407_370_955e19).abs() < 1e6);
        }
        // Cross-check the sign handling of the split representation against SQL.
        let expected: f64 = db
            .query_one("SELECT (-170141183460469231731687303715884105728)::HUGEINT::DOUBLE")
            .expect("query");
        // SAFETY: dispatch table initialised above.
        let actual = unsafe { hugeint_to_f64(i128::MIN) };
        assert!(
            (actual - expected).abs() / expected.abs() < 1e-12,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn decimal_conversions_preserve_width_and_scale() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            let decimal = f64_to_decimal(12.345, 18, 3);
            assert_eq!(decimal.width, 18);
            assert_eq!(decimal.scale, 3);
            assert_eq!(decimal.value, 12_345);
            assert!((decimal_to_f64(decimal).expect("valid") - 12.345).abs() < 1e-9);

            let negative = f64_to_decimal(-12.345, 18, 3);
            assert_eq!(negative.value, -12_345);
            assert!((decimal_to_f64(negative).expect("valid") + 12.345).abs() < 1e-9);
        }
    }

    // ─── Inputs on which DuckDB throws (formerly a process abort) ───────────

    #[test]
    fn an_invalid_date_is_none_not_an_abort() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        let date = |year, month, day| Date { year, month, day };
        for bad in [
            date(2026, 13, 1),
            date(2026, 0, 1),
            date(2026, 2, 29),
            date(2026, 4, 31),
            date(2026, 1, 0),
            date(5_881_580, 7, 11),
            date(-5_877_641, 6, 24),
            date(i32::MAX, 1, 1),
        ] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            assert_eq!(unsafe { date_to_days(bad) }, None, "{bad:?}");
            let ts = Timestamp {
                date: bad,
                time: Time {
                    hour: 0,
                    min: 0,
                    sec: 0,
                    micros: 0,
                },
            };
            // SAFETY: as above.
            assert_eq!(unsafe { timestamp_to_micros(ts) }, None, "{bad:?}");
        }
        // The extremes DuckDB does accept still convert, to the documented
        // days -(2^31 - 2) and 2^31 - 2.
        // SAFETY: as above.
        unsafe {
            assert_eq!(date_to_days(date(2024, 2, 29)), Some(19_782));
            assert_eq!(date_to_days(date(5_881_580, 7, 10)), Some(i32::MAX - 1));
            assert_eq!(date_to_days(date(-5_877_641, 6, 25)), Some(-(i32::MAX - 1)));
        }
    }

    #[test]
    fn an_undecomposable_timestamp_is_none_not_an_abort() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            assert_eq!(timestamp_from_micros(TIMESTAMP_INFINITY_MICROS), None);
            assert_eq!(
                timestamp_from_micros(TIMESTAMP_NEGATIVE_INFINITY_MICROS),
                None
            );
            assert_eq!(timestamp_from_micros(i64::MIN), None);
            // The finite neighbours still decompose.
            assert!(timestamp_from_micros(i64::MAX - 1).is_some());
            assert!(timestamp_from_micros(-106_751_991 * MICROS_PER_DAY).is_some());
            // Finite, but it floors to a day DuckDB cannot multiply back.
            assert_eq!(
                timestamp_from_micros(-106_751_991 * MICROS_PER_DAY - 1),
                None
            );
        }
    }

    #[test]
    fn a_timestamp_out_of_range_is_none_not_an_abort() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        let midnight = Time {
            hour: 0,
            min: 0,
            sec: 0,
            micros: 0,
        };
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            // A valid DATE far past the TIMESTAMP range: days * MICROS_PER_DAY
            // overflows, which makes `Timestamp::FromDatetime` throw.
            let far = Timestamp {
                date: Date {
                    year: 1_000_000,
                    month: 1,
                    day: 1,
                },
                time: midnight,
            };
            assert_eq!(timestamp_to_micros(far), None);
            // The last representable instant round-trips; one microsecond
            // later is the infinity sentinel, which DuckDB also refuses.
            let last = timestamp_from_micros(i64::MAX - 1).expect("finite");
            assert_eq!(timestamp_to_micros(last), Some(i64::MAX - 1));
            let mut past = last;
            past.time.micros += 1;
            assert_eq!(timestamp_to_micros(past), None);
        }
    }

    #[test]
    fn out_of_range_time_tz_fields_are_rejected() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            assert_eq!(time_tz_bits(0, TIME_TZ_MAX_OFFSET_SECONDS + 1), None);
            assert_eq!(time_tz_bits(0, -TIME_TZ_MAX_OFFSET_SECONDS - 1), None);
            assert_eq!(time_tz_bits(-1, 0), None);
            assert_eq!(time_tz_bits(MICROS_PER_DAY + 1, 0), None);
            // Both limits themselves encode and decode losslessly.
            for (micros, offset) in [
                (MICROS_PER_DAY, TIME_TZ_MAX_OFFSET_SECONDS),
                (0, -TIME_TZ_MAX_OFFSET_SECONDS),
            ] {
                let bits = time_tz_bits(micros, offset).expect("in range");
                let decoded = time_tz_from_bits(bits);
                assert_eq!(decoded.offset_seconds, offset);
                assert_eq!(time_to_micros(decoded.time), micros);
            }
        }
    }

    #[test]
    fn an_out_of_range_decimal_scale_is_rejected() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        let decimal = |width, scale| Decimal {
            width,
            scale,
            value: 1,
        };
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            // DuckDB returned inf for scale 40 and NaN for 60 (reads past its
            // powers-of-ten tables).
            assert_eq!(decimal_to_f64(decimal(38, 40)), None);
            assert_eq!(decimal_to_f64(decimal(38, 60)), None);
            assert_eq!(decimal_to_f64(decimal(39, 0)), None);
            assert_eq!(decimal_to_f64(decimal(4, 5)), None);
            let widest = decimal_to_f64(decimal(38, 38)).expect("valid");
            assert!((widest - 1e-38).abs() < 1e-50, "{widest}");
        }
    }
}
