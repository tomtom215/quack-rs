// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ChunkWriter` against a chunk `DuckDB` allocated: its capacity is the
//! engine's vector size, its column count the chunk's, and dropping it sets
//! the chunk's size to the rows it handed out.

use libduckdb_sys::{
    duckdb_create_data_chunk, duckdb_create_logical_type, duckdb_data_chunk_get_size,
    duckdb_destroy_data_chunk, duckdb_destroy_logical_type, duckdb_vector_size,
    DUCKDB_TYPE_DUCKDB_TYPE_BIGINT, DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR,
};
use quack_rs::chunk_writer::ChunkWriter;

use super::Fixture;

#[test]
fn a_chunk_writer_reports_the_chunk_and_sets_its_size_on_drop() {
    let _fx = Fixture::open();
    // SAFETY: the dispatch table is initialised; each handle is destroyed
    // below and used only while live.
    unsafe {
        let mut types = [
            duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_BIGINT),
            duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR),
            duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_BIGINT),
        ];
        let mut chunk = duckdb_create_data_chunk(types.as_mut_ptr(), 3);
        for t in &mut types {
            duckdb_destroy_logical_type(t);
        }

        let mut writer = ChunkWriter::new(chunk);
        assert_eq!(writer.capacity() as u64, duckdb_vector_size());
        assert_eq!(writer.column_count(), 3);
        for expected in 0..3 {
            assert_eq!(writer.next_row(), Some(expected));
        }
        drop(writer);
        assert_eq!(duckdb_data_chunk_get_size(chunk), 3, "set on drop");

        let writer = ChunkWriter::with_capacity(chunk, 2);
        assert_eq!(writer.capacity(), 2);
        writer.finish_with_size(1);
        assert_eq!(duckdb_data_chunk_get_size(chunk), 1);

        duckdb_destroy_data_chunk(&raw mut chunk);
    }
}
