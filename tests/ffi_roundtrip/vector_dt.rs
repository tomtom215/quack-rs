// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Regression tests for NULLs in nested output vectors and for selection
//! vectors.

use super::Fixture;
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::{StructWriter, VectorWriter};

/// Rows `0, 2, 4, 6, 8` NULL, the rest `i`: a NULL that comes from a column,
/// so `DuckDB` cannot constant-fold the call away.
const HALF_NULL: &str =
    "(SELECT CASE WHEN i % 2 = 0 THEN NULL ELSE i END AS x FROM range(10) t(i))";

// Writes 42 into field `a` of every row — NULL rows included — and then lets
// `propagate_nulls` null the rows whose input was NULL.
quack_rs::scalar_callback!(struct_always_42, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut fields = unsafe { StructWriter::new(output, 1) };
    for row in 0..chunk.size() {
        unsafe { fields.write_i64(row, 0, 42) };
    }
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    unsafe { chunk.propagate_nulls(&mut parent) };
});

// The same, but nulling rows through `StructWriter::set_row_null`.
quack_rs::scalar_callback!(struct_42_row_null, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut fields = unsafe { StructWriter::new(output, 1) };
    for row in 0..chunk.size() {
        unsafe { fields.write_i64(row, 0, 42) };
        if !unsafe { reader.is_valid(row) } {
            unsafe { fields.set_row_null(row) };
        }
    }
});

fn register_struct_fn(fx: &Fixture, name: &str, callback: quack_rs::scalar::ScalarFn) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::struct_type(&[("a", TypeId::BigInt)]))
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

/// A NULL struct row must read NULL through `struct_extract`, which reads the
/// field vector without looking at the parent. Before the fix only the parent
/// was nulled, and `(f(x)).a` returned the stale 42 for all 10 rows.
#[test]
fn a_null_struct_row_reads_null_fields() {
    let fx = Fixture::open();
    register_struct_fn(&fx, "struct_always_42", struct_always_42);
    register_struct_fn(&fx, "struct_42_row_null", struct_42_row_null);

    for name in ["struct_always_42", "struct_42_row_null"] {
        let field_not_null = fx.scalar(
            &format!("SELECT count(*) FILTER (WHERE ({name}(x)).a IS NOT NULL) FROM {HALF_NULL}"),
            |r, i| unsafe { r.read_i64(i) },
        );
        assert_eq!(field_not_null, Some(5), "{name}: field of a NULL row");

        let struct_null = fx.scalar(
            &format!("SELECT count(*) FILTER (WHERE {name}(x) IS NULL) FROM {HALF_NULL}"),
            |r, i| unsafe { r.read_i64(i) },
        );
        assert_eq!(struct_null, Some(5), "{name}: the struct itself");

        let rendered = fx.scalar(
            &format!("SELECT string_agg({name}(x)::VARCHAR, ',' ORDER BY x) FROM {HALF_NULL}"),
            |r, i| unsafe { r.read_str(i).to_owned() },
        );
        assert_eq!(
            rendered.as_deref(),
            Some("{'a': 42},{'a': 42},{'a': 42},{'a': 42},{'a': 42}"),
            "{name}: valid rows keep their value"
        );
    }
}

/// Direct checks of which child masks `set_null` clears, against the rule in
/// `DuckDB`'s `FlatVector::SetNull`: STRUCT children at the same row, ARRAY
/// children at `row * size .. row * size + size`, recursively; LIST children
/// never.
#[cfg(feature = "duckdb-1-5")]
mod nested_masks {
    use super::Fixture;
    use libduckdb_sys::{
        duckdb_array_vector_get_child, duckdb_list_vector_get_child,
        duckdb_struct_vector_get_child, duckdb_validity_row_is_valid, duckdb_vector,
        duckdb_vector_get_validity,
    };
    use quack_rs::types::{LogicalType, TypeId};
    use quack_rs::vector::ops::OwnedVector;
    use quack_rs::vector::VectorWriter;

    /// The rows of `vector` among `0..rows` that are NULL.
    fn nulls(vector: duckdb_vector, rows: u64) -> Vec<u64> {
        // SAFETY: `vector` is a live vector with at least `rows` rows; a NULL
        // mask reads as all-valid.
        let mask = unsafe { duckdb_vector_get_validity(vector) };
        (0..rows)
            .filter(|&r| !unsafe { duckdb_validity_row_is_valid(mask, r) })
            .collect()
    }

