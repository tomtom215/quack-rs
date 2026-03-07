// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`AggregateTestHarness`] — test aggregate logic without `DuckDB`.
//!
//! Simulates the `DuckDB` aggregate lifecycle in pure Rust:
//! `init → N × update → combine (optional) → finalize`

use crate::aggregate::AggregateState;

/// A test harness that simulates the `DuckDB` aggregate function lifecycle in
/// pure Rust, without requiring a `DuckDB` instance.
///
/// # Usage
///
/// ```rust
/// use quack_rs::testing::AggregateTestHarness;
/// use quack_rs::aggregate::AggregateState;
///
/// #[derive(Default, Debug, PartialEq)]
/// struct Counter { n: u64 }
/// impl AggregateState for Counter {}
///
/// let mut harness = AggregateTestHarness::<Counter>::new();
/// harness.update(|c| c.n += 1);
/// harness.update(|c| c.n += 1);
/// harness.update(|c| c.n += 1);
///
/// let state = harness.finalize();
/// assert_eq!(state.n, 3);
/// ```
pub struct AggregateTestHarness<S: AggregateState> {
    state: S,
}

impl<S: AggregateState> AggregateTestHarness<S> {
    /// Creates a new harness with a fresh `S::default()` state.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::testing::AggregateTestHarness;
    /// use quack_rs::aggregate::AggregateState;
    ///
    /// #[derive(Default)]
    /// struct MyState { count: u64 }
    /// impl AggregateState for MyState {}
    ///
    /// let harness = AggregateTestHarness::<MyState>::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: S::default(),
        }
    }

    /// Creates a harness with an explicit initial state.
    ///
    /// Useful for testing `combine` with a pre-populated state.
    #[must_use]
    pub const fn with_state(state: S) -> Self {
        Self { state }
    }

    /// Applies an update to the current state.
    ///
    /// Simulates one row being processed by the aggregate's `update` callback.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::testing::AggregateTestHarness;
    /// use quack_rs::aggregate::AggregateState;
    ///
    /// #[derive(Default)]
    /// struct Sum { total: i64 }
    /// impl AggregateState for Sum {}
    ///
    /// let mut h = AggregateTestHarness::<Sum>::new();
    /// h.update(|s| s.total += 5);
    /// h.update(|s| s.total += 3);
    /// ```
    pub fn update<F>(&mut self, f: F)
    where
        F: FnOnce(&mut S),
    {
        f(&mut self.state);
    }

    /// Simulates a `combine` operation: merges the source harness's state into
    /// this harness's state using the provided combine function.
    ///
    /// This tests the correctness of your `combine` implementation, including the
    /// critical requirement that all configuration fields are propagated from source
    /// to target (Pitfall L1 / Problem 4).
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::testing::AggregateTestHarness;
    /// use quack_rs::aggregate::AggregateState;
    ///
    /// #[derive(Default, Clone)]
    /// struct Sum { total: i64 }
    /// impl AggregateState for Sum {}
    ///
    /// let mut h1 = AggregateTestHarness::<Sum>::new();
    /// h1.update(|s| s.total += 10);
    ///
    /// let mut h2 = AggregateTestHarness::<Sum>::new();
    /// h2.update(|s| s.total += 20);
    ///
    /// // Combine h1 into h2
    /// h2.combine(&h1, |source, target| target.total += source.total);
    ///
    /// let result = h2.finalize();
    /// assert_eq!(result.total, 30);
    /// ```
    pub fn combine<F>(&mut self, source: &Self, combine_fn: F)
    where
        F: FnOnce(&S, &mut S),
    {
        combine_fn(&source.state, &mut self.state);
    }

    /// Consumes the harness and returns the final state.
    ///
    /// Simulates `DuckDB` calling the `finalize` callback.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::testing::AggregateTestHarness;
    /// use quack_rs::aggregate::AggregateState;
    ///
    /// #[derive(Default, Debug, PartialEq)]
    /// struct Sum { total: i64 }
    /// impl AggregateState for Sum {}
    ///
    /// let mut h = AggregateTestHarness::<Sum>::new();
    /// h.update(|s| s.total = 42);
    /// assert_eq!(h.finalize().total, 42);
    /// ```
    #[must_use]
    pub fn finalize(self) -> S {
        self.state
    }

    /// Borrows the current state for inspection without consuming the harness.
    ///
    /// Useful for asserting intermediate state during a sequence of updates.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::testing::AggregateTestHarness;
    /// use quack_rs::aggregate::AggregateState;
    ///
    /// #[derive(Default)]
    /// struct Sum { total: i64 }
    /// impl AggregateState for Sum {}
    ///
    /// let mut h = AggregateTestHarness::<Sum>::new();
    /// h.update(|s| s.total += 5);
    /// assert_eq!(h.state().total, 5);
    /// h.update(|s| s.total += 3);
    /// assert_eq!(h.state().total, 8);
    /// ```
    #[must_use]
    pub const fn state(&self) -> &S {
        &self.state
    }

    /// Resets the state to `S::default()`.
    ///
    /// Useful for running multiple scenarios with a single harness.
    pub fn reset(&mut self) {
        self.state = S::default();
    }

    /// Runs a sequence of updates from an iterator and returns the final state.
    ///
    /// This is a convenience method for testing aggregate functions over a
    /// collection of input values.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::testing::AggregateTestHarness;
    /// use quack_rs::aggregate::AggregateState;
    ///
    /// #[derive(Default)]
    /// struct Sum { total: i64 }
    /// impl AggregateState for Sum {}
    ///
    /// let result = AggregateTestHarness::<Sum>::aggregate(
    ///     [1_i64, 2, 3, 4, 5],
    ///     |s, v| s.total += v,
    /// );
    /// assert_eq!(result.total, 15);
    /// ```
    pub fn aggregate<I, T, F>(inputs: I, update_fn: F) -> S
    where
        I: IntoIterator<Item = T>,
        F: Fn(&mut S, T),
    {
        let mut harness = Self::new();
        for input in inputs {
            harness.update(|s| update_fn(s, input));
        }
        harness.finalize()
    }
}

