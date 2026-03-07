# Testing Guide

quack-rs provides a two-tier testing strategy: **pure-Rust unit tests** for
business logic (no DuckDB required), and **SQLLogicTest E2E tests** that run
inside an actual DuckDB process.

---

## Why two tiers?

> **Pitfall P3** — Unit tests are insufficient. 435 unit tests passed in
> duckdb-behavioral while the extension had three critical bugs: a SEGFAULT on
> load, 6 of 7 functions not registering, and wrong results from a combine bug.
> E2E tests caught all three.

| Test tier | What it catches | What it misses |
|-----------|-----------------|----------------|
| Unit tests | Logic bugs in state structs | FFI wiring, registration failures, SEGFAULT |
| E2E tests | Everything above + FFI integration | Nothing (it's real DuckDB) |

**Both tiers are required.** Unit tests give fast, deterministic feedback.
E2E tests prove the extension actually works inside DuckDB.

---

## Unit tests with `AggregateTestHarness`

`AggregateTestHarness<S>` simulates the DuckDB aggregate lifecycle in pure Rust
without any DuckDB dependency:

```
new() → N × update() → combine() (optional) → finalize()
```

### Basic usage

```rust
use quack_rs::testing::AggregateTestHarness;
use quack_rs::aggregate::AggregateState;

#[derive(Default, Debug, PartialEq)]
struct SumState { total: i64 }
impl AggregateState for SumState {}

#[test]
fn test_sum() {
    let mut h = AggregateTestHarness::<SumState>::new();
    h.update(|s| s.total += 10);
    h.update(|s| s.total += 20);
    h.update(|s| s.total += 5);
    assert_eq!(h.finalize().total, 35);
}
```

### Convenience: `aggregate`

For testing over a collection of inputs:

```rust
#[test]
fn test_word_count() {
    let result = AggregateTestHarness::<WordCountState>::aggregate(
        ["hello world", "one", "two three four", ""],
        |s, text| s.count += count_words(text),
    );
    assert_eq!(result.count, 6);  // 2 + 1 + 3 + 0
}
```

### Testing `combine` (Pitfall L1)

DuckDB creates fresh zero-initialized target states and calls `combine` to merge
into them. You MUST propagate ALL fields — including configuration fields —
not just accumulated data. Test this explicitly:

```rust
#[test]
fn combine_propagates_config() {
    let mut h1 = AggregateTestHarness::<MyState>::new();
    h1.update(|s| {
        s.window_size = 3600;  // config field
        s.count += 5;          // data field
    });

    // h2 simulates a fresh zero-initialized state created by DuckDB
    let mut h2 = AggregateTestHarness::<MyState>::new();

    h2.combine(&h1, |src, tgt| {
        tgt.window_size = src.window_size;  // MUST propagate config
        tgt.count += src.count;
    });

    let result = h2.finalize();
    assert_eq!(result.window_size, 3600);  // Would be 0 if forgotten
    assert_eq!(result.count, 5);
}
```

### Inspecting intermediate state

```rust
let mut h = AggregateTestHarness::<SumState>::new();
h.update(|s| s.total += 5);
assert_eq!(h.state().total, 5);   // borrow without consuming
h.update(|s| s.total += 3);
assert_eq!(h.state().total, 8);
```

### Resetting

```rust
let mut h = AggregateTestHarness::<SumState>::new();
h.update(|s| s.total = 999);
h.reset();
assert_eq!(h.state().total, 0);  // back to S::default()
```

### Pre-populating state

```rust
let initial = MyState { window_size: 3600, count: 0 };
let h = AggregateTestHarness::with_state(initial);
```

---

## Unit tests for scalar functions

Scalar logic is pure Rust — test it directly:

```rust
// From examples/hello-ext/src/lib.rs
pub fn count_words(s: &str) -> i64 {
    s.split_whitespace().count() as i64
}

#[test]
fn count_words_basic() {
    assert_eq!(count_words("hello world"), 2);
    assert_eq!(count_words("one"), 1);
    assert_eq!(count_words(""), 0);
}
```

---

## Unit tests for SQL macros

`SqlMacro::to_sql()` is pure Rust — no DuckDB connection needed:

```rust
use quack_rs::sql_macro::SqlMacro;

#[test]
fn scalar_macro_sql() {
    let m = SqlMacro::scalar("double_it", &["x"], "x * 2").unwrap();
    assert_eq!(m.to_sql(),
        "CREATE OR REPLACE MACRO double_it(x) AS (x * 2)");
}

#[test]
fn table_macro_sql() {
    let m = SqlMacro::table("recent", &["n"], "SELECT * FROM events LIMIT n").unwrap();
    assert_eq!(m.to_sql(),
        "CREATE OR REPLACE MACRO recent(n) AS TABLE SELECT * FROM events LIMIT n");
}
```

---

## E2E testing with SQLLogicTest

Community extensions are tested using DuckDB's
[SQLLogicTest](https://duckdb.org/docs/dev/sqllogictest/intro.html) format. This
format runs SQL directly in DuckDB and verifies output line-by-line.

### File location

```
test/sql/my_extension.test
```

### Format

```sql
# my_extension tests

require my_extension

statement ok
LOAD my_extension;

query I
SELECT my_function('hello world');
----
2
```

Directives:

| Directive | Meaning |
|-----------|---------|
| `require` | Skip test if extension not available |
| `statement ok` | SQL must succeed |
| `statement error` | SQL must fail |
| `query I` | Query returning one INTEGER column |
| `query II` | Query returning two columns |
| `query T` | Query returning one TEXT column |
| `----` | Expected output follows |

### Running E2E tests

```bash
# Build the extension
cargo build --release

# Load it in DuckDB CLI
duckdb -cmd "LOAD './target/release/libmy_extension.so';" test.sql
```

The community extension CI runs SQLLogicTest automatically. Each function must
have at least one test:

```sql
# Test NULL handling
query I
SELECT my_function(NULL);
----
NULL

# Test empty input
query I
SELECT my_function('');
----
0

# Test normal case
query I
SELECT my_function('hello world');
----
2
```

> **Pitfall P5** — SQLLogicTest does exact string matching. Copy expected values
> directly from DuckDB CLI output. NULL is represented as `NULL` (uppercase).
> Floats must match to the number of decimal places DuckDB outputs.

---

## Property-based testing with `proptest`

The `proptest` crate is well-suited for testing aggregate logic over arbitrary
inputs:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn saturating_never_panics(months: i32, days: i32, micros: i64) {
        let iv = DuckInterval { months, days, micros };
        // Must not panic for any input
        let _ = interval_to_micros_saturating(iv);
    }
}
```

quack-rs's own test suite uses proptest for interval conversion and aggregate
harness properties.

---

## What to test

| Scenario | Unit | E2E |
|----------|------|-----|
| NULL input → NULL output | | ✓ |
| Empty string | ✓ | ✓ |
| Unicode strings | ✓ | |
| Numeric edge cases (0, MAX, MIN) | ✓ | |
| Combine propagates config | ✓ | |
| Multi-group aggregation | | ✓ |
| Function registration success | | ✓ |
| Extension loads without crash | | ✓ |
| SQL macro produces correct output | ✓ (to_sql) | ✓ |

---

## Dev dependencies

```toml
[dev-dependencies]
quack_rs = { version = "0.1", features = [] }
proptest = "1"
```

The `testing` module is compiled unconditionally (not `#[cfg(test)]`) so it is
available as a dev-dependency to downstream crates.
