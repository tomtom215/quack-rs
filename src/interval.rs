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

/// Microseconds per month (approximated as 30 days, matching `DuckDB`'s behaviour).
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
    /// Month conversion uses 30 days/month, matching `DuckDB`'s approximation.
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
/// Uses the approximation: **1 month = 30 days**, which is what `DuckDB` uses
/// internally when comparing or arithmetically combining intervals.
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
    interval_to_micros(iv).unwrap_or({
        // Determine sign of overflow to saturate correctly
        if iv.months >= 0 && iv.days >= 0 && iv.micros >= 0 {
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
///
/// # Safety
#[inline]
pub const unsafe fn read_interval_at(data: *const u8, idx: usize) -> DuckInterval {
    // SAFETY: Each INTERVAL is exactly 16 bytes (repr(C) struct with i32, i32, i64).
    // The caller guarantees `data` points to valid INTERVAL data and `idx` is in bounds.
    let ptr = unsafe { data.add(idx * 16) };
    let months = unsafe { core::ptr::read_unaligned(ptr.cast::<i32>()) };
    let days = unsafe { core::ptr::read_unaligned(ptr.add(4).cast::<i32>()) };
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
            fn checked_and_saturating_agree_when_no_overflow(months in -100_i32..=100_i32, days in -100_i32..=100_i32, micros in -1_000_000_i64..=1_000_000_i64) {
                let iv = DuckInterval { months, days, micros };
                if let Some(checked) = interval_to_micros(iv) {
                    assert_eq!(interval_to_micros_saturating(iv), checked);
                }
            }
        }
    }
}
