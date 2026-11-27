<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215) -->

# Numbers: What Is Checked and What Is Yours

A fixed-width type is not arithmetic correctness. This page says which numeric
conversions quack-rs checks for you, which it leaves unchecked on purpose, and which
decisions only the extension can make. Each line names the API it is about, so it can be
checked against that API's documentation.

## Checked for you

| Operation | API | What happens to a value that does not fit |
|---|---|---|
| Write a `DECIMAL` | `VectorWriter::decimal()` then `DecimalWriter::write` | `Err`, nothing written; width and scale come from the vector's type |
| Make a `DECIMAL` value | `Value::decimal(width, scale, unscaled)` | `Err` |
| Bind a `DECIMAL` | `PreparedStatement::bind_decimal` | `Err` |
| Make a time or timestamp value | `Value::time*` and `Value::timestamp*` constructors | `Err` outside DuckDB's range (`Value::date` takes any `i32` day number) |
| Make a `VARCHAR` / `BLOB` value | `Value::try_varchar`, `Value::try_blob` | `Err` past 4 GiB (the plain constructors panic) |
| `f64` to `HUGEINT` / `UHUGEINT` | `datetime::try_f64_to_hugeint`, `try_f64_to_uhugeint` | `None` for NaN, an infinity or a value out of range |
| Interval to microseconds | `DuckInterval::to_micros`, `interval::interval_to_micros` | `None` on `i64` overflow (the `_saturating` forms clamp instead) |

## Unchecked on purpose

| Operation | API | Why, and what to do instead |
|---|---|---|
| Write a `DECIMAL` with a given width | `VectorWriter::write_decimal(idx, width, v)` | The hot path for a value already known to fit, such as one read from a column of the same width. A value too wide is stored wrong, and a wrong width writes the wrong integer size. Use `decimal()` for anything computed. |
| `f64` to `HUGEINT` | `datetime::f64_to_hugeint` | Mirrors DuckDB's conversion, which truncates toward zero and answers `0` for a value it cannot convert. Use the `try_` form to tell that `0` from a real one. |
| Interval to microseconds | `DuckInterval::to_micros` | Counts a month as 30 days. A month or a day is not a fixed duration in a calendar, so this is an approximation by definition. |

## Yours to decide

- **Rounding and rescaling.** No quack-rs writer rounds. A `DECIMAL` result computed at
  another scale must be rescaled by your code, with the tie rule your domain requires
  (half-even and half-away-from-zero give different answers at `.5`). Compute in integers:
  derive the unscaled value from decimal text or integer arithmetic, never through an
  `f64`.
- **Overflow in your own arithmetic.** Rust's release profile wraps on integer overflow
  unless `overflow-checks = true` is set; the profile `quack_scaffold` generates sets it
  (from 0.21.0), so an overflow panics, which the callback macros report as the query's
  error. That is a backstop, and a profile in your own `Cargo.toml` decides it, not
  quack-rs. Use `checked_*` operations and report the error yourself for values you
  expect to reject. The book's `DECIMAL` example uses `checked_mul`.
- **Decimal division.** Plain `/` on two `DECIMAL`s returns a `DOUBLE` (on 2.0.1,
  `10.00::DECIMAL(18, 2) / 4.00::DECIMAL(18, 2)` is typed `DOUBLE`). DuckDB 2.0 adds
  `decimal_division(a, b, scale)`, which returns a `DECIMAL` rounded half to even:
  `decimal_division(1.25, 1.00, 1)` is `1.2`, where `round(1.25, 1)` is `1.3`. Where
  DuckDB already computes what you need with the rounding you need, call it rather than
  re-implementing it. 1.5.6 does not have `decimal_division`.
- **Floating-point aggregates are not reproducible across plans.** DuckDB decides how rows
  are split into aggregate states and in which order `combine` merges them (threads,
  partitions, spilling); quack-rs does not fix either. Floating-point addition is not
  associative, so an `f64` sum can differ in its last bits between thread counts or runs.
  When the answer must be exact or reproducible, accumulate in integers (`i128` unscaled
  `DECIMAL`), and make `combine` order-independent.
- **NaN, infinities and signed zero.** `DOUBLE` and `FLOAT` columns hold them, and the
  vector writers store whatever `f64` they are given. Decide whether your function accepts
  them, and test that decision.
- **Units, currencies and time zones.** None is part of a numeric type. Carry them in the
  schema or the function's contract.
