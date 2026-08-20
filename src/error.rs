// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Error types for `DuckDB` extension FFI error propagation.
//!
//! [`ExtensionError`] is the primary error type. It implements [`std::error::Error`],
//! can be constructed from `&str` or `String`, and converts to a `CString` for
//! passing to `access.set_error`.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::error::ExtensionError;
//!
//! let err = ExtensionError::from("Failed to register function");
//! assert_eq!(err.to_string(), "Failed to register function");
//! ```

use std::ffi::CString;
use std::fmt;

/// The advice appended to every `duckdb_register_*_function failed` message.
///
/// The C API reports registration failure as a bare `DuckDBError` with no
/// reason attached — there is no `duckdb_..._error` for it — so the message
/// quack-rs can produce is only as useful as the list of causes it names. There
/// are three, and a name collision with a `DuckDB` built-in is by far the most
/// common: `list_sum`, `array_sum` and friends already exist, and the failure
/// looks identical to a type error.
pub(crate) const REGISTRATION_FAILURE_HINT: &str =
    "the C API reports no reason, and there are only three. The name may already \
     be taken — DuckDB built-ins like `list_sum` and `array_sum` collide silently, \
     so check `SELECT * FROM duckdb_functions() WHERE function_name = '<name>'`. \
     A parameter or return type may be invalid. Or a required callback was never set.";

/// An error that can occur during `DuckDB` extension initialization or registration.
///
/// This type is designed for use with the `?` operator inside the extension
/// entry point. It can be reported back to `DuckDB` via `access.set_error`.
///
/// # Construction
///
/// ```rust
/// use quack_rs::error::ExtensionError;
///
/// // From a string literal
/// let e = ExtensionError::from("something went wrong");
///
/// // From a String
/// let msg = format!("failed: {}", 42);
/// let e = ExtensionError::from(msg);
///
/// // Wrapping another error
/// let parse_err: Result<i32, _> = "not a number".parse();
/// let e = parse_err.map_err(ExtensionError::from_error);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionError {
    message: String,
}

impl ExtensionError {
    /// Creates a new `ExtensionError` with the given message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::error::ExtensionError;
    ///
    /// let err = ExtensionError::new("registration failed");
    /// assert_eq!(err.to_string(), "registration failed");
    /// ```
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Wraps any `std::error::Error` into an `ExtensionError`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::error::ExtensionError;
    ///
    /// let result: Result<i32, _> = "abc".parse::<i32>();
    /// let err = result.map_err(ExtensionError::from_error);
    /// assert!(err.is_err());
    /// ```
    #[inline]
    pub fn from_error<E: std::error::Error>(e: E) -> Self {
        Self {
            message: e.to_string(),
        }
    }

    /// Converts this error into a `CString` suitable for passing to `set_error`.
    ///
    /// If the message contains a null byte (which is valid in a Rust `String` but
    /// not in a C string), the message is truncated at the first null byte.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::error::ExtensionError;
    ///
    /// let err = ExtensionError::new("oops");
    /// let cstr = err.to_c_string();
    /// assert_eq!(cstr.to_str().unwrap(), "oops");
    /// ```
    #[must_use]
    pub fn to_c_string(&self) -> CString {
        CString::new(self.message.as_bytes()).unwrap_or_else(|_| {
            // Truncate at the first null byte to produce a valid C string.
            // No panic: if CString::new fails again (logically impossible since
            // we truncate at the first null byte), fall back to a generic message.
            let pos = self
                .message
                .bytes()
                .position(|b| b == 0)
                .unwrap_or(self.message.len());
            CString::new(&self.message.as_bytes()[..pos]).unwrap_or_else(|_| {
                // Defensive fallback — should never be reached.
                CString::new("extension error (message contained null bytes)")
                    .unwrap_or_else(|_| CString::default())
            })
        })
    }

