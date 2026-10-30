// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ChunkWriter` against a chunk `DuckDB` allocated: its capacity is the
//! engine's vector size, its column count the chunk's, and dropping it sets
//! the chunk's size to the rows claimed and not given back.

use std::sync::atomic::{AtomicUsize, Ordering};

use libduckdb_sys::{
    duckdb_create_data_chunk, duckdb_create_logical_type, duckdb_data_chunk_get_size,
    duckdb_destroy_data_chunk, duckdb_destroy_logical_type, duckdb_vector_size,
    DUCKDB_TYPE_DUCKDB_TYPE_BIGINT, DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR,
};
use quack_rs::chunk_writer::ChunkWriter;
use quack_rs::data_chunk::DataChunk;
use quack_rs::table::{BindInfo, FfiBindData, FfiInitData, TableFunctionBuilder};
use quack_rs::types::TypeId;
use quack_rs::value::Value;

use super::Fixture;

// ─── A claimed row the source cannot fill (UNSOUND-1) ────────────────────────

/// The value the first scan call fills its whole chunk with.
const FIRST_CALL: i64 = 7;
/// What the second scan call has to write: fewer rows than a chunk holds.
const SOURCE: [i64; 2] = [1, 2];

quack_rs::table_bind_callback!(claim_bind, |info| {
    // SAFETY: `info` is the live bind info; parameter 0 is declared.
    let bind = unsafe { BindInfo::new(info) };
    let release = unsafe { Value::from_raw(bind.get_parameter(0)) }
        .as_bool()
        .expect("a BOOLEAN argument");
    bind.add_result_column("v", TypeId::BigInt);
    // SAFETY: bind data is set once, here.
    unsafe { FfiBindData::<bool>::set(info, release) };
});

quack_rs::table_init_callback!(claim_init, |info| {
    // SAFETY: global init; the call counter is this scan's only state.
    unsafe { FfiInitData::<AtomicUsize>::set(info, AtomicUsize::new(0)) };
});

// Call 0 fills the chunk with `FIRST_CALL`. Call 1 runs the natural "claim a
// row, then ask the source" loop over `SOURCE`, which claims one row more than
// it writes, and gives that row back when the bind argument says so. Call 2
// claims nothing, ending the scan.
quack_rs::table_scan_callback!(claim_scan, |info, output| {
    // SAFETY: bind and init data were set above; `output` is the live chunk.
    let (release, calls, chunk) = unsafe {
        (
            *FfiBindData::<bool>::get_from_function(info).expect("bind data"),
            FfiInitData::<AtomicUsize>::get(info).expect("init data"),
            DataChunk::from_raw(output),
        )
    };
    // SAFETY: `output` is this scan's writable chunk, of one BIGINT column.
    let mut cw = unsafe { chunk.into_chunk_writer() };
    let mut w = unsafe { cw.writer(0) };
    match calls.fetch_add(1, Ordering::SeqCst) {
        0 => {
            while let Some(row) = cw.next_row() {
                unsafe { w.write_i64(row, FIRST_CALL) };
            }
        }
        1 => {
            let mut source = SOURCE.into_iter();
            while let Some(row) = cw.next_row() {
                let Some(v) = source.next() else {
                    if release {
                        cw.release_row(row);
                    }
                    break;
                };
                unsafe { w.write_i64(row, v) };
            }
        }
        _ => {}
    }
});

/// The reviewer's probe for UNSOUND-1: the second call's loop claims a third
/// row and its source has nothing for it. Not given back, that row is
/// published with what the reused output chunk held from the first call, a
/// `7` the scan never wrote for it. Given back with `release_row`, the call
/// publishes exactly the two rows it wrote.
#[test]
fn a_claimed_row_the_source_cannot_fill_is_given_back_not_published_stale() {
    let fx = Fixture::open();
    // SAFETY: `fx` is open; the callbacks match the declared signature.
    unsafe {
        TableFunctionBuilder::new("cw_claim")
            .param(TypeId::Boolean)
            .bind(claim_bind)
            .init(claim_init)
            .scan(claim_scan)
            .register(fx.con())
    }
    .expect("register cw_claim");
    // SAFETY: the dispatch table is initialised.
    let capacity = usize::try_from(unsafe { duckdb_vector_size() }).expect("fits");
    let values = |release: bool| -> Vec<i64> {
        let mut result = fx.query(&format!("SELECT v FROM cw_claim({release})"));
        let mut out = Vec::new();
        while let Some(chunk) = result.next_chunk().expect("fetch") {
            // SAFETY: one BIGINT column, read within the chunk's size.
            let reader = unsafe { chunk.reader(0) };
            out.extend((0..chunk.size()).map(|row| unsafe { reader.read_i64(row) }));
        }
        out
    };

    let released = values(true);
    assert_eq!(
        released.len(),
        capacity + SOURCE.len(),
        "{:?}",
        &released[capacity..]
    );
    assert!(released[..capacity].iter().all(|&v| v == FIRST_CALL));
    assert_eq!(released[capacity..], SOURCE);

    // The hazard `release_row` exists for, as the module docs describe it.
    let stale = values(false);
    assert_eq!(stale.len(), capacity + SOURCE.len() + 1);
    assert_eq!(stale[capacity..], [SOURCE[0], SOURCE[1], FIRST_CALL]);
}

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
