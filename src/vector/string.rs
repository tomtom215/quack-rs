//! `DuckDB` `VARCHAR` (`duckdb_string_t`) reading utilities.
//!
//! # Pitfall P7: Undocumented `duckdb_string_t` format
//!
//! `DuckDB` stores VARCHAR values in a 16-byte `duckdb_string_t` struct with two
//! representations:
//!
//! - **Inline** (length ≤ 12): `[ len: u32 | data: [u8; 12] ]`
//! - **Pointer** (length > 12): `[ len: u32 | prefix: [u8; 4] | ptr: *const u8 | unused: u32 ]`
//!
//! This is not documented in the Rust bindings. The layout was determined by
//! reading `DuckDB`'s C source and confirmed by the duckdb-behavioral implementation.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::vector::string::{DuckStringView, read_duck_string};
//!
//! // A short string (inline case)
//! let bytes: [u8; 16] = {
//!     let mut b = [0u8; 16];
//!     b[0] = 5; // length = 5
//!     b[4..9].copy_from_slice(b"hello");
//!     b
//! };
//! let view = DuckStringView::from_bytes(&bytes);
//! assert_eq!(view.as_str(), Some("hello"));
//! assert_eq!(view.len(), 5);
//! ```

/// The size of a `duckdb_string_t` in bytes.
pub const DUCK_STRING_SIZE: usize = 16;

/// The maximum string length that fits inline in a `duckdb_string_t` (≤12 bytes).
pub const DUCK_STRING_INLINE_MAX_LEN: usize = 12;

/// A parsed view of a `duckdb_string_t` value.
///
/// This type borrows from the raw vector data — it does not allocate.
///
/// # Safety
///
/// The `data` slice from which this view is created must outlive the view.
/// For pointer-format strings, the pointed-to heap data must also be valid.
#[derive(Debug, Clone, Copy)]
pub struct DuckStringView<'a> {
    bytes: &'a [u8],
    length: usize,
}

impl<'a> DuckStringView<'a> {
    /// Creates a `DuckStringView` from the raw 16-byte representation.
    ///
    /// # Panics
    ///
    /// Panics if `raw` is not exactly 16 bytes.
    #[must_use]
    pub const fn from_bytes(raw: &'a [u8; DUCK_STRING_SIZE]) -> Self {
        let length = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        Self { bytes: raw, length }
    }

    /// Returns the length of the string in bytes.
    #[must_use]
    #[inline]
    pub const fn len(&self) -> usize {
        self.length
    }

    /// Returns `true` if the string is empty.
    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns the string as a UTF-8 `str` slice, or `None` if it is not valid UTF-8.
    ///
    /// The returned `&'a str` has the same lifetime as the underlying data slice —
    /// not the lifetime of `self`. This allows the result to outlive the `DuckStringView`.
    ///
    /// # Safety
    ///
    /// For pointer-format strings (length > 12), the pointer stored at bytes 8–15
    /// must be a valid pointer to at least `self.length` bytes of string data that
    /// is live for lifetime `'a`.
    #[must_use]
    pub fn as_str(&self) -> Option<&'a str> {
        let slice = self.as_bytes_unsafe()?;
        std::str::from_utf8(slice).ok()
    }

    /// Returns the raw bytes of the string content.
    ///
    /// Returns `None` if the internal pointer (for long strings) is null.
    ///
    /// The returned bytes have lifetime `'a` (the lifetime of the underlying data).
    ///
    /// # Safety (internal)
    ///
    /// This method dereferences the pointer stored in the `duckdb_string_t` struct
    /// for strings longer than 12 bytes. The caller (i.e., the `DuckStringView`
    /// constructor) must ensure the underlying vector data is still valid.
    fn as_bytes_unsafe(&self) -> Option<&'a [u8]> {
        if self.length <= DUCK_STRING_INLINE_MAX_LEN {
            // Inline case: data starts at byte 4, length bytes follow
            Some(&self.bytes[4..4 + self.length])
        } else {
            // Pointer case: bytes 8–15 contain the pointer (little-endian usize)
            // SAFETY: For pointer-format strings, bytes 8..16 hold a valid pointer
            // to heap memory allocated by DuckDB and valid for the vector's lifetime.
            let ptr_bytes: [u8; 8] = self.bytes[8..16].try_into().ok()?;
            let ptr_val = usize::from_le_bytes(ptr_bytes) as *const u8;
            if ptr_val.is_null() {
                return None;
            }
            // SAFETY: `ptr_val` is a DuckDB-managed pointer; the caller guarantees
            // the underlying data is valid for the lifetime of the DuckStringView.
            Some(unsafe { std::slice::from_raw_parts(ptr_val, self.length) })
        }
    }
}

