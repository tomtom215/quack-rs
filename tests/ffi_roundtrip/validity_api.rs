// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ValidityBitmap` — `ensure_writable`, `get_read_only`, `row_is_valid`,
//! `set_row_invalid`, `set_row_valid` and `as_raw` — against live `DuckDB`
//! vectors: the bits it flips are the NULLs SQL sees, on both sides of the
//! 64-row word and the vector-size edges.

use libduckdb_sys::idx_t;
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::{StructWriter, ValidityBitmap, VectorWriter};

use super::vector_support::{count, edge_rows};
use super::Fixture;

/// Row counts on both sides of the 64-row validity word, then the vector edges.
fn word_and_vector_edges() -> Vec<usize> {
    let mut rows = vec![63, 64, 65, 127, 128, 129];
    rows.extend(edge_rows());
    rows
}

/// Whether the model leaves row `x` NULL: `x % 3 == 0` marks it invalid, and
/// `x % 6 == 0` marks it valid again.
const fn is_null_after_undo(x: usize) -> bool {
    x % 3 == 0 && x % 6 != 0
}

// Writes `x * 10` to every row, then NULLs rows through the bitmap: invalid
// where `x % 3 == 0`, valid again where `x % 6 == 0`.
quack_rs::scalar_callback!(bitmap_nulls, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        unsafe { writer.write_i64(row, x * 10) };
    }

    // Nothing has been marked NULL in this vector yet: every row reads valid
    // through a read-only view, whether or not the mask exists.
    let before = unsafe { ValidityBitmap::get_read_only(output) };
    for row in 0..rows {
        assert!(unsafe { before.row_is_valid(row as idx_t) }, "row {row}");
    }

    let mut bitmap = unsafe { ValidityBitmap::ensure_writable(output) };
    assert!(
        !bitmap.as_raw().is_null(),
        "ensure_writable allocates the mask"
    );
    for row in 0..rows {
        assert!(
            unsafe { bitmap.row_is_valid(row as idx_t) },
            "fresh mask: row {row}"
        );
    }
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        if x % 3 == 0 {
            unsafe { bitmap.set_row_invalid(row as idx_t) };
        }
    }
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        if x % 6 == 0 {
            unsafe { bitmap.set_row_valid(row as idx_t) };
        }
    }
    // The bits read back as written, and a fresh read-only view of the same
    // vector is the same mask.
    let after = unsafe { ValidityBitmap::get_read_only(output) };
    assert_eq!(after.as_raw(), bitmap.as_raw());
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) } as usize;
        let want_valid = !is_null_after_undo(x);
        assert_eq!(
            unsafe { bitmap.row_is_valid(row as idx_t) },
            want_valid,
            "row {row}"
        );
        assert_eq!(
            unsafe { after.row_is_valid(row as idx_t) },
            want_valid,
            "row {row}"
        );
    }
});

fn register_bigint_fn(fx: &Fixture, name: &str, callback: quack_rs::scalar::ScalarFn) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("valid function name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

#[test]
fn bitmap_set_row_invalid_and_valid_make_the_nulls_sql_sees() {
    let fx = Fixture::open();
    register_bigint_fn(&fx, "bitmap_nulls", bitmap_nulls);
    for n in word_and_vector_edges() {
        // DuckDB's own view of the result: NULL exactly where the model says.
        let wrong_nulls = count(
            &fx,
            &format!(
                "SELECT count(*) FROM range({n}) t(x) WHERE \
                 (bitmap_nulls(x) IS NULL) != (x % 3 = 0 AND x % 6 != 0)"
            ),
        );
        assert_eq!(wrong_nulls, 0, "n={n}");
        // Rows that stayed valid kept the value written next to the bitmap.
        let wrong_values = count(
            &fx,
            &format!(
                "SELECT count(*) FROM range({n}) t(x) WHERE \
                 bitmap_nulls(x) IS DISTINCT FROM CASE WHEN x % 3 != 0 OR x % 6 = 0 THEN x * 10 END"
            ),
        );
        assert_eq!(wrong_values, 0, "n={n}");
        // Not vacuous: the model's NULLs are really there.
        let nulls = count(
            &fx,
            &format!("SELECT count(*) FROM range({n}) t(x) WHERE bitmap_nulls(x) IS NULL"),
        );
        assert_eq!(
            nulls,
            (0..n).filter(|&x| is_null_after_undo(x)).count() as i64,
            "n={n}"
        );
    }
}

// Returns 1 for a row whose input is valid and 0 for one that is NULL, read
// through a read-only bitmap on the input vector, and checks the bitmap's own
// pointer against what `VectorReader` sees.
quack_rs::scalar_callback!(bitmap_reads_input, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let bitmap = unsafe { ValidityBitmap::get_read_only(chunk.vector(0)) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        let valid = unsafe { bitmap.row_is_valid(row as idx_t) };
        assert_eq!(valid, unsafe { reader.is_valid(row) }, "row {row}");
        unsafe { writer.write_i64(row, i64::from(valid)) };
    }
});

