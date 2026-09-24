// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `DuckDB` `INTERVAL` type conversion utilities.
//!
//! A `DuckDB` `INTERVAL` is a 16-byte struct with three fields:
//! ```text
//! { months: i32, days: i32, micros: i64 }
//! ```
//!
//! Converting to a uniform unit (microseconds) requires careful arithmetic to
//! avoid integer overflow. This module provides checked and saturating conversions.
//!
//! # Pitfall P8: Undocumented INTERVAL layout
//!
//! The `duckdb_string_t` and `INTERVAL` struct layouts are not documented in the
//! Rust bindings (`libduckdb-sys`). They must be inferred from `DuckDB`'s C headers.
//! This module encodes that knowledge so extension authors never need to look it up.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::interval::{interval_to_micros, DuckInterval};
//!
//! let iv = DuckInterval { months: 1, days: 0, micros: 0 };
//! // 1 month ≈ 30 days = 2_592_000_000_000 microseconds
//! assert_eq!(interval_to_micros(iv), Some(2_592_000_000_000_i64));
//! ```

/// Microseconds per day, used for interval conversion.
pub const MICROS_PER_DAY: i64 = 86_400 * 1_000_000;

/// Microseconds per month, approximated as 30 days — `DuckDB`'s
/// `Interval::MICROS_PER_MONTH`, used for comparing intervals (see
/// [`interval_to_micros`] for where `DuckDB` does and does not use it).
pub const MICROS_PER_MONTH: i64 = 30 * MICROS_PER_DAY;

/// A `DuckDB` `INTERVAL` value, matching the C struct layout exactly.
///
/// # Memory layout
///
/// ```text
/// offset 0:  months (i32)  — number of calendar months
/// offset 4:  days   (i32)  — number of calendar days
/// offset 8:  micros (i64)  — microseconds component
/// total:     16 bytes
/// ```
///
/// # Equality is field-by-field, not SQL equality
///
/// The derived `PartialEq`, `Eq` and `Hash` compare the three fields, so
/// `{ months: 1, .. }` and `{ days: 30, .. }` are different values here while
/// `DuckDB` says `interval '1 month' = interval '30 days'`. Comparing
/// [`to_micros`][Self::to_micros] matches SQL when the three fields share a
/// sign; see [`interval_to_micros`] for the mixed-sign case where it does not.
///
/// # Safety
///
/// This struct must remain `#[repr(C)]` with the exact field order above,
/// matching `DuckDB`'s `duckdb_interval` C struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct DuckInterval {
    /// Calendar months component.
    pub months: i32,
    /// Calendar days component.
    pub days: i32,
    /// Sub-day microseconds component.
    pub micros: i64,
}

impl DuckInterval {
    /// Returns a zero-valued interval (0 months, 0 days, 0 microseconds).
    #[inline]
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            months: 0,
            days: 0,
            micros: 0,
        }
    }

    /// Converts this interval to total microseconds with overflow checking.
    ///
    /// Returns `None` if the result would overflow `i64`.
    ///
    /// Month conversion uses 30 days/month; see [`interval_to_micros`] for
    /// where that matches `DuckDB` and where it does not.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::interval::DuckInterval;
    ///
    /// let iv = DuckInterval { months: 0, days: 1, micros: 500_000 };
    /// assert_eq!(iv.to_micros(), Some(86_400_500_000_i64));
    /// ```
    #[inline]
    #[must_use]
    pub fn to_micros(self) -> Option<i64> {
        interval_to_micros(self)
    }

    /// Converts this interval to total microseconds, saturating on overflow.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::interval::DuckInterval;
    ///
    /// let iv = DuckInterval { months: i32::MAX, days: i32::MAX, micros: i64::MAX };
    /// assert_eq!(iv.to_micros_saturating(), i64::MAX);
    /// ```
    #[inline]
    #[must_use]
    pub fn to_micros_saturating(self) -> i64 {
        interval_to_micros_saturating(self)
    }
}

impl Default for DuckInterval {
    #[inline]
    fn default() -> Self {
        Self::zero()
    }
}

