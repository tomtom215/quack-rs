// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Helpers shared by the vector-module tests (`struct_io`, `validity_api`,
//! `map_api`, `list_api`, `vector_ops`, `writer_api`, `chunk_writer_cols`).

use super::Fixture;

/// Row counts on both sides of every edge that matters for a vector:
/// one row, two rows, one short of a full vector, a full vector, and one past
/// it (which makes the engine split the work into two chunks). They follow
/// the engine's own vector size, so a build with a different
/// `STANDARD_VECTOR_SIZE` still lands on the edges.
pub fn edge_rows() -> Vec<usize> {
    let size = usize::try_from(quack_rs::vector::vector_size()).expect("vector size fits usize");
    vec![1, 2, size - 1, size, size + 1]
}

/// Every row of `sql`, each column read as text: the query must cast its
/// columns to `VARCHAR`. `None` is SQL NULL.
pub fn text_rows(fx: &Fixture, sql: &str) -> Vec<Vec<Option<String>>> {
    let mut result = fx.query(sql);
    let mut rows = Vec::new();
    while let Some(chunk) = result.next_chunk().unwrap_or_else(|e| panic!("{sql}: {e}")) {
        // SAFETY: each column index is below the chunk's column count.
        let readers: Vec<_> = (0..chunk.column_count())
            .map(|c| unsafe { chunk.reader(c) })
            .collect();
        for row in 0..chunk.size() {
            rows.push(
                readers
                    .iter()
                    // SAFETY: `row` is below the chunk's size, and the column is VARCHAR.
                    .map(|r| unsafe { r.is_valid(row).then(|| r.read_str(row).to_owned()) })
                    .collect(),
            );
        }
    }
    rows
}

/// The single `BIGINT` that `sql` returns; panics on NULL.
pub fn count(fx: &Fixture, sql: &str) -> i64 {
    fx.scalar(sql, |r, i| {
        // SAFETY: the query returns one BIGINT column.
        unsafe { r.read_i64(i) }
    })
    .unwrap_or_else(|| panic!("{sql} returned NULL"))
}
