// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Randomised cross-checks of `quack_rs::datetime` and `quack_rs::interval`
//! with the linked `DuckDB`'s own SQL as the oracle.
//!
//! The existing temporal tests pin a handful of hand-picked values. These draw
//! a few thousand from a fixed-seed generator (the seed is a constant, so a
//! failure reproduces) across the whole range each type has, plus the edges,
//! and compare what the Rust wrappers say with what `DuckDB` says about the
//! same value: decomposition (`date_from_days`, `timestamp_from_micros`),
//! composition and its refusals (`date_to_days`), and the documented relation
//! between `interval_to_micros` and `epoch_us(interval)`.
//!
//! Needs `TRY(..)`, so `DuckDB` 1.3 or later (the live suite starts at 1.4.4).

use quack_rs::datetime::{self, Date, Time, Timestamp};
use quack_rs::interval::{interval_to_micros, DuckInterval};

use super::vector_support::text_rows;
use super::Fixture;

/// `SplitMix64`: tiny, fixed-seed, good enough to spread values over a range.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`.
    fn between(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (i128::from(hi) - i128::from(lo) + 1) as u128;
        let offset = (u128::from(self.next()) * span) >> 64;
        i64::try_from(i128::from(lo) + offset as i128).expect("within lo..=hi")
    }
}

/// `DuckDB` writes the astronomical year `y <= 0` as `1 - y` followed by ` (BC)`.
fn year_text(year: i32) -> (String, &'static str) {
    if year <= 0 {
        (format!("{:04}", 1 - i64::from(year)), " (BC)")
    } else {
        (format!("{year:04}"), "")
    }
}

fn render_date(d: Date) -> String {
    let (year, era) = year_text(d.year);
    format!("{year}-{:02}-{:02}{era}", d.month, d.day)
}

fn render_time(t: Time) -> String {
    let mut out = format!("{:02}:{:02}:{:02}", t.hour, t.min, t.sec);
    if t.micros != 0 {
        out.push_str(format!(".{:06}", t.micros).trim_end_matches('0'));
    }
    out
}

fn render_timestamp(ts: Timestamp) -> String {
    let (year, era) = year_text(ts.date.year);
    format!(
        "{year}-{:02}-{:02}{era} {}",
        ts.date.month,
        ts.date.day,
        render_time(ts.time)
    )
}

/// `(index, text columns)` rows of `sql`, in index order. The first selected
/// column must be the row index.
fn indexed_rows(fx: &Fixture, sql: &str) -> Vec<Vec<Option<String>>> {
    let mut rows = text_rows(fx, sql);
    rows.sort_by_key(|r| {
        r[0].as_deref()
            .expect("index")
            .parse::<usize>()
            .expect("number")
    });
    rows
}

#[test]
fn date_decomposition_agrees_with_duckdb_for_random_days() {
    let fx = Fixture::open();
    let mut rng = Rng(0x00DA_7E00_0001);
    // DuckDB's DATE range is +-(2^31 - 2) days; the two extremes beyond it are
    // the infinity sentinels.
    let mut days: Vec<i32> = vec![
        0,
        1,
        -1,
        58,
        59,
        60,
        365,
        366,
        10_957,
        -719_162,
        -719_163,
        2_932_896,
        2_147_483_646,
        -2_147_483_646,
    ];
    while days.len() < 3_000 {
        let d = rng.between(-2_147_483_646, 2_147_483_646);
        days.push(i32::try_from(d).expect("in range"));
    }
    // A dense run around the Gregorian cycle boundaries, where a leap-year
    // mistake would show.
    for d in -800_000..-799_000 {
        days.push(d * 3);
    }
    let values: Vec<String> = days
        .iter()
        .enumerate()
        .map(|(i, d)| format!("({i}, {d})"))
        .collect();
    let rows = indexed_rows(
        &fx,
        &format!(
            "SELECT i::VARCHAR, (DATE '1970-01-01' + d)::VARCHAR FROM (VALUES {}) t(i, d)",
            values.join(", ")
        ),
    );
    assert_eq!(rows.len(), days.len());
    for (row, &d) in rows.iter().zip(&days) {
        let expected = row[1].as_deref().expect("a DATE in range renders");
        // SAFETY: the fixture initialised the dispatch table.
        let (decoded, back) = unsafe {
            let decoded = datetime::date_from_days(d);
            (decoded, datetime::date_to_days(decoded))
        };
        assert_eq!(render_date(decoded), expected, "day {d}");
        assert_eq!(back, Some(d), "day {d} does not survive a round trip");
    }
}