/// Reads a `DuckDB` `VARCHAR` value from a raw vector data pointer at a given row index.
///
/// Returns the string as a `&str` slice, or an empty string if the data is not
/// valid UTF-8 or if the pointer is null.
///
/// # Pitfall P7
///
/// `DuckDB` strings have two storage formats:
/// - **Inline** (≤ 12 bytes): stored directly in the 16-byte struct
/// - **Pointer** (> 12 bytes): struct contains a pointer to heap-allocated data
///
/// This function handles both transparently.
///
/// # Safety
///
/// - `data` must point to a `DuckDB` VARCHAR vector's data buffer.
/// - `idx` must be within bounds of the vector.
/// - For pointer-format strings, the heap data pointed to must be valid for the
///   duration of this function call and the returned `&str` slice.
/// - The returned `&str` borrows from the `DuckDB` vector — do not destroy the
///   data chunk while the returned reference is live.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::vector::string::read_duck_string;
///
/// // Inside a DuckDB aggregate callback:
/// // let data = libduckdb_sys::duckdb_vector_get_data(vec) as *const u8;
/// // let s = unsafe { read_duck_string(data, row_idx) };
/// # let data: *const u8 = std::ptr::null();
/// # let _ = data;
/// ```
pub unsafe fn read_duck_string<'a>(data: *const u8, idx: usize) -> &'a str {
    // SAFETY: Each duckdb_string_t is exactly 16 bytes. The caller guarantees
    // `data` is valid and `idx` is in bounds.
    let str_ptr = unsafe { data.add(idx * DUCK_STRING_SIZE) };
    // SAFETY: `str_ptr` points to the idx-th duckdb_string_t in the vector.
    // The reference has lifetime 'a because it borrows from the raw pointer
    // whose backing data lives for the vector's lifetime ('a per caller's contract).
    let raw_bytes: &'a [u8; DUCK_STRING_SIZE] =
        unsafe { &*str_ptr.cast::<[u8; DUCK_STRING_SIZE]>() };
    // DuckStringView<'a> stores &'a [u8], so as_str() returns Option<&'a str>.
    DuckStringView::from_bytes(raw_bytes).as_str().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_inline_bytes(s: &str) -> [u8; 16] {
        assert!(
            s.len() <= DUCK_STRING_INLINE_MAX_LEN,
            "use pointer format for long strings"
        );
        let mut bytes = [0u8; 16];
        let len = u32::try_from(s.len()).unwrap_or(u32::MAX);
        bytes[..4].copy_from_slice(&len.to_le_bytes());
        bytes[4..4 + s.len()].copy_from_slice(s.as_bytes());
        bytes
    }

    #[test]
    fn empty_string_inline() {
        let bytes = make_inline_bytes("");
        let view = DuckStringView::from_bytes(&bytes);
        assert_eq!(view.len(), 0);
        assert!(view.is_empty());
        assert_eq!(view.as_str(), Some(""));
    }

    #[test]
    fn short_string_inline() {
        let bytes = make_inline_bytes("hello");
        let view = DuckStringView::from_bytes(&bytes);
        assert_eq!(view.len(), 5);
        assert!(!view.is_empty());
        assert_eq!(view.as_str(), Some("hello"));
    }

    #[test]
    fn max_inline_string() {
        let s = "abcdefghijkl"; // exactly 12 bytes
        assert_eq!(s.len(), DUCK_STRING_INLINE_MAX_LEN);
        let bytes = make_inline_bytes(s);
        let view = DuckStringView::from_bytes(&bytes);
        assert_eq!(view.len(), 12);
        assert_eq!(view.as_str(), Some(s));
    }

    #[test]
    fn pointer_format_string() {
        let long_str = "this is a longer string that exceeds 12 bytes";
        let len = long_str.len();
        let ptr = long_str.as_ptr();

        let mut bytes = [0u8; 16];
        // Write length
        bytes[..4].copy_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
        // Write prefix (first 4 bytes of the string)
        bytes[4..8].copy_from_slice(&long_str.as_bytes()[..4]);
        // Write pointer at bytes 8..16
        let ptr_val = ptr as usize;
        bytes[8..16].copy_from_slice(&ptr_val.to_le_bytes());

        let view = DuckStringView::from_bytes(&bytes);
        assert_eq!(view.len(), len);
        assert_eq!(view.as_str(), Some(long_str));
    }

    #[test]
    fn pointer_null_returns_none() {
        let mut bytes = [0u8; 16];
        // Write length > 12
        bytes[..4].copy_from_slice(&13u32.to_le_bytes());
        // pointer bytes 8..16 remain 0 (null pointer)

        let view = DuckStringView::from_bytes(&bytes);
        // Null pointer for long string should return None
        assert!(view.as_str().is_none());
    }

    #[test]
    fn read_duck_string_inline() {
        let bytes = make_inline_bytes("world");
        let data = bytes.as_ptr();
        // SAFETY: data points to a valid 16-byte inline string at idx 0.
        let s = unsafe { read_duck_string(data, 0) };
        assert_eq!(s, "world");
    }

    #[test]
    fn read_duck_string_pointer_format() {
        let long_str = "abcdefghijklmnopqrst"; // 20 bytes
        let len = long_str.len();
        let ptr = long_str.as_ptr();

        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
        bytes[4..8].copy_from_slice(&long_str.as_bytes()[..4]);
        let ptr_val = ptr as usize;
        bytes[8..16].copy_from_slice(&ptr_val.to_le_bytes());

        // SAFETY: bytes is a valid pointer-format duckdb_string_t at idx 0.
        let s = unsafe { read_duck_string(bytes.as_ptr(), 0) };
        assert_eq!(s, long_str);
    }

    #[test]
    fn duck_string_size_constant() {
        assert_eq!(DUCK_STRING_SIZE, 16);
    }

    #[test]
    fn duck_string_inline_max_len_constant() {
        assert_eq!(DUCK_STRING_INLINE_MAX_LEN, 12);
    }
}
