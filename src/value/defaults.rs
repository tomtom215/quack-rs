// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The defaulting accessors — `Value::as_*_or`.
//!
//! Each one answers the question "what should this be when the handle is
//! null?", which is pure branching on [`Value::is_null`] and never touches
//! `DuckDB` on the default path. That is why they live here rather than in
//! `value.rs`: `mutants.toml` excludes that file wholesale — every function
//! left in it wraps a `DuckDB` C call the `--lib` mutation run cannot reach —
//! and these fourteen can be, and are, killed by ordinary unit tests.

use super::Value;

impl Value {
    /// Extracts the value as a `String`, returning `default` on failure.
    ///
    /// Convenience for `val.as_str().unwrap_or_else(|_| default.to_owned())`.
    #[inline]
    #[must_use]
    pub fn as_str_or(&self, default: &str) -> String {
        self.as_str().unwrap_or_else(|_| default.to_owned())
    }

    /// Extracts the value as a `String`, returning an empty string on failure.
    ///
    /// Convenience for `val.as_str().unwrap_or_default()`.
    #[inline]
    #[must_use]
    // The `-> String with String::new()` mutant is indistinguishable from the
    // real function for every input a unit test can build: on a null handle
    // both give `""`, and producing a non-null VARCHAR needs a live engine,
    // which the `--cargo-arg=--lib` mutation run does not have. The end-to-end
    // suite covers the non-empty case.
    #[mutants::skip]
    pub fn as_str_or_default(&self) -> String {
        self.as_str().unwrap_or_default()
    }

    /// Extracts the value as an `i32`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i32_or(&self, default: i32) -> i32 {
        if self.is_null() {
            default
        } else {
            self.as_i32()
        }
    }

    /// Extracts the value as an `i64`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i64_or(&self, default: i64) -> i64 {
        if self.is_null() {
            default
        } else {
            self.as_i64()
        }
    }

    /// Extracts the value as an `f32`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_f32_or(&self, default: f32) -> f32 {
        if self.is_null() {
            default
        } else {
            self.as_f32()
        }
    }

    /// Extracts the value as an `f64`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_f64_or(&self, default: f64) -> f64 {
        if self.is_null() {
            default
        } else {
            self.as_f64()
        }
    }

    /// Extracts the value as a `bool`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_bool_or(&self, default: bool) -> bool {
        if self.is_null() {
            default
        } else {
            self.as_bool()
        }
    }

    /// Extracts the value as an `i8`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i8_or(&self, default: i8) -> i8 {
        if self.is_null() {
            default
        } else {
            self.as_i8()
        }
    }

    /// Extracts the value as an `i16`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i16_or(&self, default: i16) -> i16 {
        if self.is_null() {
            default
        } else {
            self.as_i16()
        }
    }

    /// Extracts the value as a `u8`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u8_or(&self, default: u8) -> u8 {
        if self.is_null() {
            default
        } else {
            self.as_u8()
        }
    }

    /// Extracts the value as a `u16`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u16_or(&self, default: u16) -> u16 {
        if self.is_null() {
            default
        } else {
            self.as_u16()
        }
    }

    /// Extracts the value as a `u32`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u32_or(&self, default: u32) -> u32 {
        if self.is_null() {
            default
        } else {
            self.as_u32()
        }
    }

    /// Extracts the value as a `u64`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u64_or(&self, default: u64) -> u64 {
        if self.is_null() {
            default
        } else {
            self.as_u64()
        }
    }

    /// Extracts the value as an `i128`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i128_or(&self, default: i128) -> i128 {
        if self.is_null() {
            default
        } else {
            self.as_i128()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    // Every one of these builds a null handle, which is the branch that
    // does not call into DuckDB — the only branch a `--lib` run can take.
    // Each default is chosen so none of `0`, `1` or `-1` can impersonate it.

    #[test]
    fn as_str_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_str_or("fallback"), "fallback");
    }

    #[test]
    fn as_str_or_default_returns_empty_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_str_or_default(), "");
    }

    #[test]
    fn as_i64_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_i64_or(99), 99);
    }

    #[test]
    fn as_i32_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_i32_or(42), 42);
    }

    #[test]
    fn as_bool_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!(val.as_bool_or(true));
        assert!(!val.as_bool_or(false));
    }

    #[test]
    fn as_f64_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!((val.as_f64_or(2.72) - 2.72).abs() < f64::EPSILON);
    }

    #[test]
    fn as_f32_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!((val.as_f32_or(2.5) - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn as_i8_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_i8_or(7), 7);
    }

    #[test]
    fn as_i16_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_i16_or(-300), -300);
    }

    #[test]
    fn as_u8_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_u8_or(200), 200);
    }

    #[test]
    fn as_u16_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_u16_or(40_000), 40_000);
    }

    #[test]
    fn as_u32_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_u32_or(3_000_000_000), 3_000_000_000);
    }

    #[test]
    fn as_u64_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_u64_or(u64::MAX - 1), u64::MAX - 1);
    }

    #[test]
    fn as_i128_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_i128_or(i128::MIN + 1), i128::MIN + 1);
    }
}
