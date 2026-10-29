// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The raw table-function plumbing a hand-written `bind` / `init` / `scan`
//! uses: `BindInfo`'s parameter accessors and cardinality hint, and the global
//! and per-thread init data a scan reads and mutates.
//!
//! The typed builder reaches none of these (it wraps them), so without a raw
//! function they run nowhere.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use quack_rs::data_chunk::DataChunk;
use quack_rs::table::{
    BindInfo, FfiBindData, FfiInitData, FfiLocalInitData, InitInfo, TableFunctionBuilder,
};
use quack_rs::types::TypeId;
use quack_rs::value::Value;

use super::Fixture;

/// Every row of `sql` as `i64` columns.
fn rows(fx: &Fixture, sql: &str, columns: usize) -> Vec<Vec<i64>> {
    let mut result = fx.query(sql);
    let mut out = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("fetch") {
        for row in 0..chunk.size() {
            out.push(
                (0..columns)
                    .map(|c| {
                        // SAFETY: the query returned `columns` BIGINT columns.
                        let reader = unsafe { chunk.reader(c) };
                        // SAFETY: `row` is in range and the column is BIGINT.
                        unsafe { reader.read_i64(row) }
                    })
                    .collect(),
            );
        }
    }
    out
}

// ─── Parameters: positional, named, absent ───────────────────────────────────

/// What one bind callback saw, read through the raw accessors.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Seen {
    parameter_count: usize,
    rows: Option<i64>,
    label: Option<String>,
    step: Option<i64>,
    tag: Option<String>,
    flag: Option<bool>,
    /// `get_named_parameter("absent")` returned a null handle.
    absent_is_null: bool,
}

/// One entry per query, keyed by the `label` argument.
static BINDS: Mutex<Option<HashMap<String, Seen>>> = Mutex::new(None);

/// A named parameter as an owned `Value`, or `None` when the call omitted it.
fn named(bind: &BindInfo, name: &str) -> Option<Value> {
    // SAFETY: `name` is declared on `tia_params`; the handle is destroyed by
    // `Value`'s drop.
    let raw = unsafe { bind.get_named_parameter(name) };
    // SAFETY: DuckDB returned a fresh owned value, or null for an omitted
    // parameter.
    (!raw.is_null()).then(|| unsafe { Value::from_raw(raw) })
}

quack_rs::table_bind_callback!(params_bind, |info| {
    // SAFETY: `info` is the live bind info.
    let bind = unsafe { BindInfo::new(info) };
    // SAFETY: both positional parameters are declared, so 0 and 1 are in
    // range; each handle is wrapped at once so it is destroyed.
    let (rows, label) = unsafe {
        (
            Value::from_raw(bind.get_parameter(0)),
            Value::from_raw(bind.get_parameter(1)),
        )
    };
    let label = label.as_str().expect("a VARCHAR label");
    let seen = Seen {
        parameter_count: bind.parameter_count(),
        rows: rows.as_i64(),
        label: Some(label.clone()),
        step: named(&bind, "step").and_then(|v| v.as_i64()),
        tag: named(&bind, "tag").and_then(|v| v.as_str().ok()),
        flag: named(&bind, "flag").and_then(|v| v.as_bool()),
        // SAFETY: "absent" is a declared named parameter.
        absent_is_null: unsafe { bind.get_named_parameter("absent") }.is_null(),
    };
    BINDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(label, seen);
    bind.add_result_column("n", TypeId::BigInt);
    // SAFETY: bind data is set once, here.
    unsafe { FfiBindData::<i64>::set(info, rows.as_i64().unwrap_or(0)) };
});

quack_rs::table_init_callback!(params_init, |info| {
    // SAFETY: global init; the next-row counter is this scan's only state.
    unsafe { FfiInitData::<AtomicI64>::set(info, AtomicI64::new(0)) };
});

quack_rs::table_scan_callback!(params_scan, |info, output| {
    // SAFETY: `output` is the live chunk; bind and init data were set above.
    let (chunk, total, next) = unsafe {
        (
            DataChunk::from_raw(output),
            FfiBindData::<i64>::get_from_function(info).copied(),
            FfiInitData::<AtomicI64>::get(info),
        )
    };
    let total = total.expect("bind data");
    let next = next.expect("init data");
    let start = next.load(Ordering::SeqCst);
    let count = (total - start).clamp(0, 4);
    next.store(start + count, Ordering::SeqCst);
    for i in 0..count {
        // SAFETY: column 0 is BIGINT and `i` is below the chunk capacity.
        unsafe { chunk.writer(0).write_i64(i as usize, start + i) };
    }
    // SAFETY: `count` rows were written.
    unsafe { chunk.set_size(count as usize) };
});

