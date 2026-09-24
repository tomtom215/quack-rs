# Dates, Times and Timestamps

`VectorReader` and `VectorWriter` move DuckDB's temporal types as the raw
integers DuckDB stores:

| SQL type | Storage | Accessor |
|----------|---------|----------|
| `DATE` | `i32` — days since 1970-01-01 | `read_date` / `write_date` |
| `TIME` | `i64` — microseconds since midnight | `read_time` / `write_time` |
| `TIMETZ` | packed `u64` | `read_time_tz` / `write_time_tz` |
| `TIMESTAMP` | `i64` — microseconds since the epoch | `read_timestamp` / `write_timestamp` |
| `TIMESTAMPTZ` | `i64` — microseconds since the epoch, UTC | `read_timestamp_tz` / `write_timestamp_tz` |
| `TIMESTAMP_S` | `i64` — seconds | `read_timestamp_s` / `write_timestamp_s` |
| `TIMESTAMP_MS` | `i64` — milliseconds | `read_timestamp_ms` / `write_timestamp_ms` |
| `TIMESTAMP_NS` | `i64` — nanoseconds | `read_timestamp_ns` / `write_timestamp_ns` |
| `INTERVAL` | `{ months: i32, days: i32, micros: i64 }` | `read_interval` / `write_interval` |

Turning those integers into year/month/day means implementing the proleptic
Gregorian calendar, and getting it to agree with DuckDB's SQL semantics exactly
rather than approximately. DuckDB already exposes the conversions, and they are
in the [stable prefix](../concepts/abi.md) of the C API, so `quack_rs::datetime`
wraps them rather than reimplementing anything.

## Decomposing and composing

```rust
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(reader: &VectorReader, writer: &mut VectorWriter, row: usize) {
use quack_rs::datetime;

// DATE -> calendar date
let days = unsafe { reader.read_date(row) };
let date = unsafe { datetime::date_from_days(days) };
println!("{:04}-{:02}-{:02}", date.year, date.month, date.day);

// …and back. `None` means DuckDB cannot represent the date.
match unsafe { datetime::date_to_days(date) } {
    Some(days) => unsafe { writer.write_date(row, days) },
    None => unsafe { writer.set_null(row) },
}
# }
```

`Time`, `TimeTz` and `Timestamp` work the same way:

```rust
# use quack_rs::datetime;
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(reader: &VectorReader, writer: &mut VectorWriter, rows: usize) {
# for row in 0..rows {
let Some(ts) = (unsafe { datetime::timestamp_from_micros(reader.read_timestamp(row)) }) else {
    // ±infinity (or the first ~4 hours of the i64 range): no calendar form.
    unsafe { writer.set_null(row) };
    continue;
};
assert!((0..1_000_000).contains(&ts.time.micros));   // ts.date and ts.time are plain structs

let micros = unsafe { datetime::timestamp_to_micros(ts) };   // Option<i64>
# }
# }
```

### Invalid input is `None`, not an abort

Several of DuckDB's conversions **throw a C++ exception** on bad input, and the
C API does not catch it — so calling them directly with, say, month 13 aborts
the whole process ("Rust cannot catch foreign exceptions"). The wrappers check
first, using DuckDB's own conditions, and return `None` instead:

| Function | Returns `None` when |
|----------|---------------------|
| `date_to_days` | month not 1–12, day not in that month (leap years included), or the date is outside 5877642-06-25 BC – 5881580-07-10; `datetime::is_valid_date` is the same check |
| `timestamp_from_micros` | the value is `±infinity`, or below `-106_751_991 * MICROS_PER_DAY` (which includes `i64::MIN`) |
| `timestamp_to_micros` | the date is invalid, the result overflows `i64`, or it lands on `±infinity` |
| `time_from_micros` | the value is outside `0..=MICROS_PER_DAY` (`00:00:00`–`24:00:00`) |
| `time_tz_bits` | the time is outside `0..=MICROS_PER_DAY`, or the offset beyond ±15:59:59 (`TIME_TZ_MAX_OFFSET_SECONDS`) |
| `time_tz_from_bits` | the bits decode to a time or offset that `time_tz_bits` would refuse |
| `decimal_to_f64` | `width > 38` or `scale > width` |

