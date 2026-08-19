// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Internal helpers for the crate's `Debug` implementations.
//!
//! Rust API guideline [C-DEBUG] asks every public type to implement `Debug`,
//! and the cost of skipping it is not cosmetic: `Result::unwrap`,
//! `Result::expect_err`, `assert_eq!` and `#[derive(Debug)]` on any downstream
//! struct that stores one of our types all stop compiling. Every public type in
//! quack-rs therefore implements it, and `missing_debug_implementations` is
//! enabled crate-wide so new ones cannot forget.
//!
//! Almost every type here wraps an opaque `DuckDB` handle, so there is nothing
//! structural to print. The pointer is printed anyway: it is the only thing that
//! distinguishes two live handles, and "which chunk did I write to?" is a
//! question `Debug` output is genuinely read to answer.
//!
//! [C-DEBUG]: https://rust-lang.github.io/api-guidelines/debugging.html

use core::fmt;

/// Renders an optional callback as `set` / `unset`.
///
/// The address of an `extern "C"` function tells a reader nothing; whether a
/// builder has been given one before `register` is called is the actual
/// question.
pub struct Callback(pub bool);

impl fmt::Debug for Callback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0 { "set" } else { "unset" })
    }
}

impl Callback {
    /// `set` when the builder field holds a callback.
    ///
    /// Takes `&Option<T>` rather than `Option<&T>` so call sites read
    /// `Callback::of(&self.function)` — a `Debug` impl only ever has the field
    /// by reference, and `.as_ref()` at every one of thirty call sites buys
    /// nothing.
    #[allow(clippy::ref_option, reason = "reads better at every call site")]
    pub const fn of<T>(option: &Option<T>) -> Self {
        Self(option.is_some())
    }
}

/// Implements `Debug` for a wrapper over a single opaque `DuckDB` handle,
/// printing the type name and the handle pointer.
macro_rules! impl_handle_debug {
    ($($ty:ident . $field:ident),* $(,)?) => {
        $(
            impl ::core::fmt::Debug for $ty {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    f.debug_struct(::core::stringify!($ty))
                        .field(::core::stringify!($field), &self.$field)
                        .finish()
                }
            }
        )*
    };
}

pub(crate) use impl_handle_debug;
