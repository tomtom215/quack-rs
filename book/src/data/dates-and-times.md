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

```rust,ignore
use quack_rs::datetime;

// DATE -> calendar date
let days = unsafe { reader.read_date(row) };
let date = unsafe { datetime::date_from_days(days) };
println!("{:04}-{:02}-{:02}", date.year, date.month, date.day);

// …and back
let days = unsafe { datetime::date_to_days(date) };
unsafe { writer.write_date(row, days) };
```

`Time`, `TimeTz` and `Timestamp` work the same way:

```rust,ignore
let ts = unsafe { datetime::timestamp_from_micros(reader.read_timestamp(row)) };
assert_eq!(ts.time.micros % 1_000, 0);   // ts.date and ts.time are plain structs

let micros = unsafe { datetime::timestamp_to_micros(ts) };
```

`TIMETZ` is a packed 64-bit value, not a plain integer — build and read it
through the helpers rather than by hand:

```rust,ignore
let bits = unsafe { datetime::time_tz_bits(12 * 3_600 * 1_000_000, -5 * 3_600) };
unsafe { writer.write_time_tz(row, bits) };

let decoded = unsafe { datetime::time_tz_from_bits(reader.read_time_tz(row)) };
assert_eq!(decoded.offset_seconds, -5 * 3_600);
```

## Infinity

DuckDB reserves two values of `DATE` and of `TIMESTAMP` for `infinity` and
`-infinity`. Decomposing one into a calendar date is meaningless, so check first:

```rust,ignore
let days = unsafe { reader.read_date(row) };
if unsafe { datetime::is_finite_date(days) } {
    let date = unsafe { datetime::date_from_days(days) };
    // …
}
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

```rust,ignore
let logical = unsafe { quack_rs::vector::vector_get_column_type(vec) };
let width = unsafe { logical.decimal_width() };
let scale = unsafe { logical.decimal_scale() };

let unscaled = unsafe { reader.read_decimal(row, width) };
// The represented number is unscaled / 10^scale.
unsafe { writer.write_decimal(row, width, unscaled * 2) };
```

`datetime::f64_to_decimal` and `datetime::decimal_to_f64` convert through
DuckDB's own routines when a floating-point view is what you want.

## Wide integers

`HUGEINT` is `{ lower: u64, upper: i64 }` and `UHUGEINT` is two `u64`s.
`read_i128` / `write_i128` and `read_u128` / `write_u128` handle the halves;
`datetime::hugeint_to_f64` and friends match DuckDB's own conversion behaviour
including its rounding.
