// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

//! Test utilities for aggregate function logic.
//!
//! This module provides an [`AggregateTestHarness`] that lets you test your
//! aggregate state logic — the `update` and `combine` operations — in pure Rust
//! without spinning up a `DuckDB` instance.
//!
//! # Why you need both unit tests AND E2E tests
//!
//! **Unit tests (this harness)** verify that your `MyState::update` and
//! `MyState::combine` methods produce correct results. They run fast and
//! catch logical bugs.
//!
//! **E2E tests (`DuckDB` CLI / `SQLLogicTest`)** verify that the FFI wiring is
//! correct — that `state_size`, `state_init`, `state_destroy`, and the
//! callback signatures match what `DuckDB` expects.
//!
//! In duckdb-behavioral, 435 unit tests passed while the extension was completely
//! broken due to three bugs that only E2E tests can catch:
//! - SEGFAULT on load (wrong entry point)
//! - 6 of 7 functions failing silently (function set name not set on each member)
//! - Window funnel returning wrong results (combine not propagating config)
//!
//! **Unit tests alone are insufficient. Always run E2E tests.**
//!
//! # Example
//!
//! ```rust
//! use quack_rs::testing::AggregateTestHarness;
//! use quack_rs::aggregate::AggregateState;
//!
//! #[derive(Default, Debug, PartialEq)]
//! struct SumState { total: i64 }
//! impl AggregateState for SumState {}
//!
//! impl SumState {
//!     fn update(&mut self, value: i64) {
//!         self.total += value;
//!     }
//!     fn combine(source: &Self, target: &mut Self) {
//!         target.total += source.total;
//!     }
//!     fn finalize(&self) -> i64 {
//!         self.total
//!     }
//! }
//!
//! let mut harness = AggregateTestHarness::<SumState>::new();
//! harness.update(|s| s.update(10));
//! harness.update(|s| s.update(20));
//! harness.update(|s| s.update(5));
//!
//! let state = harness.finalize();
//! assert_eq!(state.total, 35);
//! ```

pub mod harness;
pub use harness::AggregateTestHarness;