#[test]
fn date_composition_accepts_and_refuses_exactly_what_duckdb_does() {
    let fx = Fixture::open();
    let mut rng = Rng(0x00DA_7E00_0002);
    let mut triples: Vec<(i32, i32, i32)> = vec![
        (2024, 2, 29),
        (2023, 2, 29),
        (1900, 2, 29),
        (2000, 2, 29),
        (-4, 2, 29),
        (0, 1, 1),
        (0, 12, 31),
        (-5_877_641, 6, 25),
        (-5_877_641, 6, 24),
        (-5_877_642, 12, 31),
        (5_881_580, 7, 10),
        (5_881_580, 7, 11),
        (5_881_581, 1, 1),
        (1, 0, 1),
        (1, 13, 1),
        (1, 1, 0),
        (1, 1, 32),
    ];
    while triples.len() < 4_000 {
        let year = match rng.between(0, 3) {
            0 => rng.between(-6_000_000, 6_000_000),
            1 => rng.between(1_500, 2_500),
            2 => rng.between(-5_877_645, -5_877_636),
            _ => rng.between(5_881_575, 5_881_585),
        };
        let month = rng.between(0, 13);
        let day = rng.between(0, 32);
        triples.push((
            i32::try_from(year).expect("in range"),
            i32::try_from(month).expect("in range"),
            i32::try_from(day).expect("in range"),
        ));
    }
    let values: Vec<String> = triples
        .iter()
        .enumerate()
        .map(|(i, (y, m, d))| format!("({i}, {y}, {m}, {d})"))
        .collect();
    let rows = indexed_rows(
        &fx,
        &format!(
            "SELECT i::VARCHAR, TRY(make_date(y, m, d) - DATE '1970-01-01')::VARCHAR \
             FROM (VALUES {}) t(i, y, m, d)",
            values.join(", ")
        ),
    );
    assert_eq!(rows.len(), triples.len());
    let (mut accepted, mut refused) = (0_u32, 0_u32);
    for (row, &(year, month, day)) in rows.iter().zip(&triples) {
        let date = Date {
            year,
            month: i8::try_from(month).expect("0..=13"),
            day: i8::try_from(day).expect("0..=32"),
        };
        // SAFETY: the fixture initialised the dispatch table.
        let got = unsafe { datetime::date_to_days(date) };
        let want = row[1].as_deref().map(|s| s.parse::<i32>().expect("days"));
        assert_eq!(got, want, "{year}-{month}-{day}");
        assert_eq!(
            datetime::is_valid_date(date),
            want.is_some(),
            "{year}-{month}-{day}"
        );
        if want.is_some() {
            accepted += 1;
        } else {
            refused += 1;
        }
    }
    // Both arms must have been exercised, or the comparison proves nothing.
    assert!(
        accepted > 500 && refused > 500,
        "{accepted} accepted, {refused} refused"
    );
}

