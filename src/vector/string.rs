// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `DuckDB` `VARCHAR` and `BLOB` (`duckdb_string_t`) reading utilities.
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
//! let view = DuckStringView::inline_from_bytes(&bytes).expect("inline value");
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
/// # Constructing a view
///
/// A `duckdb_string_t` longer than [`DUCK_STRING_INLINE_MAX_LEN`] stores a raw
/// heap pointer in bytes 8–15. Reading such a value means dereferencing a
/// pointer taken from the input bytes, which can only be justified by the
/// caller — so it lives behind [`from_raw`][Self::from_raw], an `unsafe fn`.
///
/// [`inline_from_bytes`][Self::inline_from_bytes] is the safe constructor: it
/// accepts any 16 bytes and refuses (returns `None`) whenever the value is in
/// pointer format, so a view built through it can never dereference anything.
#[derive(Debug, Clone, Copy)]
pub struct DuckStringView<'a> {
    bytes: &'a [u8],
    length: usize,
    /// `true` when the caller has vouched for the pointer in bytes 8–15 by going
    /// through the `unsafe` constructor. Views built safely keep this `false`
    /// and never dereference.
    pointer_is_trusted: bool,
}

impl<'a> DuckStringView<'a> {
    /// Creates a `DuckStringView` from a genuine `duckdb_string_t`.
    ///
    /// This is the constructor to use inside `DuckDB` callbacks, where the 16
    /// bytes really did come from a `DuckDB` vector.
    ///
    /// # Safety
    ///
    /// If the length field (bytes 0–3) exceeds [`DUCK_STRING_INLINE_MAX_LEN`],
    /// bytes 8–15 must hold a valid pointer to at least `length` readable bytes
    /// that stay live for `'a`.
    #[must_use]
    pub const unsafe fn from_raw(raw: &'a [u8; DUCK_STRING_SIZE]) -> Self {
        let length = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        Self {
            bytes: raw,
            length,
            pointer_is_trusted: true,
        }
    }

    /// Creates a `DuckStringView` from 16 bytes that may come from anywhere.
    ///
    /// Returns `None` if the value is in pointer format (length >
    /// [`DUCK_STRING_INLINE_MAX_LEN`]), because honouring the embedded pointer
    /// would mean dereferencing an address chosen by the input.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::vector::string::DuckStringView;
    ///
    /// let mut inline = [0u8; 16];
    /// inline[0] = 5;
    /// inline[4..9].copy_from_slice(b"hello");
    /// assert_eq!(
    ///     DuckStringView::inline_from_bytes(&inline).and_then(|v| v.as_str()),
    ///     Some("hello")
    /// );
    ///
    /// // A pointer-format value is refused rather than dereferenced.
    /// let mut pointer_format = [0u8; 16];
    /// pointer_format[..4].copy_from_slice(&64u32.to_le_bytes());
    /// pointer_format[8..16].copy_from_slice(&0xdead_beef_u64.to_le_bytes());
    /// assert!(DuckStringView::inline_from_bytes(&pointer_format).is_none());
    /// ```
    #[must_use]
    pub const fn inline_from_bytes(raw: &'a [u8; DUCK_STRING_SIZE]) -> Option<Self> {
        let length = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if length > DUCK_STRING_INLINE_MAX_LEN {
            return None;
        }
        Some(Self {
            bytes: raw,
            length,
            pointer_is_trusted: false,
        })
    }