/// Converts a [`DuckInterval`] to total microseconds with overflow checking.
///
/// Uses the approximation: **1 month = 30 days**. `DuckDB` uses the same
/// approximation in exactly two places (checked on `DuckDB` 1.4.4 and 1.5.5):
///
/// - **Comparing intervals** — `interval '1 month' = interval '30 days'` is
///   `true` (`Interval::DAYS_PER_MONTH = 30` in `interval.hpp`). `DuckDB`
///   normalises the fields (`interval_t::Normalize`: micros carry into days,
///   days into months, with truncating division) and compares them in order, so
///   this agrees with comparing total microseconds only when the fields do
///   not mix signs: `interval '1 month' - interval '1 day'` equals
///   `interval '29 days'` in microseconds but compares **greater** in SQL.
/// - **`epoch_us(interval)`** — returns exactly what this function returns;
///   `epoch_us(interval '1 year')` is 360 days.
///
/// It is **not** what `DuckDB` does elsewhere: `interval + interval` keeps
/// months, days and micros separate (`1 month 30 days`), adding an interval to
/// a date uses calendar months (`2024-01-31 + 1 month` is `2024-02-29`), and
/// `epoch(interval '1 year')` counts 365.25 days. Use this conversion for
/// ordering or bucketing intervals, not for date arithmetic.
///
/// # Returns
///
/// `None` if any intermediate multiplication or addition overflows `i64`.
///
/// # Example
///
/// ```rust
/// use quack_rs::interval::{interval_to_micros, DuckInterval};
///
/// // 2 hours 30 minutes = 9_000_000_000 microseconds
/// let iv = DuckInterval { months: 0, days: 0, micros: 9_000_000_000 };
/// assert_eq!(interval_to_micros(iv), Some(9_000_000_000_i64));
///
/// // 1 day
/// let iv = DuckInterval { months: 0, days: 1, micros: 0 };
/// assert_eq!(interval_to_micros(iv), Some(86_400_000_000_i64));
///
/// // Overflow returns None
/// let iv = DuckInterval { months: i32::MAX, days: i32::MAX, micros: i64::MAX };
/// assert_eq!(interval_to_micros(iv), None);
/// ```
#[inline]
pub fn interval_to_micros(iv: DuckInterval) -> Option<i64> {
    let months_us = i64::from(iv.months).checked_mul(MICROS_PER_MONTH)?;
    let days_us = i64::from(iv.days).checked_mul(MICROS_PER_DAY)?;
    months_us.checked_add(days_us)?.checked_add(iv.micros)
}

/// Converts a [`DuckInterval`] to total microseconds, saturating on overflow.
///
/// Uses the approximation: **1 month = 30 days**.
///
/// # Example
///
/// ```rust
/// use quack_rs::interval::{interval_to_micros_saturating, DuckInterval};
///
/// let iv = DuckInterval { months: 0, days: 0, micros: 1_000_000 };
/// assert_eq!(interval_to_micros_saturating(iv), 1_000_000_i64);
/// ```
#[inline]
pub fn interval_to_micros_saturating(iv: DuckInterval) -> i64 {
    interval_to_micros(iv).unwrap_or_else(|| {
        // Determine sign of the true (overflowed) result using i128 arithmetic.
        // This correctly handles mixed-sign cases where some components are
        // positive and others are negative.
        let months_us = i128::from(iv.months) * i128::from(MICROS_PER_MONTH);
        let days_us = i128::from(iv.days) * i128::from(MICROS_PER_DAY);
        let total = months_us + days_us + i128::from(iv.micros);
        if total >= 0 {
            i64::MAX
        } else {
            i64::MIN
        }
    })
}