#[test]
fn bitmap_get_read_only_reads_the_nulls_of_an_input_column() {
    let fx = Fixture::open();
    register_bigint_fn(&fx, "bitmap_reads_input", bitmap_reads_input);
    for n in word_and_vector_edges() {
        // Some NULLs; none (no mask at all); all.
        for (label, input, nulls) in [
            (
                "some",
                "CASE WHEN x % 4 = 1 OR x % 9 = 5 THEN NULL ELSE x END",
                (0..n).filter(|x| x % 4 == 1 || x % 9 == 5).count(),
            ),
            ("none", "x", 0),
            ("all", "CASE WHEN x >= 0 THEN NULL END::BIGINT", n),
        ] {
            let table = format!("(SELECT {input} AS v, x FROM range({n}) t(x))");
            let wrong = count(
                &fx,
                &format!(
                    "SELECT count(*) FROM {table} WHERE (bitmap_reads_input(v) = 1) != (v IS NOT NULL)"
                ),
            );
            assert_eq!(wrong, 0, "{label} n={n}");
            let seen_valid = count(
                &fx,
                &format!("SELECT count(*) FROM {table} WHERE bitmap_reads_input(v) = 1"),
            );
            assert_eq!(seen_valid, (n - nulls) as i64, "{label} n={n}");
        }
    }
}

// Marks the parent row of a STRUCT NULL through a bare `ValidityBitmap` — which,
// as its docs say, touches only this one mask — after writing field `a`.
quack_rs::scalar_callback!(bitmap_nulls_only_the_parent, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut fields = unsafe { StructWriter::new(output, 1) };
    let mut bitmap = unsafe { ValidityBitmap::ensure_writable(output) };
    for row in 0..chunk.size() {
        let x = unsafe { reader.read_i64(row) };
        unsafe { fields.write_i64(row, 0, x) };
        if x % 2 == 1 {
            unsafe { bitmap.set_row_invalid(row as idx_t) };
        }
    }
});

#[test]
fn bitmap_set_row_invalid_on_a_struct_leaves_its_fields_alone() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::new("bitmap_nulls_only_the_parent")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::struct_type(&[("a", TypeId::BigInt)]))
            .function(bitmap_nulls_only_the_parent)
            .register(fx.con())
            .expect("register");
    }
    for n in edge_rows() {
        let odd = (0..n).filter(|x| x % 2 == 1).count() as i64;
        // The struct is NULL on odd rows ...
        assert_eq!(
            count(
                &fx,
                &format!(
                    "SELECT count(*) FROM range({n}) t(x) WHERE bitmap_nulls_only_the_parent(x) IS NULL"
                )
            ),
            odd,
            "n={n}: NULL structs"
        );
        // ... and, because only the parent's bit was cleared, the field still
        // holds what was written: `VectorWriter::set_null` is the call that
        // clears the children too.
        assert_eq!(
            count(
                &fx,
                &format!(
                    "SELECT count(*) FROM range({n}) t(x) \
                     WHERE (bitmap_nulls_only_the_parent(x)).a = x"
                )
            ),
            n as i64,
            "n={n}: fields"
        );
    }
}

// ─── Directly on an OwnedVector ──────────────────────────────────────────────

#[cfg(feature = "duckdb-1-5")]
mod owned {
    use libduckdb_sys::{duckdb_validity_row_is_valid, duckdb_vector_get_validity, idx_t};
    use quack_rs::types::{LogicalType, TypeId};
    use quack_rs::vector::{OwnedVector, ValidityBitmap};

    use super::Fixture;

    const CAPACITY: usize = 4100;

    /// The rows that cross a word edge, a vector edge, or both.
    const INVALID: [usize; 12] = [0, 1, 62, 63, 64, 65, 127, 128, 2047, 2048, 2049, 4099];
    const REVALIDATED: [usize; 4] = [1, 64, 2048, 4099];

    /// What `DuckDB` itself reports for `vector`'s validity, row by row.
    fn engine_view(vector: libduckdb_sys::duckdb_vector) -> Vec<bool> {
        // SAFETY: `vector` is a live vector with `CAPACITY` rows; a NULL mask
        // reads as all valid.
        let mask = unsafe { duckdb_vector_get_validity(vector) };
        (0..CAPACITY as idx_t)
            .map(|r| mask.is_null() || unsafe { duckdb_validity_row_is_valid(mask, r) })
            .collect()
    }