`time_from_micros` and `time_tz_from_bits` guard an assertion rather than an
exception: a release build of DuckDB decomposes an out-of-range time into
out-of-range fields, and a build with assertions enabled aborts.

`time_to_micros` does no range check, exactly like DuckDB: an hour of 25
simply gives a `TIME` past midnight.

`TIMETZ` is a packed 64-bit value, not a plain integer — build and read it
through the helpers rather than by hand:

```rust
# use quack_rs::datetime;
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(reader: &VectorReader, writer: &mut VectorWriter, row: usize) {
let bits = unsafe { datetime::time_tz_bits(12 * 3_600 * 1_000_000, -5 * 3_600) }
    .expect("noon, UTC-5, is in range");
unsafe { writer.write_time_tz(row, bits) };

let decoded = unsafe { datetime::time_tz_from_bits(reader.read_time_tz(row)) }
    .expect("DuckDB wrote a valid TIMETZ");
assert_eq!(decoded.offset_seconds, -5 * 3_600);
# }
```

## Infinity

DuckDB reserves two values of `DATE` and of `TIMESTAMP` for `infinity` and
`-infinity`. Decomposing one into a calendar date is meaningless, so check first:

```rust
# use quack_rs::datetime;
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(reader: &VectorReader, writer: &mut VectorWriter, row: usize) {
let days = unsafe { reader.read_date(row) };
if unsafe { datetime::is_finite_date(days) } {
    let date = unsafe { datetime::date_from_days(days) };
    // …
}
# }
```

Note the exact values, which are easy to get wrong:

| Constant | Value |
|----------|-------|
| `DATE_INFINITY_DAYS` | `i32::MAX` |
| `DATE_NEGATIVE_INFINITY_DAYS` | `-i32::MAX` |
| `TIMESTAMP_INFINITY_MICROS` | `i64::MAX` |
| `TIMESTAMP_NEGATIVE_INFINITY_MICROS` | `-i64::MAX` |

Negative infinity is `-i32::MAX`, **not** `i32::MIN`. `i32::MIN` is an ordinary
(if absurd) finite date, and treating it as infinity would silently drop real
rows.

## DECIMAL

`DECIMAL` is stored in the narrowest integer that fits its declared width, so the
width has to travel with the value:

| Declared width | Physical storage |
|----------------|------------------|
| 1 – 4 | `i16` |
| 5 – 9 | `i32` |
| 10 – 18 | `i64` |
| 19 – 38 | `i128` |

`read_decimal` / `write_decimal` take the width and pick the right one. Get it
from the column's `LogicalType`:

```rust
# use libduckdb_sys::duckdb_vector;
# use quack_rs::vector::{VectorReader, VectorWriter};
# fn demo(vec: duckdb_vector, reader: &VectorReader, writer: &mut VectorWriter, row: usize) {
let logical = unsafe { quack_rs::vector::vector_get_column_type(vec) };
let width = unsafe { logical.decimal_width() };
let scale = unsafe { logical.decimal_scale() };

let unscaled = unsafe { reader.read_decimal(row, width) };
// The represented number is unscaled / 10^scale.
unsafe { writer.write_decimal(row, width, unscaled * 2) };
# }
```

`datetime::f64_to_decimal` and `datetime::decimal_to_f64` convert through
DuckDB's own routines when a floating-point view is what you want.
`decimal_to_f64` returns `None` for a width above 38 or a scale above the
width: DuckDB would index its powers-of-ten table out of bounds.

## Wide integers

`HUGEINT` is `{ lower: u64, upper: i64 }` and `UHUGEINT` is two `u64`s.
`read_i128` / `write_i128` and `read_u128` / `write_u128` handle the halves;
`datetime::hugeint_to_f64` and friends match DuckDB's own conversion behaviour
including its rounding.
