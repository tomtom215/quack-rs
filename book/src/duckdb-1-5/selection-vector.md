# Selection Vectors

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

A `SelectionVector` is a list of row indices used to logically reorder or filter
a data vector **without copying its payload** — the building block behind
DuckDB's zero-copy filtering. Extensions that implement custom filtering or
reordering in vectorized callbacks can allocate one, fill in the indices, and
hand its raw handle to the relevant DuckDB vector operations.

This is an advanced, low-level primitive; most extensions never need it.

## Allocating and filling

```rust,no_run
use quack_rs::selection_vector::SelectionVector;

// Select source rows 3, 1, 4, 1, 5 (in that order) — note repeats are allowed.
let mut sel = SelectionVector::new(5);
sel.as_mut_slice().copy_from_slice(&[3, 1, 4, 1, 5]);

assert_eq!(sel.len(), 5);
assert_eq!(sel.as_slice(), &[3, 1, 4, 1, 5]);
```

The indices are 32-bit (`sel_t` / `u32`) and are **uninitialised** after `new` —
fill them via `as_mut_slice()` before use.

## API

| Method | Description |
|--------|-------------|
| `SelectionVector::new(size)` | Allocate a vector of `size` indices |
| `len()` / `is_empty()` | Number of indices |
| `as_slice()` | `&[u32]` — read the indices |
| `as_mut_slice()` | `&mut [u32]` — fill the indices |
| `as_raw()` | The raw `duckdb_selection_vector` handle for DuckDB vector ops |

`SelectionVector` is RAII: it is destroyed on drop.

## Related modules

- [Reading & Writing Vectors](../data/vectors.md) — the data vectors a selection
  vector reorders or filters
- [Complex Types](../data/complex-types.md) — STRUCT / LIST / MAP / ARRAY vectors