fn register_params(fx: &Fixture) {
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        TableFunctionBuilder::new("tia_params")
            .param(TypeId::BigInt)
            .param(TypeId::Varchar)
            .named_param("step", TypeId::BigInt)
            .named_param("tag", TypeId::Varchar)
            .named_param("flag", TypeId::Boolean)
            .named_param("absent", TypeId::Double)
            .bind(params_bind)
            .init(params_init)
            .scan(params_scan)
            .register(fx.con())
    }
    .expect("register tia_params");
}

fn seen(label: &str) -> Seen {
    BINDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(label))
        .cloned()
        .unwrap_or_else(|| panic!("no bind recorded for {label}"))
}

#[test]
fn a_bind_callback_reads_each_parameter_by_position_and_by_name() {
    let fx = Fixture::open();
    register_params(&fx);

    assert_eq!(
        rows(
            &fx,
            "SELECT n FROM tia_params(7, 'all', step := 3, tag := 'x', flag := true)",
            1
        ),
        (0..7).map(|n| vec![n]).collect::<Vec<_>>(),
        "the scan sees the positional rows argument"
    );
    assert_eq!(
        seen("all"),
        Seen {
            parameter_count: 2,
            rows: Some(7),
            label: Some("all".into()),
            step: Some(3),
            tag: Some("x".into()),
            flag: Some(true),
            absent_is_null: true,
        },
        "named parameters are not counted, and an omitted one is a null handle"
    );

    // Named parameters in another order, with other values: nothing is
    // positional about them, and nothing is cached from the call before.
    rows(
        &fx,
        "SELECT n FROM tia_params(3, 'some', flag := false, step := -9)",
        1,
    );
    assert_eq!(
        seen("some"),
        Seen {
            parameter_count: 2,
            rows: Some(3),
            label: Some("some".into()),
            step: Some(-9),
            tag: None,
            flag: Some(false),
            absent_is_null: true,
        }
    );

    // No named parameter at all.
    rows(&fx, "SELECT n FROM tia_params(0, 'none')", 1);
    assert_eq!(
        seen("none"),
        Seen {
            parameter_count: 2,
            rows: Some(0),
            label: Some("none".into()),
            step: None,
            tag: None,
            flag: None,
            absent_is_null: true,
        }
    );

    // Positional values come from the arguments, not from a fixed slot.
    rows(&fx, "SELECT n FROM tia_params(1 + 1, 'ab' || 'c')", 1);
    assert_eq!(seen("abc").rows, Some(2));
}