    #[test]
    fn ensure_writable_allocates_the_mask_and_set_row_flips_single_bits() {
        let _fx = Fixture::open();
        let vector = OwnedVector::new(&LogicalType::new(TypeId::BigInt), CAPACITY).expect("vector");
        let raw = vector.as_raw();

        // A fresh vector has no mask: a read-only view is the null pointer,
        // and reads every row as valid.
        // SAFETY: `raw` is a live BIGINT vector.
        let fresh = unsafe { ValidityBitmap::get_read_only(raw) };
        assert!(
            fresh.as_raw().is_null(),
            "a vector no NULL was written to has no mask"
        );
        assert!((0..CAPACITY).all(|r| unsafe { fresh.row_is_valid(r as idx_t) }));

        // SAFETY: as above.
        let mut bitmap = unsafe { ValidityBitmap::ensure_writable(raw) };
        assert!(!bitmap.as_raw().is_null());
        assert_eq!(
            bitmap.as_raw(),
            unsafe { duckdb_vector_get_validity(raw) },
            "as_raw is the engine's own mask"
        );
        assert!(
            engine_view(raw).iter().all(|&v| v),
            "a new mask is all valid"
        );

        let mut model = vec![true; CAPACITY];
        for &r in &INVALID {
            // SAFETY: every row is below CAPACITY, the bitmap is writable.
            unsafe { bitmap.set_row_invalid(r as idx_t) };
            model[r] = false;
        }
        assert_eq!(engine_view(raw), model, "after set_row_invalid");
        for &r in &REVALIDATED {
            // SAFETY: as above.
            unsafe { bitmap.set_row_valid(r as idx_t) };
            model[r] = true;
        }
        assert_eq!(engine_view(raw), model, "after set_row_valid");

        // The bitmap reports the same bits, and so does a new read-only view.
        // SAFETY: `raw` is still live.
        let again = unsafe { ValidityBitmap::get_read_only(raw) };
        assert_eq!(again.as_raw(), bitmap.as_raw());
        for (r, &want) in model.iter().enumerate() {
            assert_eq!(unsafe { bitmap.row_is_valid(r as idx_t) }, want, "row {r}");
            assert_eq!(unsafe { again.row_is_valid(r as idx_t) }, want, "row {r}");
        }
    }

    /// Setting a bit that is already in the requested state changes nothing.
    #[test]
    fn set_row_invalid_and_valid_are_idempotent() {
        let _fx = Fixture::open();
        let vector = OwnedVector::new(&LogicalType::new(TypeId::Integer), 130).expect("vector");
        // SAFETY: a live INTEGER vector with 130 rows.
        let mut bitmap = unsafe { ValidityBitmap::ensure_writable(vector.as_raw()) };
        // SAFETY: rows are below 130, the bitmap is writable.
        unsafe {
            bitmap.set_row_valid(5);
            bitmap.set_row_invalid(129);
            bitmap.set_row_invalid(129);
            assert!(bitmap.row_is_valid(5));
            assert!(!bitmap.row_is_valid(129));
            bitmap.set_row_valid(129);
            bitmap.set_row_valid(129);
            assert!(bitmap.row_is_valid(129));
            // The neighbours in the same word never moved.
            assert!(bitmap.row_is_valid(128));
            assert!(bitmap.row_is_valid(64));
        }
    }

    /// `set_row_invalid` on a read-only bitmap of a vector with no mask would
    /// write through a null pointer; debug builds refuse it.
    #[cfg(debug_assertions)]
    #[test]
    fn set_row_invalid_on_a_read_only_bitmap_without_a_mask_panics_in_debug() {
        let _fx = Fixture::open();
        let vector = OwnedVector::new(&LogicalType::new(TypeId::BigInt), 8).expect("vector");
        // SAFETY: a live BIGINT vector.
        let mut read_only = unsafe { ValidityBitmap::get_read_only(vector.as_raw()) };
        assert!(read_only.as_raw().is_null());
        for call in [0_u8, 1] {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // SAFETY: row 0 is in range; the debug assertion fires before
                // the null mask is touched.
                unsafe {
                    if call == 0 {
                        read_only.set_row_invalid(0);
                    } else {
                        read_only.set_row_valid(0);
                    }
                }
            }));
            assert!(outcome.is_err(), "call {call} must hit the debug assertion");
        }
    }
}