    #[test]
    fn set_null_clears_struct_fields_and_array_elements_recursively() {
        let _fx = Fixture::open();
        let ty = LogicalType::struct_type_from_logical(&[
            ("a", LogicalType::new(TypeId::BigInt)),
            ("arr", LogicalType::array(TypeId::BigInt, 3)),
            ("inner", LogicalType::struct_type(&[("b", TypeId::Integer)])),
            ("l", LogicalType::list(TypeId::BigInt)),
        ]);
        let vec = OwnedVector::new(&ty, 8).expect("allocate");
        let raw = vec.as_raw();
        // SAFETY: `raw` is a flat STRUCT vector with 8 rows.
        let mut writer = unsafe { VectorWriter::from_vector(raw) };
        // SAFETY: rows 2 and 5..7 are within the 8-row capacity.
        unsafe {
            writer.set_null(2);
            writer.set_null_range(5..7);
        }

        // SAFETY: field indices follow the declared type.
        let (a, arr, inner, l) = unsafe {
            (
                duckdb_struct_vector_get_child(raw, 0),
                duckdb_struct_vector_get_child(raw, 1),
                duckdb_struct_vector_get_child(raw, 2),
                duckdb_struct_vector_get_child(raw, 3),
            )
        };
        // SAFETY: `arr` is an ARRAY vector, `inner` a STRUCT, `l` a LIST.
        let (arr_child, b, l_child) = unsafe {
            (
                duckdb_array_vector_get_child(arr),
                duckdb_struct_vector_get_child(inner, 0),
                duckdb_list_vector_get_child(l),
            )
        };

        assert_eq!(nulls(raw, 8), [2, 5, 6]);
        assert_eq!(nulls(a, 8), [2, 5, 6]);
        assert_eq!(nulls(arr, 8), [2, 5, 6]);
        assert_eq!(nulls(arr_child, 24), [6, 7, 8, 15, 16, 17, 18, 19, 20]);
        assert_eq!(nulls(inner, 8), [2, 5, 6]);
        assert_eq!(nulls(b, 8), [2, 5, 6]);
        // The LIST itself is a field and is nulled; its elements are not.
        assert_eq!(nulls(l, 8), [2, 5, 6]);
        assert_eq!(nulls(l_child, 8), Vec::<u64>::new());
    }

    #[test]
    fn set_null_on_an_array_of_structs_reaches_the_struct_fields() {
        let _fx = Fixture::open();
        let element = LogicalType::struct_type(&[("x", TypeId::BigInt)]);
        let ty = LogicalType::array_from_logical(&element, 2);
        let vec = OwnedVector::new(&ty, 4).expect("allocate");
        let raw = vec.as_raw();
        // SAFETY: `raw` is a flat ARRAY vector with 4 rows.
        let mut writer = unsafe { VectorWriter::from_vector(raw) };
        // SAFETY: row 1 is within capacity.
        unsafe { writer.set_null(1) };

        // SAFETY: `raw` is an ARRAY of STRUCT(x).
        let (structs, x) = unsafe {
            let structs = duckdb_array_vector_get_child(raw);
            (structs, duckdb_struct_vector_get_child(structs, 0))
        };
        assert_eq!(nulls(raw, 4), [1]);
        assert_eq!(nulls(structs, 8), [2, 3]);
        assert_eq!(nulls(x, 8), [2, 3]);
    }

    #[test]
    fn set_null_on_a_flat_vector_touches_only_that_vector() {
        let _fx = Fixture::open();
        let vec = OwnedVector::new(&LogicalType::new(TypeId::BigInt), 4).expect("allocate");
        // SAFETY: a flat BIGINT vector with 4 rows.
        let mut writer = unsafe { VectorWriter::from_vector(vec.as_raw()) };
        // SAFETY: rows in range.
        unsafe {
            writer.set_null(0);
            writer.set_null(3);
            writer.set_valid(0);
        }
        assert_eq!(nulls(vec.as_raw(), 4), [3]);
    }
}

/// `SelectionVector::new` must never hand out uninitialised indices, and must
/// refuse lengths on which `DuckDB` would miscompute or throw.
#[cfg(feature = "duckdb-1-5")]
mod selection {
    use super::Fixture;
    use quack_rs::selection_vector::{SelectionVector, MAX_LEN};

    /// Before the fix, the reused allocation read back as `0xDEADBEEF`.
    #[test]
    fn a_new_selection_vector_is_zeroed_even_on_reused_memory() {
        let _fx = Fixture::open();
        for _ in 0..50 {
            let mut dirty = SelectionVector::new(1024).expect("allocate");
            dirty.as_mut_slice().fill(0xDEAD_BEEF);
            drop(dirty);
            let fresh = SelectionVector::new(1024).expect("allocate");
            assert!(
                fresh.as_slice().iter().all(|&i| i == 0),
                "stale indices: {:x?}",
                &fresh.as_slice()[..4]
            );
        }
    }

    /// `(1 << 62) + 4` indices is 16 bytes after `DuckDB`'s unchecked
    /// `count * 4`; `1 << 47` indices is past its allocator's limit and throws.
    /// Both are refused before `DuckDB` is called.
    #[test]
    fn oversized_selection_vectors_are_refused() {
        let _fx = Fixture::open();
        assert!(SelectionVector::new(MAX_LEN + 1).is_err());
        #[cfg(target_pointer_width = "64")]
        {
            assert!(SelectionVector::new((1 << 62) + 4).is_err());
            assert!(SelectionVector::new(1 << 47).is_err());
        }
        // A realistic size still works.
        assert_eq!(SelectionVector::new(2048).expect("allocate").len(), 2048);
    }
}