/// `parameter_count` is the number of positional parameters of the call, for
/// functions declared with none, one and four.
#[test]
fn parameter_count_is_the_number_of_positional_parameters() {
    static COUNTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

    quack_rs::table_bind_callback!(count_bind, |info| {
        // SAFETY: `info` is the live bind info.
        let bind = unsafe { BindInfo::new(info) };
        COUNTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(bind.parameter_count());
        bind.add_result_column("n", TypeId::BigInt);
    });
    quack_rs::table_init_callback!(count_init, |info| {
        let _ = info;
    });
    quack_rs::table_scan_callback!(count_scan, |info, output| {
        let _ = info;
        // SAFETY: `output` is the live chunk; zero rows end the scan.
        unsafe { DataChunk::from_raw(output).set_size(0) };
    });

    let fx = Fixture::open();
    for (name, arity) in [("tia_arity0", 0), ("tia_arity1", 1), ("tia_arity4", 4)] {
        let mut builder = TableFunctionBuilder::new(name)
            .named_param("ignored", TypeId::BigInt)
            .bind(count_bind)
            .init(count_init)
            .scan(count_scan);
        for _ in 0..arity {
            builder = builder.param(TypeId::BigInt);
        }
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe { builder.register(fx.con()) }.unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
    COUNTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    fx.query("SELECT * FROM tia_arity0(ignored := 5)");
    fx.query("SELECT * FROM tia_arity1(1)");
    fx.query("SELECT * FROM tia_arity4(1, 2, 3, 4, ignored := 1)");
    assert_eq!(
        *COUNTS.lock().unwrap_or_else(PoisonError::into_inner),
        [0, 1, 4]
    );
}

// ─── The cardinality hint ────────────────────────────────────────────────────

// Declares `n` rows with the hint's exactness taken from the `exact` named
// parameter; a negative `n` sets no hint at all. The scan itself returns
// three rows whatever the hint says.
quack_rs::table_bind_callback!(card_bind, |info| {
    // SAFETY: `info` is the live bind info.
    let bind = unsafe { BindInfo::new(info) };
    // SAFETY: parameter 0 is declared.
    let n = unsafe { Value::from_raw(bind.get_parameter(0)) }
        .as_i64()
        .expect("a BIGINT row count");
    let exact = named(&bind, "exact")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if let Ok(rows) = u64::try_from(n) {
        bind.set_cardinality(rows, exact);
    }
    bind.add_result_column("n", TypeId::BigInt);
});

quack_rs::table_init_callback!(card_init, |info| {
    // SAFETY: global init; a "done" flag is the only state.
    unsafe { FfiInitData::<AtomicBool>::set(info, AtomicBool::new(false)) };
});

quack_rs::table_scan_callback!(card_scan, |info, output| {
    // SAFETY: `output` is the live chunk and init set the flag.
    let (chunk, done) = unsafe {
        (
            DataChunk::from_raw(output),
            FfiInitData::<AtomicBool>::get(info).expect("init data"),
        )
    };
    if done.swap(true, Ordering::SeqCst) {
        // SAFETY: end of stream.
        unsafe { chunk.set_size(0) };
        return;
    }
    for i in 0..3 {
        // SAFETY: column 0 is BIGINT and three rows fit.
        unsafe { chunk.writer(0).write_i64(i, i as i64) };
    }
    // SAFETY: three rows were written.
    unsafe { chunk.set_size(3) };
});

/// The plan `EXPLAIN` prints for `sql`, with thousands separators removed.
fn plan(fx: &Fixture, sql: &str) -> String {
    let mut result = fx.query(&format!("EXPLAIN {sql}"));
    let chunk = result
        .next_chunk()
        .expect("fetch")
        .expect("EXPLAIN returns a row");
    // SAFETY: EXPLAIN returns (explain_key, explain_value), both VARCHAR.
    let text = unsafe { chunk.reader(1).read_str(0).to_owned() };
    text.replace(',', "")
}

/// `set_cardinality` is the planner's only view of a table function's size:
/// without it every scan is estimated at one row.
#[test]
fn set_cardinality_reaches_the_planner_as_the_scans_estimated_rows() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        TableFunctionBuilder::new("tia_card")
            .param(TypeId::BigInt)
            .named_param("exact", TypeId::Boolean)
            .bind(card_bind)
            .init(card_init)
            .scan(card_scan)
            .register(fx.con())
    }
    .expect("register tia_card");

    // No hint: DuckDB's default estimate.
    assert!(
        plan(&fx, "SELECT * FROM tia_card(-1)").contains("~1 row"),
        "{}",
        plan(&fx, "SELECT * FROM tia_card(-1)")
    );
    // Both flags carry the row count itself: `is_exact` decides only whether
    // DuckDB also records an upper bound, and which flag does so differs by
    // major version (see `BindInfo::set_cardinality`). The estimate is the
    // same either way.
    for exact in ["true", "false"] {
        for (rows, shown) in [
            (18_u64, "~18 rows"),
            (1234, "~1234 rows"),
            // Wider than 32 bits: the count must not be truncated.
            (5_000_000_000, "~5000000000 rows"),
        ] {
            let sql = format!("SELECT * FROM tia_card({rows}, exact := {exact})");
            let plan = plan(&fx, &sql);
            assert!(plan.contains(shown), "{sql}: expected {shown} in\n{plan}");
        }
    }
    // The hint is only a hint: the scan's real output is unchanged.
    assert_eq!(
        rows(&fx, "SELECT n FROM tia_card(1000000)", 1),
        vec![vec![0], vec![1], vec![2]]
    );
}

// ─── Global and per-thread init data ─────────────────────────────────────────

/// Counts how many init-data values were created and destroyed, so a
/// destructor that was never registered is visible.
static GLOBAL_SET: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_DROPPED: AtomicUsize = AtomicUsize::new(0);
static LOCAL_SET: AtomicUsize = AtomicUsize::new(0);
static LOCAL_DROPPED: AtomicUsize = AtomicUsize::new(0);

struct Global {
    next: i64,
    total: i64,
    calls: i64,
}