    /// Returns the error message as a string slice.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::error::ExtensionError;
    ///
    /// let err = ExtensionError::new("bad input");
    /// assert_eq!(err.as_str(), "bad input");
    /// ```
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExtensionError {}

impl From<&str> for ExtensionError {
    #[inline]
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for ExtensionError {
    #[inline]
    fn from(s: String) -> Self {
        Self { message: s }
    }
}

impl From<Box<dyn std::error::Error>> for ExtensionError {
    #[inline]
    fn from(e: Box<dyn std::error::Error>) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for ExtensionError {
    #[inline]
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

impl From<std::io::Error> for ExtensionError {
    #[inline]
    fn from(e: std::io::Error) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

impl From<std::ffi::NulError> for ExtensionError {
    #[inline]
    fn from(e: std::ffi::NulError) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

impl From<std::fmt::Error> for ExtensionError {
    #[inline]
    fn from(e: std::fmt::Error) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

/// Convenience type alias for `Result<T, ExtensionError>`.
pub type ExtResult<T> = Result<T, ExtensionError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_with_message() {
        let err = ExtensionError::new("test error");
        assert_eq!(err.to_string(), "test error");
        assert_eq!(err.as_str(), "test error");
    }

    #[test]
    fn from_str() {
        let err = ExtensionError::from("from str");
        assert_eq!(err.message, "from str");
    }

    #[test]
    fn from_string() {
        let s = String::from("from String");
        let err = ExtensionError::from(s);
        assert_eq!(err.message, "from String");
    }

    #[test]
    fn from_error_wraps_display() {
        let parse_err = "abc".parse::<i32>().unwrap_err();
        let err = ExtensionError::from_error(parse_err);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn to_c_string_normal() {
        let err = ExtensionError::new("hello world");
        let cstr = err.to_c_string();
        assert_eq!(cstr.to_str().unwrap(), "hello world");
    }

    #[test]
    fn to_c_string_with_null_byte() {
        // A message with an embedded null byte should be truncated at the null
        let err = ExtensionError::new("before\0after");
        let cstr = err.to_c_string();
        assert_eq!(cstr.to_str().unwrap(), "before");
    }

    #[test]
    fn to_c_string_empty() {
        let err = ExtensionError::new("");
        let cstr = err.to_c_string();
        assert_eq!(cstr.to_str().unwrap(), "");
    }

    #[test]
    fn display_impl() {
        let err = ExtensionError::new("display test");
        let s = format!("{err}");
        assert_eq!(s, "display test");
    }

    #[test]
    fn debug_impl() {
        let err = ExtensionError::new("debug");
        let s = format!("{err:?}");
        assert!(s.contains("debug"));
    }

    #[test]
    fn clone_eq() {
        let err1 = ExtensionError::new("clone test");
        let err2 = err1.clone();
        assert_eq!(err1, err2);
    }

    #[test]
    fn from_box_dyn_error() {
        let boxed: Box<dyn std::error::Error> = "abc".parse::<i32>().unwrap_err().into();
        let err = ExtensionError::from(boxed);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn question_mark_operator_with_str() {
        fn fails() -> Result<(), ExtensionError> {
            Err("explicit error")?;
            Ok(())
        }
        assert_eq!(fails().unwrap_err().as_str(), "explicit error");
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = ExtensionError::from(io_err);
        assert_eq!(err.as_str(), "file not found");
    }

    #[test]
    fn question_mark_with_io_error() {
        fn fails() -> Result<(), ExtensionError> {
            Err(std::io::Error::other("runtime init failed"))?;
            Ok(())
        }
        assert_eq!(fails().unwrap_err().as_str(), "runtime init failed");
    }

    #[test]
    fn from_nul_error() {
        let nul_err = std::ffi::CString::new("hello\0world").unwrap_err();
        let err = ExtensionError::from(nul_err);
        assert!(!err.as_str().is_empty());
    }

    #[test]
    fn from_fmt_error() {
        let fmt_err = std::fmt::Error;
        let err = ExtensionError::from(fmt_err);
        assert!(!err.as_str().is_empty());
    }

    #[test]
    fn to_c_string_leading_null_byte() {
        // A message starting with null should truncate to empty string
        let err = ExtensionError::new("\0trailing");
        let cstr = err.to_c_string();
        assert_eq!(cstr.to_str().unwrap(), "");
    }

    #[test]
    fn to_c_string_multiple_null_bytes() {
        let err = ExtensionError::new("first\0second\0third");
        let cstr = err.to_c_string();
        assert_eq!(cstr.to_str().unwrap(), "first");
    }

    #[test]
    fn from_box_dyn_error_send_sync() {
        let boxed: Box<dyn std::error::Error + Send + Sync> =
            "abc".parse::<i32>().unwrap_err().into();
        let err = ExtensionError::from(boxed);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn ext_result_alias() {
        // Verify ExtResult<T> alias works as expected
        let ok_val: ExtResult<i32> = Ok(42);
        assert!(ok_val.is_ok());

        let err_val: ExtResult<i32> = Err(ExtensionError::new("fail"));
        assert!(err_val.is_err());
    }
}