#[test]
fn timestamp_decomposition_agrees_with_duckdb_for_random_microseconds() {
    let fx = Fixture::open();
    let mut rng = Rng(0x00DA_7E00_0003);
    let mut micros: Vec<i64> = vec![
        0,
        1,
        -1,
        86_399_999_999,
        86_400_000_000,
        -86_400_000_000,
        -86_400_000_001,
        i64::MAX,
        i64::MAX - 1,
        -i64::MAX,
        -i64::MAX + 1,
        i64::MIN,
        i64::MIN + 1,
        // The first day whose floor-divided day number overflows when
        // multiplied back (the mirror in `datetime::checks`).
        -106_751_991 * 86_400_000_000,
        -106_751_991 * 86_400_000_000 - 1,
        -106_751_991 * 86_400_000_000 + 1,
        // The documented range ends: 294247-01-10 04:00:54.775807, and the
        // earliest, 290309-12-22 (BC) 00:00:00.
        9_223_371_331_200_000_000,
        9_223_372_036_854_775_806,
        -9_223_372_022_400_000_000,
    ];
    while micros.len() < 4_000 {
        let m = match rng.between(0, 2) {
            0 => {
                i64::try_from(rng.next() >> 1).expect("63 bits")
                    * if rng.next() & 1 == 0 { 1 } else { -1 }
            }
            1 => rng.between(-4_000_000_000_000_000, 4_000_000_000_000_000),
            _ => rng.between(i64::MIN, i64::MAX),
        };
        micros.push(m);
    }
    // DuckDB 2.0.0 raises an INTERNAL error, which `TRY` does not catch, when it
    // renders a timestamp below the documented range (its `__cast` to VARCHAR
    // "threw an execution error, but the function is not marked as fallible");
    // 1.x answers NULL. Those values are not put to the engine on 2.0.0 and are
    // expected to be refused: the 1.x runs hold the engine to that answer.
    // `-i64::MAX` is the negative-infinity sentinel and answers NULL on both.
    let engine_throws =
        |m: i64| fx.engine_major() >= 2 && m < -9_223_372_022_400_000_000 && m != -i64::MAX;
    let values: Vec<String> = micros
        .iter()
        .enumerate()
        .filter(|(_, m)| !engine_throws(**m))
        .map(|(i, m)| format!("({i}, CAST('{m}' AS BIGINT))"))
        .collect();
    let mut answers = indexed_rows(
        &fx,
        &format!(
            "SELECT i::VARCHAR, TRY(CAST(make_timestamp(m) AS VARCHAR)) \
             FROM (VALUES {}) t(i, m)",
            values.join(", ")
        ),
    )
    .into_iter();
    let skipped = micros.iter().filter(|m| engine_throws(**m)).count();
    assert_eq!(answers.len() + skipped, micros.len());
    let (mut some, mut none) = (0_u32, 0_u32);
    for &m in &micros {
        let row = if engine_throws(m) {
            vec![None, None]
        } else {
            answers.next().expect("one answer per value asked")
        };
        // SAFETY: the fixture initialised the dispatch table.
        let decoded = unsafe { datetime::timestamp_from_micros(m) };
        match (&decoded, row[1].as_deref()) {
            (Some(ts), Some(expected)) => {
                assert_eq!(render_timestamp(*ts), expected, "{m} microseconds");
                // SAFETY: as above.
                let back = unsafe { datetime::timestamp_to_micros(*ts) };
                assert_eq!(
                    back,
                    Some(m),
                    "{m} microseconds does not survive a round trip"
                );
                some += 1;
            }
            (None, None) => none += 1,
            (got, want) => panic!(
                "{m} microseconds: the wrapper says {got:?}, DuckDB says {want:?} \
                 (None here means a refusal on both sides)"
            ),
        }
    }
    assert!(
        some > 1_000 && none >= 4,
        "{some} decomposed, {none} refused"
    );
}