impl Drop for Global {
    fn drop(&mut self) {
        GLOBAL_DROPPED.fetch_add(1, Ordering::SeqCst);
    }
}

struct Local {
    calls: i64,
}

impl Drop for Local {
    fn drop(&mut self) {
        LOCAL_DROPPED.fetch_add(1, Ordering::SeqCst);
    }
}

quack_rs::table_bind_callback!(state_bind, |info| {
    // SAFETY: `info` is the live bind info.
    let bind = unsafe { BindInfo::new(info) };
    for name in ["row", "call", "global_calls"] {
        bind.add_result_column(name, TypeId::BigInt);
    }
    // SAFETY: parameter 0 is declared; the bind data is set once.
    unsafe {
        let total = Value::from_raw(bind.get_parameter(0))
            .as_i64()
            .expect("a BIGINT total");
        FfiBindData::<i64>::set(info, total);
    }
});

quack_rs::table_init_callback!(state_init, |info| {
    // SAFETY: global init, so the bind data is readable and this sets the
    // global (not the local) init data.
    unsafe {
        let total = *FfiBindData::<i64>::get_from_init(info).expect("bind data");
        FfiInitData::<Global>::set(
            info,
            Global {
                next: 0,
                total,
                calls: 0,
            },
        );
    }
    GLOBAL_SET.fetch_add(1, Ordering::SeqCst);
});

quack_rs::table_init_callback!(state_local_init, |info| {
    // SAFETY: `local_init`, so this sets the local init data.
    unsafe { FfiLocalInitData::<Local>::set(info, Local { calls: 0 }) };
    LOCAL_SET.fetch_add(1, Ordering::SeqCst);
});

// Three rows per call. The counters are bumped through `get_mut` and read
// back through `get`: each accessor must see the other's writes, and the
// writes must survive into the next call.
quack_rs::table_scan_callback!(state_scan, |info, output| {
    // SAFETY: `output` is the live chunk; `max_threads` is 1 (the default), so
    // no other scan call holds a reference to the global data, and local data
    // belongs to this task alone. Every reference below is dropped before the
    // next accessor is called.
    let chunk = unsafe { DataChunk::from_raw(output) };
    let (start, count) = {
        let global = unsafe { FfiInitData::<Global>::get_mut(info) }.expect("global init data");
        global.calls += 1;
        let start = global.next;
        let count = (global.total - start).clamp(0, 3);
        global.next += count;
        (start, count)
    };
    {
        let local = unsafe { FfiLocalInitData::<Local>::get_mut(info) }.expect("local init data");
        local.calls += 1;
    }
    let call = unsafe { FfiLocalInitData::<Local>::get(info) }
        .expect("local init data")
        .calls;
    let global_calls = unsafe { FfiInitData::<Global>::get(info) }
        .expect("global init data")
        .calls;
    for i in 0..count {
        let row = i as usize;
        // SAFETY: three BIGINT columns and `row` is below the capacity.
        unsafe {
            chunk.writer(0).write_i64(row, start + i);
            chunk.writer(1).write_i64(row, call);
            chunk.writer(2).write_i64(row, global_calls);
        }
    }
    // SAFETY: `count` rows were written.
    unsafe { chunk.set_size(count as usize) };
});

#[test]
fn init_data_set_in_init_and_local_init_is_mutated_across_scan_calls() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        TableFunctionBuilder::new("tia_state")
            .param(TypeId::BigInt)
            .bind(state_bind)
            .init(state_init)
            .local_init(state_local_init)
            .scan(state_scan)
            .register(fx.con())
    }
    .expect("register tia_state");

    // 7 rows at 3 per call: three calls that produce rows, then one that
    // ends the scan. `call` and `global_calls` are the counters after the
    // call's own increment, so each is the same for the rows of one call and
    // grows between calls.
    let got = rows(&fx, "SELECT row, call, global_calls FROM tia_state(7)", 3);
    assert_eq!(
        got,
        vec![
            vec![0, 1, 1],
            vec![1, 1, 1],
            vec![2, 1, 1],
            vec![3, 2, 2],
            vec![4, 2, 2],
            vec![5, 2, 2],
            vec![6, 3, 3],
        ]
    );
    assert_eq!(GLOBAL_SET.load(Ordering::SeqCst), 1);
    assert_eq!(LOCAL_SET.load(Ordering::SeqCst), 1);

    // A second query gets fresh state, not the counters left by the first.
    let again = rows(&fx, "SELECT row, call FROM tia_state(4)", 2);
    assert_eq!(again, vec![vec![0, 1], vec![1, 1], vec![2, 1], vec![3, 2]]);

    // Both values are destroyed exactly once per query, by the destructors
    // `set` registered.
    drop(fx);
    assert_eq!(GLOBAL_SET.load(Ordering::SeqCst), 2);
    assert_eq!(GLOBAL_DROPPED.load(Ordering::SeqCst), 2, "global init data");
    assert_eq!(LOCAL_SET.load(Ordering::SeqCst), 2);
    assert_eq!(LOCAL_DROPPED.load(Ordering::SeqCst), 2, "local init data");
}

