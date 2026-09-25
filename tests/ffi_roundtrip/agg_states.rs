// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Every aggregate state `FfiState` creates is dropped, including the ones
//! `DuckDB` moves.
//!
//! A parallel hash aggregate repartitions its groups, copying each state's
//! bytes to a new row (`radix_partitioned_hashtable.cpp`) and destroying it
//! there. The fourth audit's `FfiState` tag was derived from the slot's
//! address, so every moved state failed the tag check, was skipped, and its
//! `T` leaked: on eight threads, tens of thousands of states per query of this
//! size, and millions on larger ones.

use std::sync::atomic::{AtomicI64, Ordering};

use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};
use quack_rs::data_chunk::DataChunk;
use quack_rs::query::OwnedConnection;
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::Fixture;

/// Live `CountedSum` values.
static LIVE: AtomicI64 = AtomicI64::new(0);

struct CountedSum {
    total: i64,
}
impl Default for CountedSum {
    fn default() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Self { total: 0 }
    }
}
impl Drop for CountedSum {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
impl AggregateState for CountedSum {}

quack_rs::aggregate_update_callback!(counted_update, |_info, input, states| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if let Some(s) = unsafe { FfiState::<CountedSum>::with_state_mut(*states.add(row)) } {
            s.total += unsafe { reader.read_i64(row) };
        }
    }
});

quack_rs::aggregate_combine_callback!(counted_combine, |_info, source, target, count| {
    for i in 0..count as usize {
        let v =
            unsafe { FfiState::<CountedSum>::with_state(*source.add(i)) }.map_or(0, |s| s.total);
        if let Some(t) = unsafe { FfiState::<CountedSum>::with_state_mut(*target.add(i)) } {
            t.total += v;
        }
    }
});

quack_rs::aggregate_finalize_callback!(counted_finalize, |_info, source, result, count, offset| {
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        let row = offset as usize + i;
        match unsafe { FfiState::<CountedSum>::with_state(*source.add(i)) } {
            Some(s) => unsafe { writer.write_i64(row, s.total) },
            None => unsafe { writer.set_null(row) },
        }
    }
});

#[test]
fn every_state_of_a_parallel_grouped_aggregate_is_dropped() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the builder's signatures.
    unsafe {
        AggregateFunctionBuilder::try_new("counted_sum")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            // All three state callbacks for one type; the live-state count
            // below also checks that the destructor is among them.
            .ffi_state::<CountedSum>()
            .update(counted_update)
            .combine(counted_combine)
            .finalize(counted_finalize)
            .register(fx.con())
            .expect("register counted_sum");
    }
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    con.execute("SET threads = 8").expect("threads");
    let rows: i64 = 2_000_000;
    con.execute(&format!(
        "CREATE TABLE numbers AS SELECT i::BIGINT AS i FROM range({rows}) t(i)"
    ))
    .expect("create");
    let expected = i128::from((rows - 1) * rows / 2);
    for sql in [
        "SELECT counted_sum(i)::HUGEINT FROM numbers",
        "SELECT sum(s)::HUGEINT FROM (SELECT i % 10007 AS g, counted_sum(i) AS s FROM numbers GROUP BY g)",
        "SELECT sum(s)::HUGEINT FROM (SELECT i % 400000 AS g, counted_sum(i) AS s FROM numbers GROUP BY g)",
    ] {
        let mut result = con.query(sql).expect(sql);
        let chunk = result.next_chunk().expect("fetch").expect("one row");
        // SAFETY: one HUGEINT column, one row.
        assert_eq!(unsafe { chunk.reader(0).read_i128(0) }, expected, "{sql}");
        drop(chunk);
        drop(result);
        // The next statement finishes the previous one's cleanup.
        con.execute("SELECT 1").expect("next statement");
        assert_eq!(LIVE.load(Ordering::SeqCst), 0, "states still live after {sql}");
    }
}

/// Live `WindowSum` values (a counter of its own: tests run in parallel).
static WINDOW_LIVE: AtomicI64 = AtomicI64::new(0);

struct WindowSum {
    total: i64,
}
impl Default for WindowSum {
    fn default() -> Self {
        WINDOW_LIVE.fetch_add(1, Ordering::SeqCst);
        Self { total: 0 }
    }
}
impl Drop for WindowSum {
    fn drop(&mut self) {
        WINDOW_LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
impl AggregateState for WindowSum {}

quack_rs::aggregate_update_callback!(window_update, |_info, input, states| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if let Some(s) = unsafe { FfiState::<WindowSum>::with_state_mut(*states.add(row)) } {
            s.total += unsafe { reader.read_i64(row) };
        }
    }
});

quack_rs::aggregate_combine_callback!(window_combine, |_info, source, target, count| {
    for i in 0..count as usize {
        let v = unsafe { FfiState::<WindowSum>::with_state(*source.add(i)) }.map_or(0, |s| s.total);
        if let Some(t) = unsafe { FfiState::<WindowSum>::with_state_mut(*target.add(i)) } {
            t.total += v;
        }
    }
});

quack_rs::aggregate_finalize_callback!(window_finalize, |_info, source, result, count, offset| {
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        let row = offset as usize + i;
        match unsafe { FfiState::<WindowSum>::with_state(*source.add(i)) } {
            Some(s) => unsafe { writer.write_i64(row, s.total) },
            None => unsafe { writer.set_null(row) },
        }
    }
});

/// A window frame with `EXCLUDE` is evaluated by `DuckDB`'s segment tree in
/// two parts, and the second part's states are initialised for every chunk
/// but never destroyed (`WindowSegmentTreeLocalState::Evaluate`,
/// `window_segment_tree.cpp`; upstream item 35). A small `T` lives in
/// `DuckDB`'s state bytes, so the result is right and only what `T` owns on
/// the heap leaks; this counts the `T`s never dropped. Without `EXCLUDE`
/// every state is dropped.
#[test]
fn a_window_frame_with_exclude_leaves_states_undestroyed() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the builder's signatures.
    unsafe {
        AggregateFunctionBuilder::try_new("window_sum")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .ffi_state::<WindowSum>()
            .update(window_update)
            .combine(window_combine)
            .finalize(window_finalize)
            .register(fx.con())
            .expect("register window_sum");
    }
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let leaked = |frame: &str| {
        let sql = format!(
            "SELECT count(*) FILTER (WHERE a IS DISTINCT FROM b) FROM (\
               SELECT window_sum(i) OVER w AS a, coalesce(sum(i) OVER w, 0) AS b \
               FROM range(5000) t(i) \
               WINDOW w AS (ORDER BY i ROWS BETWEEN 2 PRECEDING AND CURRENT ROW {frame}))"
        );
        let mut result = con.query(&sql).expect(&sql);
        let chunk = result.next_chunk().expect("fetch").expect("one row");
        // SAFETY: one BIGINT column, one row.
        assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 0, "{sql}");
        drop(chunk);
        drop(result);
        con.execute("SELECT 1").expect("next statement");
        let live = WINDOW_LIVE.swap(0, Ordering::SeqCst);
        println!("window_sum {frame:?}: {live} states never dropped");
        live
    };
    assert_eq!(leaked(""), 0);
    assert!(leaked("EXCLUDE CURRENT ROW") > 0);
}