#[test]
fn interval_to_micros_matches_epoch_us_whenever_duckdb_has_an_answer() {
    let fx = Fixture::open();
    let mut rng = Rng(0x00DA_7E00_0004);
    const PER_MONTH: i128 = 30 * 86_400_000_000;
    const PER_DAY: i128 = 86_400_000_000;
    let mut intervals: Vec<(i32, i32, i64)> = vec![
        (0, 0, 0),
        (1, 0, 0),
        (0, 1, 0),
        (0, 0, 1),
        (0, 0, i64::MAX),
        (0, 0, i64::MIN),
        (i32::MAX, i32::MAX, i64::MAX),
        (i32::MIN, i32::MIN, i64::MIN),
        (i32::MAX, 0, 0),
        (i32::MIN, 0, 0),
        (0, i32::MAX, 0),
        (0, i32::MIN, 0),
        // The documented case: each field fits on its own, the DuckDB sum may not.
        (0, 106_751_992, -86_400_000_000),
        (2_770_036, 31_299_404, -2_032_753_245_989_188_716),
    ];
    while intervals.len() < 4_000 {
        let months = match rng.between(0, 2) {
            0 => rng.between(-120, 120),
            1 => rng.between(-3_000_000, 3_000_000),
            _ => rng.between(i64::from(i32::MIN), i64::from(i32::MAX)),
        };
        let days = match rng.between(0, 2) {
            0 => rng.between(-400, 400),
            1 => rng.between(-110_000_000, 110_000_000),
            _ => rng.between(i64::from(i32::MIN), i64::from(i32::MAX)),
        };
        let mut micros = match rng.between(0, 2) {
            0 => rng.between(-1_000_000_000, 1_000_000_000),
            1 => rng.between(i64::MIN, i64::MAX),
            _ => 0,
        };
        // A third of the rows are aimed at the i64 boundary, one microsecond
        // either side of it, so the overflow decision is what is being tested.
        if rng.between(0, 2) == 0 {
            let fixed = i128::from(months) * PER_MONTH + i128::from(days) * PER_DAY;
            let edge = if rng.between(0, 1) == 0 {
                i128::from(i64::MAX)
            } else {
                i128::from(i64::MIN)
            };
            let aimed = edge - fixed + i128::from(rng.between(-2, 2));
            if let Ok(m) = i64::try_from(aimed) {
                micros = m;
            }
        }
        intervals.push((
            i32::try_from(months).expect("i32"),
            i32::try_from(days).expect("i32"),
            micros,
        ));
    }
    let values: Vec<String> = intervals
        .iter()
        .enumerate()
        .map(|(i, (m, d, u))| {
            format!("({i}, CAST('{m}' AS INTEGER), CAST('{d}' AS INTEGER), CAST('{u}' AS BIGINT))")
        })
        .collect();
    let rows = indexed_rows(
        &fx,
        &format!(
            "SELECT i::VARCHAR, TRY(epoch_us(to_months(m) + to_days(d) + to_microseconds(u)))::VARCHAR \
             FROM (VALUES {}) t(i, m, d, u)",
            values.join(", ")
        ),
    );
    assert_eq!(rows.len(), intervals.len());
    let (mut agreed, mut wrapper_only, mut both_refused) = (0_u32, 0_u32, 0_u32);
    for (row, &(months, days, micros)) in rows.iter().zip(&intervals) {
        let ours = interval_to_micros(DuckInterval {
            months,
            days,
            micros,
        });
        let exact =
            i128::from(months) * PER_MONTH + i128::from(days) * PER_DAY + i128::from(micros);
        assert_eq!(
            ours,
            i64::try_from(exact).ok(),
            "{months}m {days}d {micros}us: not the exact total"
        );
        match (ours, row[1].as_deref()) {
            // "returns exactly what this function returns whenever it returns a value"
            (Some(a), Some(b)) => {
                assert_eq!(a.to_string(), b, "{months}m {days}d {micros}us");
                agreed += 1;
            }
            // DuckDB adds one field at a time and may fail where the exact total fits.
            (Some(_), None) => wrapper_only += 1,
            (None, None) => both_refused += 1,
            (None, Some(b)) => panic!(
                "{months}m {days}d {micros}us: the exact total does not fit an i64 but \
                 DuckDB's epoch_us returned {b}"
            ),
        }
    }
    assert!(
        agreed > 1_000 && both_refused > 100,
        "{agreed} agreed, {both_refused} both refused"
    );
    eprintln!("interval oracle: {agreed} agreed, {wrapper_only} wrapper-only (documented), {both_refused} both refused");
}
