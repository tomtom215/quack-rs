# INTERVAL Type

DuckDB's `INTERVAL` type represents a duration with three independent components:
months, days, and sub-day microseconds. The `quack_rs::interval` module provides
the `DuckInterval` struct and safe conversion utilities.

---

## Why a custom struct?

> **Pitfall P8** — The `INTERVAL` struct layout and its conversion semantics are
> not documented in the Rust bindings. This module encodes that knowledge.

DuckDB's C `duckdb_interval` struct is 16 bytes with this exact layout:

```
offset 0:  months (i32)  — calendar months
offset 4:  days   (i32)  — calendar days
offset 8:  micros (i64)  — sub-day microseconds
total:     16 bytes
```

`DuckInterval` is `#[repr(C)]` with the same field order and is verified at
compile time to be exactly 16 bytes.

---

## Reading INTERVAL values

```rust
let iv: DuckInterval = unsafe { reader.read_interval(row) };
println!("{} months, {} days, {} µs", iv.months, iv.days, iv.micros);
```

`VectorReader::read_interval` handles the raw pointer arithmetic and alignment
using `read_interval_at` internally.

---

## DuckInterval fields

```rust
use quack_rs::interval::DuckInterval;

let iv = DuckInterval {
    months: 1,    // 1 calendar month
    days: 15,     // 15 calendar days
    micros: 3600_000_000,  // 1 hour in microseconds
};
```

Fields are public and can be constructed directly.

### Zero interval

```rust
let zero = DuckInterval::zero();    // { months: 0, days: 0, micros: 0 }
let zero = DuckInterval::default(); // same
```

---

## Converting to microseconds

Intervals are not directly comparable because months and days have variable
lengths in wall-clock time. When you need a single numeric value, convert to
microseconds using the DuckDB approximation: **1 month = 30 days**.

### Checked conversion (returns `Option`)

```rust
use quack_rs::interval::interval_to_micros;

let iv = DuckInterval { months: 0, days: 1, micros: 500_000 };
match interval_to_micros(iv) {
    Some(us) => println!("{us} microseconds"),
    None => println!("overflow"),
}

// Method form:
let us: Option<i64> = iv.to_micros();
```

Returns `None` if the result would overflow `i64`. This can happen with extreme
values (e.g., `months: i32::MAX`).

### Saturating conversion (never panics)

```rust
use quack_rs::interval::interval_to_micros_saturating;

let iv = DuckInterval { months: i32::MAX, days: i32::MAX, micros: i64::MAX };
let us: i64 = interval_to_micros_saturating(iv); // i64::MAX

// Method form:
let us: i64 = iv.to_micros_saturating();
```

Use the saturating form in FFI callbacks where panics are not allowed.

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
use quack_rs::interval::read_interval_at;

// SAFETY: data is a valid DuckDB INTERVAL vector data pointer, idx is in bounds.
let iv = unsafe { read_interval_at(data_ptr, row_idx) };
```

In practice you should use `VectorReader::read_interval(row)` instead, which
handles all safety invariants.

---

## Complete example: aggregate over INTERVAL

```rust
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

`DuckInterval` includes a compile-time assertion that validates its size and
alignment against DuckDB's C struct. If the assertion fails, the crate will not
compile — catching any future mismatch at build time rather than runtime.