    /// Creates a `DuckStringView` from the raw 16-byte representation.
    ///
    /// # Deprecated
    ///
    /// This constructor is safe but cannot honour pointer-format values: doing
    /// so would dereference an address supplied by its (safe) caller. It now
    /// behaves exactly like [`inline_from_bytes`][Self::inline_from_bytes]
    /// except that it cannot report the refusal, so a pointer-format value
    /// yields a view whose accessors return `None`.
    ///
    /// Use [`from_raw`][Self::from_raw] for real `duckdb_string_t` values, or
    /// [`inline_from_bytes`][Self::inline_from_bytes] when the input is
    /// untrusted and you want the refusal to be visible.
    #[must_use]
    #[deprecated(
        since = "0.16.0",
        note = "use `DuckStringView::from_raw` (unsafe, honours pointer format) or \
                `DuckStringView::inline_from_bytes` (safe, refuses pointer format)"
    )]
    pub const fn from_bytes(raw: &'a [u8; DUCK_STRING_SIZE]) -> Self {
        let length = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        Self {
            bytes: raw,
            length,
            pointer_is_trusted: false,
        }
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
    /// Returns `None` when the bytes are not valid UTF-8, when a pointer-format
    /// value carries a null pointer, or when this view was built through a safe
    /// constructor and the value is in pointer format (see
    /// [`inline_from_bytes`][Self::inline_from_bytes]).
    ///
    /// The pointer-format obligation — that bytes 8–15 address `len()` live
    /// bytes — was discharged when the view was created via
    /// [`from_raw`][Self::from_raw].
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
    /// # Platform assumption
    ///
    /// The pointer-format branch reads bytes 8–15 as a `u64` and truncates to
    /// `usize`. On 64-bit targets this is a lossless round-trip; on 32-bit
    /// (DuckDB-WASM via `wasm32-unknown-emscripten`) the C union still reserves
    /// 8 bytes for the pointer slot but only the lower 4 carry the address —
    /// the upper 4 are padding/zero. `u64 as usize` returns those lower bytes.
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
        } else if !self.pointer_is_trusted {
            // Pointer format, but this view was built through a safe
            // constructor. Dereferencing bytes 8..16 would follow an address the
            // safe caller supplied, so refuse instead.
            None
        } else {
            // Pointer case: bytes 8–15 hold the heap pointer in an 8-byte slot.
            // SAFETY: For pointer-format strings, bytes 8..16 hold a valid pointer
            // to heap memory allocated by DuckDB and valid for the vector's lifetime.
            let ptr_bytes: [u8; 8] = self.bytes[8..16].try_into().ok()?;
            // Read as u64 so this works regardless of `usize` width; truncating
            // to `usize` is a no-op on 64-bit and yields the low 4 bytes on wasm32
            // (where the upper 4 bytes of the 8-byte slot are zero padding), so the
            // truncation is intentional and lossless on every supported target.
            #[allow(clippy::cast_possible_truncation)]
            let ptr_val = u64::from_le_bytes(ptr_bytes) as usize as *const u8;
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
    // SAFETY: the caller vouched for the vector's pointer-format payloads.
    unsafe { DuckStringView::from_raw(raw_bytes) }
        .as_str()
        .unwrap_or("")
}