/// A function that sets no init data gets `None` from every accessor, rather
/// than a dangling or default-constructed value.
#[test]
fn init_data_that_was_never_set_reads_as_none() {
    static DONE: AtomicBool = AtomicBool::new(false);

    quack_rs::table_bind_callback!(none_bind, |info| {
        // SAFETY: `info` is the live bind info.
        unsafe { BindInfo::new(info) }.add_result_column("all_none", TypeId::BigInt);
    });
    quack_rs::table_init_callback!(none_init, |info| {
        let _ = info;
    });
    quack_rs::table_scan_callback!(none_scan, |info, output| {
        // SAFETY: `output` is the live chunk, `info` the live scan info.
        let chunk = unsafe { DataChunk::from_raw(output) };
        let all_none = unsafe {
            FfiInitData::<u64>::get(info).is_none()
                && FfiInitData::<u64>::get_mut(info).is_none()
                && FfiLocalInitData::<u64>::get(info).is_none()
                && FfiLocalInitData::<u64>::get_mut(info).is_none()
        };
        // The first call reports one row; the next ends the scan.
        if DONE.swap(true, Ordering::SeqCst) {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            return;
        }
        // SAFETY: one BIGINT row.
        unsafe {
            chunk.writer(0).write_i64(0, i64::from(all_none));
            chunk.set_size(1);
        }
    });

    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        TableFunctionBuilder::new("tia_none")
            .bind(none_bind)
            .init(none_init)
            .scan(none_scan)
            .register(fx.con())
    }
    .expect("register tia_none");
    assert_eq!(
        rows(&fx, "SELECT all_none FROM tia_none()", 1),
        vec![vec![1]]
    );
}

/// With several scan threads, every thread's `local_init` data is created and
/// destroyed, and the shared global data (read through `get`, behind atomics)
/// hands each row to exactly one of them.
#[test]
fn a_parallel_scan_gives_each_thread_its_own_local_data() {
    const TOTAL: i64 = 20_000;
    static NEXT: AtomicI64 = AtomicI64::new(0);
    static SET: AtomicUsize = AtomicUsize::new(0);
    static DROPPED: AtomicUsize = AtomicUsize::new(0);

    struct ThreadLocal(i64);
    impl Drop for ThreadLocal {
        fn drop(&mut self) {
            DROPPED.fetch_add(1, Ordering::SeqCst);
        }
    }

    quack_rs::table_bind_callback!(par_bind, |info| {
        // SAFETY: `info` is the live bind info.
        unsafe { BindInfo::new(info) }.add_result_column("n", TypeId::BigInt);
    });
    quack_rs::table_init_callback!(par_init, |info| {
        // SAFETY: global init.
        let init = unsafe { InitInfo::new(info) };
        init.set_max_threads(4);
        // SAFETY: global init data, set once.
        unsafe { FfiInitData::<AtomicI64>::set(info, AtomicI64::new(0)) };
    });
    quack_rs::table_init_callback!(par_local_init, |info| {
        // SAFETY: `local_init`.
        unsafe { FfiLocalInitData::<ThreadLocal>::set(info, ThreadLocal(0)) };
        SET.fetch_add(1, Ordering::SeqCst);
    });
    quack_rs::table_scan_callback!(par_scan, |info, output| {
        // SAFETY: `output` is the live chunk. Global data is only read (it
        // holds an atomic); local data is this task's own.
        let chunk = unsafe { DataChunk::from_raw(output) };
        let global = unsafe { FfiInitData::<AtomicI64>::get(info) }.expect("global init data");
        let local = unsafe { FfiLocalInitData::<ThreadLocal>::get_mut(info) }.expect("local data");
        let start = global.fetch_add(100, Ordering::SeqCst);
        let count = (TOTAL - start).clamp(0, 100);
        for i in 0..count {
            // SAFETY: one BIGINT column and at most 100 rows.
            unsafe { chunk.writer(0).write_i64(i as usize, start + i) };
        }
        local.0 += count;
        NEXT.fetch_add(count, Ordering::SeqCst);
        // SAFETY: `count` rows were written.
        unsafe { chunk.set_size(count as usize) };
    });

    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        TableFunctionBuilder::new("tia_parallel")
            .bind(par_bind)
            .init(par_init)
            .local_init(par_local_init)
            .scan(par_scan)
            .register(fx.con())
    }
    .expect("register tia_parallel");
    assert_eq!(
        rows(
            &fx,
            "SELECT count(*), sum(n)::BIGINT, count(DISTINCT n) FROM tia_parallel()",
            3
        ),
        vec![vec![TOTAL, TOTAL * (TOTAL - 1) / 2, TOTAL]]
    );
    assert_eq!(NEXT.load(Ordering::SeqCst), TOTAL);
    drop(fx);
    let created = SET.load(Ordering::SeqCst);
    assert!(
        (1..=4).contains(&created),
        "one local data per scanning thread, got {created}"
    );
    assert_eq!(DROPPED.load(Ordering::SeqCst), created);
}

