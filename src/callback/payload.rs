// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Disposing of a caught panic payload without re-entering the unwinder.
//!
//! `catch_unwind` hands back the payload as a `Box<dyn Any + Send>`, and
//! dropping that box runs the payload's `Drop` — which is arbitrary user code:
//! `std::panic::panic_any(value)` makes *any* `Send + 'static` value the payload.
//! If that `Drop` panics while the box is being dropped outside a guard, the new
//! unwind reaches the `extern "C"` frame the payload was caught in and the
//! process aborts (`panic_cannot_unwind`) — exactly what the first
//! `catch_unwind` existed to prevent.
//!
//! Everything in this crate that catches a panic at an FFI boundary therefore
//! gives the payload to [`take_panic_message`] or [`drop_panic_payload`] rather
//! than letting it fall out of scope.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Drops a caught panic payload, containing any panic its `Drop` raises.
///
/// If dropping the payload panics, that second panic's payload is dropped the
/// same way, and so on — a payload whose `Drop` panics usually panics with an
/// ordinary message, whose own drop cannot panic, so the chain ends at once and
/// nothing is leaked. Only after [`MAX_NESTED_PAYLOAD_DROPS`] consecutive
/// panicking drops is the remaining payload leaked with [`std::mem::forget`]:
/// the chain is user code with no inherent bound, and leaking one allocation is
/// the price of never aborting.
///
/// # Example
///
/// ```rust
/// struct Bomb;
/// impl Drop for Bomb {
///     fn drop(&mut self) {
///         panic!("payload destructor exploded");
///     }
/// }
///
/// let payload = std::panic::catch_unwind(|| std::panic::panic_any(Bomb)).unwrap_err();
/// // Dropping `payload` directly would unwind out of here.
/// quack_rs::callback::drop_panic_payload(payload);
/// ```
pub fn drop_panic_payload(payload: Box<dyn Any + Send>) {
    let mut payload = payload;
    for _ in 0..MAX_NESTED_PAYLOAD_DROPS {
        match catch_unwind(AssertUnwindSafe(move || drop(payload))) {
            Ok(()) => return,
            Err(next) => payload = next,
        }
    }
    std::mem::forget(payload);
}

/// How many nested panicking payload drops [`drop_panic_payload`] unwinds
/// through before it leaks what is left rather than keep going.
pub const MAX_NESTED_PAYLOAD_DROPS: usize = 8;

/// Extracts the panic message from a caught payload, then disposes of the
/// payload with [`drop_panic_payload`].
///
/// The message is copied out first, so it survives even when the payload's
/// `Drop` panics. Used by every callback macro in this crate; public because the
/// macros expand in the extension crate and need to reach it.
///
/// # Example
///
/// ```rust
/// let payload = std::panic::catch_unwind(|| panic!("boom")).unwrap_err();
/// assert_eq!(quack_rs::callback::take_panic_message(payload), "boom");
/// ```
#[must_use]
pub fn take_panic_message(payload: Box<dyn Any + Send>) -> String {
    let message = super::panic_message(&payload);
    drop_panic_payload(payload);
    message
}

#[cfg(test)]
mod tests {
    use super::{drop_panic_payload, take_panic_message};
    use crate::callback::catch_ffi_panic;
    use std::cell::Cell;

    thread_local! {
        // Per thread, because the test harness runs these in parallel and the
        // payload is always dropped on the thread that caught it.
        static BOMB_DROPS: Cell<usize> = const { Cell::new(0) };
    }

    /// A panic payload whose destructor itself panics.
    struct PayloadBomb;
    impl Drop for PayloadBomb {
        fn drop(&mut self) {
            BOMB_DROPS.with(|n| n.set(n.get() + 1));
            panic!("panic payload destructor deliberately exploded");
        }
    }

    // Each of these would abort the whole test binary before the fix
    // (`panic_cannot_unwind` / "panic in a function that cannot unwind"), so
    // reaching the assertions is itself the regression check.

    #[test]
    fn a_payload_whose_drop_panics_is_contained() {
        let payload = std::panic::catch_unwind(|| std::panic::panic_any(PayloadBomb))
            .expect_err("panic_any must unwind");
        let before = BOMB_DROPS.with(Cell::get);
        drop_panic_payload(payload);
        assert_eq!(BOMB_DROPS.with(Cell::get), before + 1);
    }

    #[test]
    fn take_panic_message_survives_a_payload_whose_drop_panics() {
        let payload = std::panic::catch_unwind(|| std::panic::panic_any(PayloadBomb))
            .expect_err("panic_any must unwind");
        assert_eq!(take_panic_message(payload), "<non-string panic payload>");
    }

    #[test]
    fn catch_ffi_panic_contains_a_payload_whose_drop_panics() {
        let outcome: Result<(), String> = catch_ffi_panic(|| std::panic::panic_any(PayloadBomb));
        assert_eq!(outcome, Err("<non-string panic payload>".to_owned()));
    }

    // The generated destructor is the one macro with no DuckDB call in it, so it
    // is the one that can be driven without a database.
    crate::aggregate_destroy_callback!(bomb_destroy, |_states, _count| {
        std::panic::panic_any(PayloadBomb);
    });

    #[test]
    fn a_generated_destructor_contains_a_payload_whose_drop_panics() {
        // SAFETY: the body ignores both arguments.
        unsafe { bomb_destroy(std::ptr::null_mut(), 0) };
    }

    thread_local! {
        static NESTED_DROPS: Cell<usize> = const { Cell::new(0) };
    }

    /// A payload whose `Drop` panics with another `NestedBomb` one level
    /// shallower, until level 0, whose `Drop` returns normally.
    struct NestedBomb(usize);
    impl Drop for NestedBomb {
        fn drop(&mut self) {
            NESTED_DROPS.with(|n| n.set(n.get() + 1));
            if self.0 > 0 {
                std::panic::panic_any(Self(self.0 - 1));
            }
        }
    }

    /// Every payload in a chain shorter than the bound is dropped, not leaked:
    /// before, the payload raised by the first panicking `Drop` was
    /// `mem::forget`-ed unconditionally, which LeakSanitizer and Miri report.
    #[test]
    fn a_chain_of_panicking_payload_drops_is_freed_to_the_end() {
        let depth = super::MAX_NESTED_PAYLOAD_DROPS - 2;
        let payload = std::panic::catch_unwind(|| std::panic::panic_any(NestedBomb(depth)))
            .expect_err("panic_any must unwind");
        let before = NESTED_DROPS.with(Cell::get);
        drop_panic_payload(payload);
        // Levels `depth, depth - 1, …, 0` each dropped exactly once.
        assert_eq!(NESTED_DROPS.with(Cell::get), before + depth + 1);
    }

    #[test]
    fn a_string_payload_keeps_its_message() {
        let payload =
            std::panic::catch_unwind(|| panic!("boom {}", 7)).expect_err("panic must unwind");
        assert_eq!(take_panic_message(payload), "boom 7");
    }
}
