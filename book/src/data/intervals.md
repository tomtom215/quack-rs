# INTERVAL Type

DuckDB's `INTERVAL` type represents a duration with three independent components:
months, days and microseconds. The `quack_rs::interval` module provides the
`DuckInterval` struct, which matches DuckDB's in-memory layout, and overflow-safe
conversions to microseconds.

---

## Why a custom struct?

> **Pitfall P8**: the Rust bindings do not document the `INTERVAL` layout or how
> DuckDB converts intervals. `DuckInterval` and the functions below encode both.
> See [Pitfall P8](../reference/pitfalls.md#p8-interval-struct-layout-is-undocumented).

DuckDB's C `duckdb_interval` struct is 16 bytes with this exact layout:

```text
offset 0:  months (i32)  — calendar months
offset 4:  days   (i32)  — calendar days
offset 8:  micros (i64)  — microseconds (not limited to one day)
total:     16 bytes
```

`DuckInterval` is `#[repr(C)]` with the same field order, and a compile-time
assertion checks that it is exactly 16 bytes.

---

## Reading INTERVAL values

```rust
# use quack_rs::interval::DuckInterval;
# use quack_rs::vector::VectorReader;
# fn demo(reader: &VectorReader, row: usize) {
let iv: DuckInterval = unsafe { reader.read_interval(row) };
println!("{} months, {} days, {} µs", iv.months, iv.days, iv.micros);
# }
```

`VectorReader::read_interval` handles the raw pointer arithmetic and alignment
using `read_interval_at` internally.

---

## `DuckInterval` fields

```rust
use quack_rs::interval::DuckInterval;

let iv = DuckInterval {
    months: 1,    // 1 calendar month
    days: 15,     // 15 calendar days
    micros: 3_600_000_000, // 1 hour in microseconds
};
```

The fields are public, so a `DuckInterval` can be built directly.

The derived `PartialEq`, `Eq` and `Hash` compare the three fields, so
`{ months: 1, .. }` and `{ days: 30, .. }` are different values here, while in
SQL `INTERVAL '1 month' = INTERVAL '30 days'` is true.

### Zero interval

```rust
# use quack_rs::interval::DuckInterval;
let zero = DuckInterval::zero();    // { months: 0, days: 0, micros: 0 }
let zero = DuckInterval::default(); // same
# assert_eq!(zero, DuckInterval::zero());
```

---

## Converting to microseconds

Months and days have no fixed length in wall-clock time, so an interval has no
single exact length. When you need one number, for ordering or bucketing,
convert to microseconds with the approximation DuckDB uses when it compares
intervals and in `epoch_us(interval)`: **1 month = 30 days**.

This is not date arithmetic: DuckDB adds an interval to a date by calendar
months (`DATE '2024-01-31' + INTERVAL 1 MONTH` is `2024-02-29`). It also matches
SQL comparison only when the three fields share a sign: `INTERVAL '1 month' -
INTERVAL '1 day'` converts to the same total as `INTERVAL '29 days'` but compares
greater in SQL.

### Checked conversion (returns `Option`)

```rust
# use quack_rs::interval::DuckInterval;
use quack_rs::interval::interval_to_micros;

let iv = DuckInterval { months: 0, days: 1, micros: 500_000 };
match interval_to_micros(iv) {
    Some(us) => println!("{us} microseconds"),
    None => println!("overflow"),
}

// Method form:
let us: Option<i64> = iv.to_micros();
# assert_eq!(us, Some(86_400_000_000 + 500_000));
```

Returns `None` if the total does not fit in an `i64`, which takes extreme values
such as `months: i32::MAX, days: i32::MAX, micros: i64::MAX`. The sum is computed
exactly, so large fields of opposite signs that cancel out still convert.

### Saturating conversion (returns `i64`)

```rust
# use quack_rs::interval::DuckInterval;
use quack_rs::interval::interval_to_micros_saturating;

let iv = DuckInterval { months: i32::MAX, days: i32::MAX, micros: i64::MAX };
let us: i64 = interval_to_micros_saturating(iv); // i64::MAX

// Method form:
let us: i64 = iv.to_micros_saturating();
# assert_eq!(us, i64::MAX);
# assert_eq!(interval_to_micros_saturating(iv), i64::MAX);
```

The saturating form clamps an out-of-range total to `i64::MAX` or `i64::MIN`.
Neither form panics; use the checked form when an overflow must be reported
rather than clamped.

---

## Conversion constants

| Constant | Value | Meaning |
|----------|-------|---------|
| `MICROS_PER_DAY` | `86_400_000_000` | Microseconds in 24 hours |
| `MICROS_PER_MONTH` | `2_592_000_000_000` | Microseconds in 30 days |

```rust
use quack_rs::interval::{MICROS_PER_DAY, MICROS_PER_MONTH};

assert_eq!(MICROS_PER_DAY, 86_400 * 1_000_000);
assert_eq!(MICROS_PER_MONTH, 30 * MICROS_PER_DAY);
```

---

## Low-level: `read_interval_at`

If you have a raw data pointer (e.g., from `duckdb_vector_get_data`), you can
read an interval directly:

```rust
# fn demo(data_ptr: *const u8, row_idx: usize) {
use quack_rs::interval::read_interval_at;

// SAFETY: data is a valid DuckDB INTERVAL vector data pointer, idx is in bounds.
let iv = unsafe { read_interval_at(data_ptr, row_idx) };
# }
```

In practice, use `VectorReader::read_interval(row)`, which computes the data
pointer for you; its remaining `unsafe` contract is the row index and the
column type.

---

## Complete example: aggregate over INTERVAL

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info};
# use quack_rs::aggregate::{AggregateState, FfiState};
# use quack_rs::vector::VectorReader;
#[derive(Default)]
struct TotalDurationState {
    total_micros: i64,
}
impl AggregateState for TotalDurationState {}

unsafe extern "C" fn update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let reader = unsafe { VectorReader::new(input, 0) };
    for row in 0..reader.row_count() {
        if unsafe { !reader.is_valid(row) } { continue; }
        let iv = unsafe { reader.read_interval(row) };
        let us = iv.to_micros_saturating();
        let state_ptr = unsafe { *states.add(row) };
        if let Some(st) = unsafe { FfiState::<TotalDurationState>::with_state_mut(state_ptr) } {
            st.total_micros = st.total_micros.saturating_add(us);
        }
    }
}
```

---

## Memory layout verification

A compile-time assertion checks that `DuckInterval` is 16 bytes with at least
4-byte alignment, the layout of DuckDB's `duckdb_interval`. If it fails, the
crate does not compile, so a layout change is caught at build time rather than
at run time.