impl<S: AggregateState> Default for AggregateTestHarness<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::AggregateState;

    #[derive(Default, Debug, PartialEq, Clone)]
    struct SumState {
        total: i64,
    }
    impl AggregateState for SumState {}

    #[derive(Default, Debug, PartialEq, Clone)]
    struct CountConfig {
        /// Configuration field — must be propagated in combine
        window_size: i64,
        count: u64,
    }
    impl AggregateState for CountConfig {}

    #[test]
    fn new_creates_default_state() {
        let h = AggregateTestHarness::<SumState>::new();
        assert_eq!(h.state().total, 0);
    }

    #[test]
    fn with_state() {
        let initial = SumState { total: 100 };
        let h = AggregateTestHarness::with_state(initial);
        assert_eq!(h.state().total, 100);
    }

    #[test]
    fn update_accumulates() {
        let mut h = AggregateTestHarness::<SumState>::new();
        h.update(|s| s.total += 10);
        h.update(|s| s.total += 20);
        h.update(|s| s.total += 5);
        assert_eq!(h.finalize().total, 35);
    }

    #[test]
    fn finalize_returns_state() {
        let mut h = AggregateTestHarness::<SumState>::new();
        h.update(|s| s.total = 42);
        assert_eq!(h.finalize(), SumState { total: 42 });
    }

    #[test]
    fn state_borrow() {
        let mut h = AggregateTestHarness::<SumState>::new();
        h.update(|s| s.total = 7);
        assert_eq!(h.state().total, 7);
        // Can still update after borrowing
        h.update(|s| s.total += 1);
        assert_eq!(h.state().total, 8);
    }

    #[test]
    fn reset_clears_state() {
        let mut h = AggregateTestHarness::<SumState>::new();
        h.update(|s| s.total = 999);
        h.reset();
        assert_eq!(h.state().total, 0);
    }

    #[test]
    fn combine_merges_states() {
        let mut h1 = AggregateTestHarness::<SumState>::new();
        h1.update(|s| s.total += 10);

        let mut h2 = AggregateTestHarness::<SumState>::new();
        h2.update(|s| s.total += 20);

        h2.combine(&h1, |src, tgt| tgt.total += src.total);
        assert_eq!(h2.finalize().total, 30);
    }

    #[test]
    fn combine_propagates_config_fields() {
        // This test demonstrates Pitfall L1: combine must propagate config fields.
        // If window_size is not propagated, the merged state will have window_size = 0.
        let mut h1 = AggregateTestHarness::<CountConfig>::new();
        h1.update(|s| {
            s.window_size = 3600; // config field
            s.count += 5;
        });

        let h2 = AggregateTestHarness::<CountConfig>::new();
        // h2 is zero-initialized (simulates a fresh state created by DuckDB)

        let mut target = h2;
        target.combine(&h1, |src, tgt| {
            // Correct: propagate ALL fields including config
            tgt.window_size = src.window_size;
            tgt.count += src.count;
        });

        let result = target.finalize();
        assert_eq!(
            result.window_size, 3600,
            "config field must be propagated in combine"
        );
        assert_eq!(result.count, 5);
    }

    #[test]
    fn combine_bug_missing_config_propagation() {
        // This test demonstrates WHAT GOES WRONG if you forget to propagate config fields.
        let mut h1 = AggregateTestHarness::<CountConfig>::new();
        h1.update(|s| {
            s.window_size = 3600;
            s.count += 5;
        });

        let h2 = AggregateTestHarness::<CountConfig>::new();
        let mut target = h2;

        // BUG: only propagate count, forget window_size
        target.combine(&h1, |src, tgt| {
            tgt.count += src.count; // forgot: tgt.window_size = src.window_size
        });

        let result = target.finalize();
        assert_eq!(
            result.window_size, 0,
            "demonstrates the bug: config is lost"
        );
        assert_eq!(result.count, 5);
    }

    #[test]
    fn aggregate_convenience_method() {
        let result =
            AggregateTestHarness::<SumState>::aggregate([1_i64, 2, 3, 4, 5], |s, v| s.total += v);
        assert_eq!(result.total, 15);
    }

    #[test]
    fn aggregate_empty_input() {
        let result =
            AggregateTestHarness::<SumState>::aggregate(std::iter::empty::<i64>(), |s, v| {
                s.total += v;
            });
        assert_eq!(result.total, 0);
    }

    #[test]
    fn default_impl() {
        let h = AggregateTestHarness::<SumState>::default();
        assert_eq!(h.state().total, 0);
    }

    mod proptest_harness {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn aggregate_sum_matches_iter_sum(values: Vec<i64>) {
                // Filter values that would overflow
                let safe_values: Vec<i64> = values.iter().copied()
                    .filter(|&v| v.abs() < 1_000_000)
                    .collect();

                let harness_sum = AggregateTestHarness::<SumState>::aggregate(
                    safe_values.iter().copied(),
                    |s, v| s.total += v,
                );
                let iter_sum: i64 = safe_values.iter().sum();
                prop_assert_eq!(harness_sum.total, iter_sum);
            }

            #[test]
            fn combine_is_associative(a: i64, b: i64, c: i64) {
                // Test: (a + b) + c == a + (b + c) for SumState
                let limit = 1_000_000_i64;
                let (a, b, c) = (a % limit, b % limit, c % limit);

                // (a + b) + c
                let mut h1 = AggregateTestHarness::<SumState>::new();
                h1.update(|s| s.total = a);
                let mut h2 = AggregateTestHarness::<SumState>::new();
                h2.update(|s| s.total = b);
                h2.combine(&h1, |src, tgt| tgt.total += src.total);
                let mut h3 = AggregateTestHarness::<SumState>::new();
                h3.update(|s| s.total = c);
                h3.combine(&h2, |src, tgt| tgt.total += src.total);
                let result1 = h3.finalize().total;

                // a + (b + c)
                let mut h_b = AggregateTestHarness::<SumState>::new();
                h_b.update(|s| s.total = b);
                let mut h_c = AggregateTestHarness::<SumState>::new();
                h_c.update(|s| s.total = c);
                h_c.combine(&h_b, |src, tgt| tgt.total += src.total);
                let mut h_a = AggregateTestHarness::<SumState>::new();
                h_a.update(|s| s.total = a);
                h_a.combine(&h_c, |src, tgt| tgt.total += src.total);
                let result2 = h_a.finalize().total;

                prop_assert_eq!(result1, result2);
            }

            #[test]
            fn combine_identity_element(value: i64) {
                // Combining with an empty (default) state is idempotent
                let v = value % 1_000_000;
                let mut h = AggregateTestHarness::<SumState>::new();
                h.update(|s| s.total = v);

                let empty = AggregateTestHarness::<SumState>::new();
                h.combine(&empty, |src, tgt| tgt.total += src.total);

                prop_assert_eq!(h.finalize().total, v);
            }
        }
    }
}