/// Reads a [`DuckInterval`] from a raw `DuckDB` vector data pointer at a given row index.
///
/// # Safety
///
/// - `data` must be a valid pointer to a `DuckDB` vector's data buffer containing
///   `INTERVAL` values (16 bytes each).
/// - `idx` must be within bounds of the vector.
///
/// # Pitfall P8
///
/// The `INTERVAL` struct is 16 bytes: `{ months: i32, days: i32, micros: i64 }`.
/// This layout matches `duckdb_interval` in `DuckDB`'s C headers.
///
/// # Example
///
/// ```rust
/// use quack_rs::interval::{read_interval_at, DuckInterval};
///
/// let ivs = [DuckInterval { months: 2, days: 15, micros: 1_000 }];
/// let data = ivs.as_ptr() as *const u8;
/// let read = unsafe { read_interval_at(data, 0) };
/// assert_eq!(read.months, 2);
/// assert_eq!(read.days, 15);
/// assert_eq!(read.micros, 1_000);
/// ```
#[inline]
pub const unsafe fn read_interval_at(data: *const u8, idx: usize) -> DuckInterval {
    // SAFETY: Each INTERVAL is exactly 16 bytes (repr(C) struct with i32, i32, i64).
    // The caller guarantees `data` points to valid INTERVAL data and `idx` is in bounds.
    let ptr = unsafe { data.add(idx * 16) };
    // SAFETY: `ptr` is the start of the 16-byte INTERVAL at row `idx` inside the buffer
    // `data` points to (both `# Safety` clauses), so `ptr + 0` through `+ 4` are
    // initialised bytes of that element (`months: i32`); `read_unaligned` needs no
    // alignment, only readable memory.
    let months = unsafe { core::ptr::read_unaligned(ptr.cast::<i32>()) };
    // SAFETY: `ptr` is the start of the 16-byte INTERVAL at row `idx` inside the buffer
    // `data` points to (both `# Safety` clauses), so `ptr + 4` through `+ 8` are
    // initialised bytes of that element (`days: i32`); `read_unaligned` needs no alignment,
    // only readable memory.
    let days = unsafe { core::ptr::read_unaligned(ptr.add(4).cast::<i32>()) };
    // SAFETY: `ptr` is the start of the 16-byte INTERVAL at row `idx` inside the buffer
    // `data` points to (both `# Safety` clauses), so `ptr + 8` through `+ 16` are
    // initialised bytes of that element (`micros: i64`); `read_unaligned` needs no
    // alignment, only readable memory.
    let micros = unsafe { core::ptr::read_unaligned(ptr.add(8).cast::<i64>()) };
    DuckInterval {
        months,
        days,
        micros,
    }
}