/// Reads a `DuckDB` `BLOB` value at the given row index.
///
/// Returns the raw bytes without UTF-8 validation, or an empty slice if a
/// pointer-format value contains a null pointer. `BLOB` and `VARCHAR` share the
/// same `duckdb_string_t` layout (inline for ≤ 12 bytes, pointer otherwise).
///
/// # Safety
///
/// - `data` must point at a `DuckDB` BLOB (or VARCHAR) vector's data buffer.
/// - `idx` must be within bounds of the vector.
/// - For pointer-format blobs, the heap data must be valid for the lifetime of
///   the returned slice.
pub unsafe fn read_duck_blob<'a>(data: *const u8, idx: usize) -> &'a [u8] {
    // SAFETY: each duckdb_string_t is exactly DUCK_STRING_SIZE bytes.
    let str_ptr = unsafe { data.add(idx * DUCK_STRING_SIZE) };
    let raw_bytes: &'a [u8; DUCK_STRING_SIZE] =
        unsafe { &*str_ptr.cast::<[u8; DUCK_STRING_SIZE]>() };
    // SAFETY: the caller vouched for the vector's pointer-format payloads.
    unsafe { DuckStringView::from_raw(raw_bytes) }
        .as_bytes_unsafe()
        .unwrap_or(&[])
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
        let view = DuckStringView::inline_from_bytes(&bytes).expect("inline");
        assert_eq!(view.len(), 0);
        assert!(view.is_empty());
        assert_eq!(view.as_str(), Some(""));
    }

    #[test]
    fn short_string_inline() {
        let bytes = make_inline_bytes("hello");
        let view = DuckStringView::inline_from_bytes(&bytes).expect("inline");
        assert_eq!(view.len(), 5);
        assert!(!view.is_empty());
        assert_eq!(view.as_str(), Some("hello"));
    }

    #[test]
    fn max_inline_string() {
        let s = "abcdefghijkl"; // exactly 12 bytes
        assert_eq!(s.len(), DUCK_STRING_INLINE_MAX_LEN);
        let bytes = make_inline_bytes(s);
        let view = DuckStringView::inline_from_bytes(&bytes).expect("inline");
        assert_eq!(view.len(), 12);
        assert_eq!(view.as_str(), Some(s));
    }

    #[test]
    fn read_duck_blob_preserves_non_utf8_inline_bytes() {
        let payload = [0x80u8, 0xF0, 0x01, 0x42];
        let mut bytes = [0u8; 16];
        let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        bytes[..4].copy_from_slice(&len.to_le_bytes());
        bytes[4..4 + payload.len()].copy_from_slice(&payload);

        assert_eq!(unsafe { read_duck_blob(bytes.as_ptr(), 0) }, &payload);
        assert_eq!(unsafe { read_duck_string(bytes.as_ptr(), 0) }, "");
    }

    #[test]
    fn read_duck_blob_preserves_non_utf8_pointer_bytes() {
        let payload = [
            0x80u8, 0xF0, 0x01, 0x42, 0xFF, 0x00, 0xFE, 0x7F, 0xAA, 0x55, 0xC0, 0xAF, 0x90,
        ];
        let mut bytes = [0u8; 16];
        let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        bytes[..4].copy_from_slice(&len.to_le_bytes());
        bytes[4..8].copy_from_slice(&payload[..4]);
        let ptr = payload.as_ptr() as usize as u64;
        bytes[8..16].copy_from_slice(&ptr.to_le_bytes());

        assert_eq!(unsafe { read_duck_blob(bytes.as_ptr(), 0) }, &payload);
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
        // Widen through `u64` so the 8-byte slot is filled on every target:
        // on wasm32 `usize` is 4 bytes, so `usize::to_le_bytes()` would yield
        // only 4 bytes and panic on this 8-byte copy (and would not match
        // DuckDB's 16-byte layout that `as_bytes_unsafe` reads back as a `u64`).
        let ptr_val = ptr as usize as u64;
        bytes[8..16].copy_from_slice(&ptr_val.to_le_bytes());

        // SAFETY: `bytes` really does point at `long_str`, which outlives the view.
        let view = unsafe { DuckStringView::from_raw(&bytes) };
        assert_eq!(view.len(), len);
        assert_eq!(view.as_str(), Some(long_str));
    }

    #[test]
    fn pointer_null_returns_none() {
        let mut bytes = [0u8; 16];
        // Write length > 12
        bytes[..4].copy_from_slice(&13u32.to_le_bytes());
        // pointer bytes 8..16 remain 0 (null pointer)

        // SAFETY: the length says pointer-format, and the null pointer case is
        // exactly what this test exercises.
        let view = unsafe { DuckStringView::from_raw(&bytes) };
        // Null pointer for long string should return None
        assert!(view.as_str().is_none());
        assert!(unsafe { read_duck_blob(bytes.as_ptr(), 0) }.is_empty());
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
        // Widen through `u64` so the 8-byte slot is filled on every target:
        // on wasm32 `usize` is 4 bytes, so `usize::to_le_bytes()` would yield
        // only 4 bytes and panic on this 8-byte copy (and would not match
        // DuckDB's 16-byte layout that `as_bytes_unsafe` reads back as a `u64`).
        let ptr_val = ptr as usize as u64;
        bytes[8..16].copy_from_slice(&ptr_val.to_le_bytes());

        // SAFETY: bytes is a valid pointer-format duckdb_string_t at idx 0.
        let s = unsafe { read_duck_string(bytes.as_ptr(), 0) };
        assert_eq!(s, long_str);
    }

    #[test]
    fn safe_constructor_refuses_pointer_format_instead_of_dereferencing() {
        // A hostile 16 bytes: length says "pointer format", and the pointer slot
        // holds an address that must never be dereferenced.
        let mut bytes = [0u8; DUCK_STRING_SIZE];
        bytes[..4].copy_from_slice(&1024u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&0xdead_beef_dead_beef_u64.to_le_bytes());

        assert!(DuckStringView::inline_from_bytes(&bytes).is_none());
    }

    #[test]
    fn safe_constructor_accepts_every_inline_length() {
        for len in 0..=DUCK_STRING_INLINE_MAX_LEN {
            let mut bytes = [b'x'; DUCK_STRING_SIZE];
            bytes[..4].copy_from_slice(&u32::try_from(len).expect("fits").to_le_bytes());
            let view = DuckStringView::inline_from_bytes(&bytes)
                .unwrap_or_else(|| panic!("length {len} must be inline"));
            assert_eq!(view.len(), len);
            assert_eq!(view.as_str().map(str::len), Some(len));
        }
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_constructor_no_longer_dereferences() {
        // The old safe `from_bytes` used to follow this pointer. It must not.
        let mut bytes = [0u8; DUCK_STRING_SIZE];
        bytes[..4].copy_from_slice(&64u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&0xdead_beef_dead_beef_u64.to_le_bytes());

        let view = DuckStringView::from_bytes(&bytes);
        assert_eq!(view.len(), 64);
        assert_eq!(view.as_str(), None);
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