// ─── The function's extra info, from each phase ──────────────────────────────

/// `BindInfo`, `InitInfo` and `FunctionInfo` all hand back the pointer the
/// function was registered with, and `DuckDB` frees it with the function.
#[test]
fn every_table_phase_reads_the_functions_extra_info() {
    use quack_rs::table::{FunctionInfo, InitInfo};
    use std::os::raw::c_void;

    const EXTRA: u64 = 0x5EED_CAFE;
    static EXTRA_DROPS: AtomicUsize = AtomicUsize::new(0);
    static SEEN: Mutex<Vec<(&str, u64)>> = Mutex::new(Vec::new());

    unsafe extern "C" fn drop_extra(ptr: *mut c_void) {
        if !ptr.is_null() {
            EXTRA_DROPS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: allocated by `Box::into_raw` below.
            drop(unsafe { Box::from_raw(ptr.cast::<u64>()) });
        }
    }
    fn note(phase: &'static str, ptr: *mut c_void) {
        // A null pointer is recorded as 0 so the test fails by assertion, not
        // by a crash in the callback.
        // SAFETY: the function was registered with a `Box<u64>`.
        let value = if ptr.is_null() {
            0
        } else {
            unsafe { *ptr.cast::<u64>() }
        };
        SEEN.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((phase, value));
    }

    quack_rs::table_bind_callback!(extra_bind, |info| {
        // SAFETY: `info` is the live bind info.
        let bind = unsafe { BindInfo::new(info) };
        note("bind", unsafe { bind.get_extra_info() });
        bind.add_result_column("n", TypeId::BigInt);
    });
    quack_rs::table_init_callback!(extra_init, |info| {
        // SAFETY: `info` is the live init info.
        note("init", unsafe { InitInfo::new(info).get_extra_info() });
    });
    quack_rs::table_scan_callback!(extra_scan, |info, output| {
        // SAFETY: `info` is the live scan info, `output` the live chunk.
        note("scan", unsafe { FunctionInfo::new(info).get_extra_info() });
        unsafe { DataChunk::from_raw(output).set_size(0) };
    });

    let fx = Fixture::open();
    let data = Box::into_raw(Box::new(EXTRA)).cast::<c_void>();
    // SAFETY: `con` is open; `data` is a Box that `drop_extra` frees once.
    unsafe {
        TableFunctionBuilder::new("tia_extra")
            .bind(extra_bind)
            .init(extra_init)
            .scan(extra_scan)
            .extra_info(data, drop_extra)
            .register(fx.con())
    }
    .expect("register tia_extra");
    fx.query("SELECT * FROM tia_extra()");
    let seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(seen.first().map(|(phase, _)| *phase), Some("bind"));
    for phase in ["bind", "init", "scan"] {
        assert!(
            seen.contains(&(phase, EXTRA)),
            "{phase} must see the registered value: {seen:x?}"
        );
    }
    assert!(seen.iter().all(|(_, v)| *v == EXTRA), "{seen:x?}");
    assert_eq!(
        EXTRA_DROPS.load(Ordering::SeqCst),
        0,
        "alive while registered"
    );
    drop(fx);
    assert_eq!(
        EXTRA_DROPS.load(Ordering::SeqCst),
        1,
        "freed once with the function"
    );
}