/// Asserts the size and alignment of [`DuckInterval`] match `DuckDB`'s C struct.
const _: () = {
    assert!(
        core::mem::size_of::<DuckInterval>() == 16,
        "DuckInterval must be exactly 16 bytes"
    );
    assert!(
        core::mem::align_of::<DuckInterval>() >= 4,
        "DuckInterval must have at least 4-byte alignment"
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_of_duck_interval() {
        assert_eq!(core::mem::size_of::<DuckInterval>(), 16);
    }

    #[test]
    fn zero_interval() {
        let iv = DuckInterval::zero();
        assert_eq!(interval_to_micros(iv), Some(0));
    }

    #[test]
    fn default_interval() {
        let iv = DuckInterval::default();
        assert_eq!(iv, DuckInterval::zero());
    }

    #[test]
    fn one_day() {
        let iv = DuckInterval {
            months: 0,
            days: 1,
            micros: 0,
        };
        assert_eq!(interval_to_micros(iv), Some(MICROS_PER_DAY));
    }

    #[test]
    fn one_month() {
        let iv = DuckInterval {
            months: 1,
            days: 0,
            micros: 0,
        };
        assert_eq!(interval_to_micros(iv), Some(MICROS_PER_MONTH));
    }

    #[test]
    fn combined_interval() {
        let iv = DuckInterval {
            months: 0,
            days: 1,
            micros: 500_000,
        };
        let expected = MICROS_PER_DAY + 500_000;
        assert_eq!(interval_to_micros(iv), Some(expected));
    }

    #[test]
    fn negative_interval() {
        let iv = DuckInterval {
            months: -1,
            days: 0,
            micros: 0,
        };
        assert_eq!(interval_to_micros(iv), Some(-MICROS_PER_MONTH));
    }

    #[test]
    fn overflow_returns_none() {
        let iv = DuckInterval {
            months: i32::MAX,
            days: i32::MAX,
            micros: i64::MAX,
        };
        assert_eq!(interval_to_micros(iv), None);
    }

    #[test]
    fn saturating_on_overflow() {
        let iv = DuckInterval {
            months: i32::MAX,
            days: i32::MAX,
            micros: i64::MAX,
        };
        assert_eq!(interval_to_micros_saturating(iv), i64::MAX);
    }

    #[test]
    fn saturating_no_overflow() {
        let iv = DuckInterval {
            months: 0,
            days: 0,
            micros: 42,
        };
        assert_eq!(interval_to_micros_saturating(iv), 42);
    }

    #[test]
    fn to_micros_method() {
        let iv = DuckInterval {
            months: 0,
            days: 0,
            micros: 12345,
        };
        assert_eq!(iv.to_micros(), Some(12345));
    }

    #[test]
    fn to_micros_saturating_method() {
        let iv = DuckInterval {
            months: 0,
            days: 0,
            micros: 12345,
        };
        assert_eq!(iv.to_micros_saturating(), 12345);
    }

    #[test]
    fn read_interval_at_basic() {
        let data = [
            DuckInterval {
                months: 2,
                days: 15,
                micros: 999_000,
            },
            DuckInterval {
                months: -1,
                days: 3,
                micros: 0,
            },
        ];
        // SAFETY: data is a valid array of DuckInterval, idx 0 and 1 are in bounds.
        let iv0 = unsafe { read_interval_at(data.as_ptr().cast::<u8>(), 0) };
        assert_eq!(iv0.months, 2);
        assert_eq!(iv0.days, 15);
        assert_eq!(iv0.micros, 999_000);

        let iv1 = unsafe { read_interval_at(data.as_ptr().cast::<u8>(), 1) };
        assert_eq!(iv1.months, -1);
        assert_eq!(iv1.days, 3);
        assert_eq!(iv1.micros, 0);
    }

    #[test]
    fn exactly_max_i64_micros_no_overflow() {
        // If all overflow is in micros only (months=0, days=0), no overflow
        let iv = DuckInterval {
            months: 0,
            days: 0,
            micros: i64::MAX,
        };
        assert_eq!(interval_to_micros(iv), Some(i64::MAX));
    }

    #[test]
    fn months_calculation() {
        // 12 months = 12 * 30 days * 86400 * 1_000_000 us
        let iv = DuckInterval {
            months: 12,
            days: 0,
            micros: 0,
        };
        let expected = 12_i64 * MICROS_PER_MONTH;
        assert_eq!(interval_to_micros(iv), Some(expected));
    }

    /// `DuckDB`'s `Interval::MICROS_PER_MONTH` is 30 days: `epoch_us(interval
    /// '1 month')` is `2_592_000_000_000`. Spelled out as a literal so the
    /// constant is checked, not reused.
    #[test]
    fn one_month_is_thirty_days_of_micros() {
        assert_eq!(MICROS_PER_MONTH, 2_592_000_000_000);
        let iv = DuckInterval {
            months: 1,
            days: 0,
            micros: 0,
        };
        assert_eq!(interval_to_micros(iv), Some(2_592_000_000_000));
        assert_eq!(
            interval_to_micros(iv),
            interval_to_micros(DuckInterval {
                months: 0,
                days: 30,
                micros: 0,
            })
        );
    }

    // Mixed-sign overflow saturation tests (CRIT-5 regression tests)

    /// When only the final micros addition overflows, the true total has the
    /// sign of the micros, even though the days component is far smaller in
    /// magnitude: one day plus `i64::MAX` micros saturates up, minus one day
    /// plus `i64::MIN` micros saturates down.
    #[test]
    fn saturation_direction_follows_micros_when_micros_overflow() {
        let up = DuckInterval {
            months: 0,
            days: 1,
            micros: i64::MAX,
        };
        assert_eq!(interval_to_micros(up), None);
        assert_eq!(interval_to_micros_saturating(up), i64::MAX);

        let down = DuckInterval {
            months: 0,
            days: -1,
            micros: i64::MIN,
        };
        assert_eq!(interval_to_micros(down), None);
        assert_eq!(interval_to_micros_saturating(down), i64::MIN);
    }

    #[test]
    fn saturating_positive_overflow_with_negative_days() {
        // months = i32::MAX overflows to massive positive; days = -1 is tiny negative.
        // True result is still massively positive → should saturate to i64::MAX.
        let iv = DuckInterval {
            months: i32::MAX,
            days: -1,
            micros: 0,
        };
        assert_eq!(interval_to_micros(iv), None); // confirm it overflows
        assert_eq!(interval_to_micros_saturating(iv), i64::MAX);
    }

    #[test]
    fn saturating_negative_overflow_with_positive_days() {
        // months = i32::MIN overflows to massive negative; days = 1 is tiny positive.
        // True result is still massively negative → should saturate to i64::MIN.
        let iv = DuckInterval {
            months: i32::MIN,
            days: 1,
            micros: 0,
        };
        assert_eq!(interval_to_micros(iv), None);
        assert_eq!(interval_to_micros_saturating(iv), i64::MIN);
    }

    #[test]
    fn saturating_positive_overflow_negative_micros() {
        // Months alone overflow positive; negative micros doesn't change sign.
        let iv = DuckInterval {
            months: i32::MAX,
            days: 0,
            micros: -1_000_000,
        };
        assert_eq!(interval_to_micros(iv), None);
        assert_eq!(interval_to_micros_saturating(iv), i64::MAX);
    }

    /// The days term alone decides the direction: it overflows, outweighs
    /// months and micros of the other sign, and is itself far past `i64`.
    /// Only the randomized `saturating_direction_matches_i128` reached this
    /// case before, so CI's mutation job caught `days * MICROS_PER_DAY` ->
    /// `days + MICROS_PER_DAY` on some runs and not others.
    #[test]
    fn saturation_direction_follows_days_when_days_dominate() {
        let down = DuckInterval {
            months: 1_000_000,
            days: i32::MIN,
            micros: i64::MAX,
        };
        assert_eq!(interval_to_micros(down), None);
        assert_eq!(interval_to_micros_saturating(down), i64::MIN);

        let up = DuckInterval {
            months: -1_000_000,
            days: i32::MAX,
            micros: i64::MIN,
        };
        assert_eq!(interval_to_micros(up), None);
        assert_eq!(interval_to_micros_saturating(up), i64::MAX);
    }

    #[test]
    fn saturating_negative_overflow_all_negative() {
        let iv = DuckInterval {
            months: i32::MIN,
            days: i32::MIN,
            micros: i64::MIN,
        };
        assert_eq!(interval_to_micros(iv), None);
        assert_eq!(interval_to_micros_saturating(iv), i64::MIN);
    }

    mod proptest_interval {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn micros_only_never_overflows_within_i64(micros: i64) {
                let iv = DuckInterval { months: 0, days: 0, micros };
                // micros-only interval always succeeds (no multiplication needed)
                assert_eq!(interval_to_micros(iv), Some(micros));
            }

            #[test]
            fn saturating_never_panics(months: i32, days: i32, micros: i64) {
                let iv = DuckInterval { months, days, micros };
                // Must not panic for any input
                let _ = interval_to_micros_saturating(iv);
            }

            #[test]
            fn saturating_direction_matches_i128(months: i32, days: i32, micros: i64) {
                let iv = DuckInterval { months, days, micros };
                let sat = interval_to_micros_saturating(iv);
                if interval_to_micros(iv).is_none() {
                    // Verify saturation direction using i128 ground truth
                    let total = i128::from(months) * i128::from(MICROS_PER_MONTH)
                        + i128::from(days) * i128::from(MICROS_PER_DAY)
                        + i128::from(micros);
                    if total >= 0 {
                        prop_assert_eq!(sat, i64::MAX);
                    } else {
                        prop_assert_eq!(sat, i64::MIN);
                    }
                }
            }

            #[test]
            fn checked_and_saturating_agree_when_no_overflow(months in -100_i32..=100_i32, days in -100_i32..=100_i32, micros in -1_000_000_i64..=1_000_000_i64) {
                let iv = DuckInterval { months, days, micros };
                if let Some(checked) = interval_to_micros(iv) {
                    assert_eq!(interval_to_micros_saturating(iv), checked);
                }
            }
        }
    }
}
