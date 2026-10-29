// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `DataChunk::any_null`, `DataChunk::struct_writer`,
//! `DataChunk::into_chunk_writer`, and `ChunkWriter`'s `is_full` and
//! `struct_writer`, on the chunks `DuckDB` hands a scalar and a table function.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::table::{BindInfo, TableFunctionBuilder};
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::VectorWriter;

use super::Fixture;

quack_rs::scalar_callback!(report_any_null, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        let any = unsafe { chunk.any_null(row) };
        unsafe { writer.write_i64(row, i64::from(any)) };
    }
});

/// A NULL in any argument position, and in no other case, is reported.
#[test]
fn any_null_is_true_exactly_when_some_column_is_null_at_the_row() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the signature.
    unsafe {
        ScalarFunctionBuilder::try_new("dch_any_null")
            .expect("name")
            .param(TypeId::BigInt)
            .param(TypeId::BigInt)
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(report_any_null)
            .register(fx.con())
    }
    .expect("register dch_any_null");
    let mut result = fx.query(
        "SELECT dch_any_null(a, b, c) FROM (VALUES \
           (1, 2, 3), (NULL, 2, 3), (1, NULL, 3), (1, 2, NULL), \
           (NULL, NULL, NULL), (4, 5, 6), (0, 0, 0)) t(a, b, c)",
    );
    let chunk = result.next_chunk().expect("fetch").expect("rows");
    // SAFETY: one BIGINT column.
    let got: Vec<i64> = (0..chunk.size())
        .map(|row| unsafe { chunk.reader(0).read_i64(row) })
        .collect();
    assert_eq!(got, [0, 1, 1, 1, 1, 0, 0]);
}

// ─── STRUCT output through the chunk and through a ChunkWriter ───────────────

/// How a scan writes its rows.
#[derive(Clone, Copy)]
enum Via {
    /// `DataChunk::struct_writer`, then `set_size`.
    Chunk,
    /// `DataChunk::into_chunk_writer`, then `ChunkWriter::struct_writer`.
    Writer,
}

static CAPACITY_SEEN: AtomicUsize = AtomicUsize::new(0);
/// The most rows one scan call handed out through a `ChunkWriter`.
static MOST_ROWS_IN_A_CALL: AtomicUsize = AtomicUsize::new(0);
static SAW_FULL: AtomicBool = AtomicBool::new(false);

fn struct_type() -> LogicalType {
    LogicalType::struct_type(&[("a", TypeId::BigInt), ("b", TypeId::Varchar)])
}

fn scan(
    via: Via,
) -> impl Fn(&mut i64, &DataChunk) -> Result<(), quack_rs::error::ExtensionError> + Send + Sync + 'static
{
    move |remaining, chunk| {
        let start = *remaining;
        // SAFETY: column 0 is the BIGINT `id` and column 1 the STRUCT(a BIGINT,
        // b VARCHAR) `s` declared in bind, and the chunk is the scan's writable
        // output chunk.
        let written = unsafe {
            match via {
                Via::Chunk => {
                    let mut ids = chunk.writer(0);
                    let mut out = chunk.struct_writer(1, 2);
                    let count = start.min(quack_rs::vector::vector_size() as i64);
                    for row in 0..count {
                        ids.write_i64(row as usize, start - row);
                        out.write_i64(row as usize, 0, start - row);
                        out.write_varchar(row as usize, 1, &format!("row{}", start - row));
                    }
                    chunk.set_size(count as usize);
                    count
                }
                Via::Writer => {
                    // `DataChunk` is a view; `into_chunk_writer` takes the
                    // output chunk by value, and sets the size when dropped.
                    let mut rows = DataChunk::from_raw(chunk.as_raw()).into_chunk_writer();
                    let mut ids = rows.writer(0);
                    let mut out = rows.struct_writer(1, 2);
                    let mut count = 0;
                    while count < start {
                        let Some(row) = rows.next_row() else { break };
                        ids.write_i64(row, start - count);
                        out.write_i64(row, 0, start - count);
                        out.write_varchar(row, 1, &format!("row{}", start - count));
                        count += 1;
                    }
                    if rows.is_full() {
                        SAW_FULL.store(true, Ordering::SeqCst);
                    }
                    CAPACITY_SEEN.store(rows.capacity(), Ordering::SeqCst);
                    MOST_ROWS_IN_A_CALL.fetch_max(rows.row_count(), Ordering::SeqCst);
                    count
                }
            }
        };
        *remaining -= written;
        Ok(())
    }
}

fn register(fx: &Fixture, name: &str, via: Via) {
    let builder = TableFunctionBuilder::new(name)
        .param(TypeId::BigInt)
        .with_state::<i64, _>(|bind: &BindInfo| {
            bind.add_result_column("id", TypeId::BigInt);
            bind.add_result_column_with_type("s", &struct_type());
            // SAFETY: parameter 0 is declared.
            Ok(unsafe { bind.get_parameter_value(0) }.as_i64_or(0))
        })
        .scan(scan(via))
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.unwrap_or_else(|e| panic!("register {name}: {e}"));
}

/// 5000 rows span several chunks, so the full-chunk case and the short last
/// chunk both occur; every row's fields land in the right struct member.
#[test]
fn struct_columns_written_through_the_chunk_and_through_a_chunk_writer_agree() {
    let fx = Fixture::open();
    register(&fx, "dch_via_chunk", Via::Chunk);
    register(&fx, "dch_via_writer", Via::Writer);
    let total: i64 = 5000;
    for name in ["dch_via_chunk", "dch_via_writer"] {
        let sql = format!(
            "SELECT count(*)::BIGINT, sum(s.a)::BIGINT, min(s.a)::BIGINT, max(s.a)::BIGINT, \
                    sum((s.b = 'row' || s.a::VARCHAR)::INTEGER)::BIGINT, \
                    sum((id = s.a)::INTEGER)::BIGINT FROM {name}({total})"
        );
        let mut result = fx.query(&sql);
        let chunk = result.next_chunk().expect("fetch").expect("one row");
        // SAFETY: six BIGINT columns, one row.
        let got: Vec<i64> = (0..6)
            .map(|c| unsafe { chunk.reader(c).read_i64(0) })
            .collect();
        assert_eq!(
            got,
            [total, total * (total + 1) / 2, 1, total, total, total],
            "{name}: count, sum, min, max, rows whose b matches their a, rows whose id does"
        );
    }
    assert!(
        SAW_FULL.load(Ordering::SeqCst),
        "a chunk writer reports full once it has handed out a whole chunk"
    );
    assert_eq!(
        CAPACITY_SEEN.load(Ordering::SeqCst) as u64,
        quack_rs::vector::vector_size() as u64
    );
    // The writer stops handing out rows at the chunk's capacity: a call with
    // 5000 rows left still writes one chunk's worth, not 5000 into a vector
    // sized for 2048.
    assert_eq!(
        MOST_ROWS_IN_A_CALL.load(Ordering::SeqCst) as u64,
        quack_rs::vector::vector_size() as u64
    );
}
