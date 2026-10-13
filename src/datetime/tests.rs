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

    #[test]
    fn infinity_sentinels_have_duckdbs_exact_values() {
        // `date_t::ninfinity()` / `timestamp_t::ninfinity()` are `-MAX`, one
        // above `MIN`; the live test below checks DuckDB agrees. Pinned here
        // too so the values are checked without a live engine.
        assert_eq!(DATE_INFINITY_DAYS, 2_147_483_647);
        assert_eq!(DATE_NEGATIVE_INFINITY_DAYS, -2_147_483_647);
        assert_eq!(DATE_NEGATIVE_INFINITY_DAYS, i32::MIN + 1);
        assert_eq!(TIMESTAMP_INFINITY_MICROS, 9_223_372_036_854_775_807);
        assert_eq!(
            TIMESTAMP_NEGATIVE_INFINITY_MICROS,
            -9_223_372_036_854_775_807
        );
        assert_eq!(TIMESTAMP_NEGATIVE_INFINITY_MICROS, i64::MIN + 1);
    }

    // The checked conversions refuse these inputs before calling `DuckDB`, so
    // they run without a database. (Reaching `DuckDB` here would panic: the
    // dispatch table is not initialised.) The accepted side, and agreement
    // with `DuckDB`'s own failures, are in `live_tests`.

    #[test]
    fn try_f64_to_hugeint_refuses_without_calling_duckdb() {
        // Exact literals, not `2f64.powi(127)`: Miri perturbs `powi`'s result.
        let two_127 = 170_141_183_460_469_231_731_687_303_715_884_105_728.0_f64;
        // SAFETY: every input is refused before the `DuckDB` call.
        unsafe {
            for refused in [
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                two_127,
                -two_127,
                1e40,
                -1e40,
            ] {
                assert_eq!(try_f64_to_hugeint(refused), None, "{refused}");
            }
        }
    }

    #[test]
    fn try_f64_to_uhugeint_refuses_without_calling_duckdb() {
        // Exact literals, not `2f64.powi(128)`: Miri perturbs `powi`'s result.
        let two_128 = 340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64;
        // SAFETY: every input is refused before the `DuckDB` call.
        unsafe {
            for refused in [
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                -1.0,
                -f64::MIN_POSITIVE,
                two_128,
            ] {
                assert_eq!(try_f64_to_uhugeint(refused), None, "{refused}");
            }
        }
    }

    #[test]
    fn try_f64_to_decimal_refuses_an_invalid_width_or_scale_without_calling_duckdb() {
        // SAFETY: every input is refused before the `DuckDB` call.
        unsafe {
            for (width, scale) in [(0, 0), (39, 0), (4, 5), (38, 39)] {
                assert_eq!(
                    try_f64_to_decimal(1.0, width, scale),
                    None,
                    "{width} {scale}"
                );
            }
        }
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

    /// `duckdb_from_date` does no validation and never throws, so a sentinel
    /// decomposes into fields rather than failing. The documented consequence
    /// is pinned here: `infinity` becomes a *plausible* date one day past
    /// `DuckDB`'s largest, so a caller must check `is_finite_date` first.
    #[test]
    fn decomposing_a_date_sentinel_is_arithmetic_not_an_error() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            let inf = date_from_days(DATE_INFINITY_DAYS);
            assert_eq!((inf.year, inf.month, inf.day), (5_881_580, 7, 11));
            let _ = date_from_days(DATE_NEGATIVE_INFINITY_DAYS);
            let _ = date_from_days(i32::MIN);
        }
    }

    /// `Time::Convert` asserts that the fields it produces are a valid time,
    /// so these inputs aborted a `DuckDB` built with assertions (the
    /// `bundled-test` feature builds one); a release build returned
    /// out-of-range fields instead.
    #[test]
    fn an_out_of_range_time_is_none_not_an_abort() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            for bad in [-1, MICROS_PER_DAY + 1, i64::MIN, i64::MAX] {
                assert_eq!(time_from_micros(bad), None, "{bad}");
            }
            let midnight = time_from_micros(MICROS_PER_DAY).expect("24:00:00");
            assert_eq!((midnight.hour, midnight.min, midnight.micros), (24, 0, 0));

            // The 40-bit time field of a TIMETZ, one microsecond past a day
            // and at its maximum; the offset field is a valid +00:00.
            let offset_zero = u64::try_from(TIME_TZ_MAX_OFFSET_SECONDS).unwrap();
            for micros in [MICROS_PER_DAY.unsigned_abs() + 1, (1 << 40) - 1] {
                assert_eq!(time_tz_from_bits((micros << 24) | offset_zero), None);
            }
            // An offset field past +-15:59:59 is not one `time_tz_bits` makes.
            assert_eq!(time_tz_from_bits(0x00FF_FFFF), None);
            let bits = time_tz_bits(MICROS_PER_DAY, 0).expect("in range");
            assert_eq!(time_tz_from_bits(bits).expect("in range").time.hour, 24);
        }
    }

    #[test]
    fn time_round_trips_including_microsecond_precision() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        for micros in [0_i64, 1, 999_999, 1_000_000, 86_399_999_999] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let time = unsafe { time_from_micros(micros) }.expect("in range");
            assert_eq!(unsafe { time_to_micros(time) }, micros, "for {micros} us");
        }
        // SAFETY: dispatch table initialised above.
        let end_of_day = unsafe { time_from_micros(86_399_999_999) }.expect("in range");
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
            let decoded = time_tz_from_bits(bits).expect("in range");
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
                let decoded = time_tz_from_bits(bits).expect("in range");
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

    // ─── The checked f64 conversions mirror DuckDB's failure conditions ─────

    #[test]
    fn try_f64_to_hugeint_refuses_exactly_what_duckdb_zeroes() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // Exact literals, not `2f64.powi(127)`: Miri perturbs `powi`'s result.
        let two_127 = 170_141_183_460_469_231_731_687_303_715_884_105_728.0_f64;
        let below = f64::from_bits(two_127.to_bits() - 1); // largest double < 2^127
                                                           // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            for refused in [
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                two_127,
                -two_127,
                1e40,
            ] {
                assert_eq!(try_f64_to_hugeint(refused), None, "{refused}");
                // DuckDB itself fails these: it answers 0.
                assert_eq!(f64_to_hugeint(refused), 0, "{refused}");
            }
            for accepted in [below, -below, 42.9, -42.9, 0.4, 0.0] {
                assert_eq!(
                    try_f64_to_hugeint(accepted),
                    Some(f64_to_hugeint(accepted)),
                    "{accepted}"
                );
            }
            assert_ne!(f64_to_hugeint(below), 0);
            assert_eq!(try_f64_to_hugeint(-42.9), Some(-42));
            assert_eq!(try_f64_to_hugeint(0.4), Some(0));
        }
    }

    #[test]
    fn try_f64_to_uhugeint_refuses_exactly_what_duckdb_zeroes() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // Exact literals, not `2f64.powi(128)`: Miri perturbs `powi`'s result.
        let two_128 = 340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64;
        let below = f64::from_bits(two_128.to_bits() - 1);
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            for refused in [f64::NAN, f64::INFINITY, -1.0, -0.5, two_128] {
                assert_eq!(try_f64_to_uhugeint(refused), None, "{refused}");
                assert_eq!(f64_to_uhugeint(refused), 0, "{refused}");
            }
            for accepted in [below, 42.9, 0.0, -0.0] {
                assert_eq!(
                    try_f64_to_uhugeint(accepted),
                    Some(f64_to_uhugeint(accepted)),
                    "{accepted}"
                );
            }
            assert_ne!(f64_to_uhugeint(below), 0);
            assert_eq!(try_f64_to_uhugeint(42.9), Some(42));
        }
    }

    #[test]
    fn try_f64_to_decimal_is_none_where_duckdb_returns_width_zero() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            for (value, width, scale) in [
                (1e30, 18, 2),
                (f64::NAN, 18, 2),
                (f64::INFINITY, 18, 2),
                (1.5, 40, 2),
                (1.5, 4, 5),
                (1000.0, 4, 1),
            ] {
                assert_eq!(
                    try_f64_to_decimal(value, width, scale),
                    None,
                    "{value} {width} {scale}"
                );
                assert_eq!(
                    f64_to_decimal(value, width, scale).width,
                    0,
                    "{value} {width} {scale}"
                );
            }
            assert_eq!(try_f64_to_decimal(0.4, 0, 0), None);
            let ok = try_f64_to_decimal(999.9, 4, 1).expect("fits DECIMAL(4,1)");
            assert_eq!((ok.width, ok.scale, ok.value), (4, 1, 9_999));
            let zero = try_f64_to_decimal(0.0, 18, 3).expect("zero fits");
            assert_eq!((zero.width, zero.scale, zero.value), (18, 3, 0));
            assert_eq!(
                decimal_to_f64(Decimal {
                    width: 0,
                    scale: 0,
                    value: 0
                }),
                None
            );
        }
    }
}
