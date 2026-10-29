// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The four copy-function info handles: `set_error` from each phase, and what
//! each phase can read of the state the earlier ones left.
//!
//! Requires `duckdb-1-5` (declared on the `mod` line).

use std::os::raw::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use quack_rs::client_context::ClientContext;
use quack_rs::copy_function::info::EMPTY_ERROR_PLACEHOLDER;
use quack_rs::copy_function::{
    CopyBindInfo, CopyFinalizeInfo, CopyFunctionBuilder, CopyGlobalInitInfo, CopySinkInfo,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::types::TypeId;

use super::Fixture;

// ─── set_error from each phase ───────────────────────────────────────────────

/// The phase that should fail, and with what message.
static FAIL: Mutex<(&str, String)> = Mutex::new(("", String::new()));
/// Which phases ran, in order, for the current `COPY`.
static RAN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Records that `phase` ran and reports whether it is the one to fail.
fn enter(phase: &'static str) -> Option<String> {
    RAN.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(phase);
    let fail = FAIL.lock().unwrap_or_else(PoisonError::into_inner);
    (fail.0 == phase).then(|| fail.1.clone())
}

quack_rs::copy_bind_callback!(fail_bind, |info| {
    // SAFETY: `info` is the live bind info.
    let bind = unsafe { CopyBindInfo::new(info) };
    if let Some(message) = enter("bind") {
        bind.set_error(&message);
    }
});

quack_rs::copy_global_init_callback!(fail_init, |info| {
    // SAFETY: `info` is the live global-init info.
    let init = unsafe { CopyGlobalInitInfo::new(info) };
    if let Some(message) = enter("global_init") {
        init.set_error(&message);
    }
});

quack_rs::copy_sink_callback!(fail_sink, |info, chunk| {
    let _ = chunk;
    // SAFETY: `info` is the live sink info.
    let sink = unsafe { CopySinkInfo::new(info) };
    if let Some(message) = enter("sink") {
        sink.set_error(&message);
    }
});

quack_rs::copy_finalize_callback!(fail_finalize, |info| {
    // SAFETY: `info` is the live finalize info.
    let finalize = unsafe { CopyFinalizeInfo::new(info) };
    if let Some(message) = enter("finalize") {
        finalize.set_error(&message);
    }
});

/// Each phase's `set_error` fails the `COPY` with its message, stops the
/// phases after it, and treats a message the way every other error channel
/// does: a NUL becomes `?` and an empty message becomes the placeholder.
#[test]
fn set_error_in_any_phase_fails_the_copy_and_stops_the_later_phases() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        CopyFunctionBuilder::try_new("copy_err")
            .expect("name")
            .bind(fail_bind)
            .global_init(fail_init)
            .sink(fail_sink)
            .finalize(fail_finalize)
            .register(fx.con())
    }
    .expect("register copy_err");
    let dir = tempfile::tempdir().expect("tempdir");
    let sql = format!(
        "COPY (SELECT 1 AS x) TO '{}' (FORMAT copy_err)",
        dir.path().join("out.err").display()
    );

    let phases = ["bind", "global_init", "sink", "finalize"];
    // A COPY that fails nowhere runs all four, once each: the baseline the
    // failing runs are cut short from.
    *FAIL.lock().unwrap_or_else(PoisonError::into_inner) = ("", String::new());
    RAN.lock().unwrap_or_else(PoisonError::into_inner).clear();
    fx.query(&sql);
    assert_eq!(*RAN.lock().unwrap_or_else(PoisonError::into_inner), phases);

    for (index, phase) in phases.into_iter().enumerate() {
        for (message, shown) in [
            ("plain message", "plain message"),
            ("with\0nul", "with?nul"),
            ("", EMPTY_ERROR_PLACEHOLDER),
        ] {
            *FAIL.lock().unwrap_or_else(PoisonError::into_inner) = (phase, message.to_owned());
            RAN.lock().unwrap_or_else(PoisonError::into_inner).clear();
            // SAFETY: `con` is open.
            let err = unsafe { quack_rs::query::query(fx.con(), &sql) }
                .map(drop)
                .expect_err(&format!("{phase} reported {message:?}"));
            assert!(
                err.as_str().contains(shown),
                "{phase} reported {message:?}, the query said: {err}"
            );
            assert_eq!(
                *RAN.lock().unwrap_or_else(PoisonError::into_inner),
                phases[..=index],
                "{phase} reported {message:?}: phases that ran"
            );
        }
    }

    // None of those left the connection unusable.
    *FAIL.lock().unwrap_or_else(PoisonError::into_inner) = ("", String::new());
    fx.query(&sql);
}

// ─── What each phase can read ────────────────────────────────────────────────

/// Passed as the function's extra info; every phase must see this value.
const EXTRA: u64 = 0x00C0_FFEE;
static EXTRA_DROPS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn drop_extra(ptr: *mut c_void) {
    if !ptr.is_null() {
        EXTRA_DROPS.fetch_add(1, Ordering::SeqCst);
        // SAFETY: allocated by `Box::into_raw` below.
        drop(unsafe { Box::from_raw(ptr.cast::<u64>()) });
    }
}

struct BindState {
    columns: u64,
}

struct GlobalState {
    rows: u64,
}

/// What each phase observed, formatted for one comparison.
static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn note(line: String) {
    SEEN.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(line);
}

/// The connection id `get_client_context` reported, rendered.
fn connection_of(ctx: &ClientContext) -> String {
    format!("conn={}", ctx.connection_id())
}

unsafe extern "C" fn drop_bind_state(ptr: *mut c_void) {
    if !ptr.is_null() {
        // SAFETY: allocated by `Box::into_raw` in the bind callback.
        drop(unsafe { Box::from_raw(ptr.cast::<BindState>()) });
    }
}

unsafe extern "C" fn drop_global_state(ptr: *mut c_void) {
    if !ptr.is_null() {
        // SAFETY: allocated by `Box::into_raw` in the global-init callback.
        drop(unsafe { Box::from_raw(ptr.cast::<GlobalState>()) });
    }
}

quack_rs::copy_bind_callback!(state_bind, |info| {
    // SAFETY: `info` is the live bind info.
    let bind = unsafe { CopyBindInfo::new(info) };
    let count = bind.column_count();
    // SAFETY: bind callback, and each index is below `count` or deliberately
    // past it (which must answer `None`).
    let types: Vec<Option<TypeId>> = (0..=count)
        .map(|i| unsafe { bind.column_type(i) }.and_then(|t| unsafe { t.try_get_type_id() }))
        .collect();
    // SAFETY: the extra info was set to a `Box<u64>` at registration.
    let extra = unsafe { *bind.get_extra_info().cast::<u64>() };
    // SAFETY: bind callback; the context is only used within it.
    let ctx = unsafe { bind.get_client_context() };
    note(format!(
        "bind: columns={count} types={types:?} extra={extra:#x} {}",
        connection_of(&ctx)
    ));
    let state = Box::new(BindState { columns: count });
    // SAFETY: a fresh Box, freed once by `drop_bind_state`.
    unsafe {
        bind.set_bind_data(Box::into_raw(state).cast::<c_void>(), Some(drop_bind_state));
    }
});

quack_rs::copy_global_init_callback!(state_init, |info| {
    // SAFETY: `info` is the live global-init info.
    let init = unsafe { CopyGlobalInitInfo::new(info) };
    // SAFETY: bind data was set by `state_bind`; extra info at registration.
    let (columns, extra) = unsafe {
        (
            (*init.get_bind_data().cast::<BindState>()).columns,
            *init.get_extra_info().cast::<u64>(),
        )
    };
    // SAFETY: the destination path is available during global init.
    let (path, bytes) = unsafe { (init.get_file_path(), init.get_file_path_bytes()) };
    let path = path.expect("a UTF-8 path");
    assert_eq!(
        path.as_bytes(),
        bytes.as_slice(),
        "the two path getters agree"
    );
    // SAFETY: global-init callback.
    let ctx = unsafe { init.get_client_context() };
    note(format!(
        "init: columns={columns} extra={extra:#x} file={} {}",
        path.rsplit('/').next().unwrap_or_default(),
        connection_of(&ctx)
    ));
    let state = Box::new(GlobalState { rows: 0 });
    // SAFETY: a fresh Box, freed once by `drop_global_state`.
    unsafe {
        init.set_global_state(
            Box::into_raw(state).cast::<c_void>(),
            Some(drop_global_state),
        );
    }
});

quack_rs::copy_sink_callback!(state_sink, |info, chunk| {
    // SAFETY: `info` is the live sink info and `chunk` its input chunk.
    let (sink, chunk) = unsafe { (CopySinkInfo::new(info), DataChunk::from_raw(chunk)) };
    // SAFETY: bind data, global state and extra info were all set earlier;
    // sink calls are serialised for this single-threaded export.
    let (columns, extra, state) = unsafe {
        (
            (*sink.get_bind_data().cast::<BindState>()).columns,
            *sink.get_extra_info().cast::<u64>(),
            &mut *sink.get_global_state().cast::<GlobalState>(),
        )
    };
    state.rows += chunk.size() as u64;
    // SAFETY: sink callback.
    let ctx = unsafe { sink.get_client_context() };
    note(format!(
        "sink: columns={columns} extra={extra:#x} rows_so_far={} {}",
        state.rows,
        connection_of(&ctx)
    ));
});

quack_rs::copy_finalize_callback!(state_finalize, |info| {
    // SAFETY: `info` is the live finalize info.
    let fin = unsafe { CopyFinalizeInfo::new(info) };
    // SAFETY: all three were set by earlier phases.
    let (columns, extra, rows) = unsafe {
        (
            (*fin.get_bind_data().cast::<BindState>()).columns,
            *fin.get_extra_info().cast::<u64>(),
            (*fin.get_global_state().cast::<GlobalState>()).rows,
        )
    };
    // SAFETY: finalize callback.
    let ctx = unsafe { fin.get_client_context() };
    note(format!(
        "finalize: columns={columns} extra={extra:#x} rows={rows} {}",
        connection_of(&ctx)
    ));
});

#[test]
fn each_phase_reads_the_state_the_earlier_phases_left() {
    let fx = Fixture::open();
    let extra = Box::into_raw(Box::new(EXTRA)).cast::<c_void>();
    // SAFETY: `con` is open; `extra` is a Box `drop_extra` frees exactly once.
    unsafe {
        CopyFunctionBuilder::try_new("copy_state")
            .expect("name")
            .bind(state_bind)
            .global_init(state_init)
            .sink(state_sink)
            .finalize(state_finalize)
            .extra_info(extra, Some(drop_extra))
            .register(fx.con())
    }
    .expect("register copy_state");
    // SAFETY: `con` is open.
    let conn = format!(
        "conn={}",
        unsafe { ClientContext::from_connection(fx.con()) }
            .expect("a client context")
            .connection_id()
    );

    let dir = tempfile::tempdir().expect("tempdir");
    SEEN.lock().unwrap_or_else(PoisonError::into_inner).clear();
    fx.query(&format!(
        "COPY (SELECT i::BIGINT AS a, i::VARCHAR AS b, i::DOUBLE AS c FROM range(5000) t(i)) \
         TO '{}' (FORMAT copy_state)",
        dir.path().join("out.state").display()
    ));
    let seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner).clone();

    // 5000 rows spans several chunks; the exact split is DuckDB's, so check
    // the first and last sink lines and the running count instead.
    let sinks: Vec<&String> = seen.iter().filter(|l| l.starts_with("sink:")).collect();
    assert!(sinks.len() >= 2, "a multi-chunk export: {seen:#?}");
    let last_sink = sinks.last().expect("a sink call");
    assert_eq!(
        **last_sink,
        format!("sink: columns=3 extra=0xc0ffee rows_so_far=5000 {conn}")
    );
    assert_eq!(
        seen.first().map(String::as_str),
        Some(
            format!(
                "bind: columns=3 types=[Some(BigInt), Some(Varchar), Some(Double), None] \
                 extra=0xc0ffee {conn}"
            )
            .as_str()
        ),
        "bind sees every column's type, and None past the last"
    );
    assert_eq!(
        seen.get(1).map(String::as_str),
        Some(format!("init: columns=3 extra=0xc0ffee file=out.state {conn}").as_str())
    );
    assert_eq!(
        seen.last().map(String::as_str),
        Some(format!("finalize: columns=3 extra=0xc0ffee rows=5000 {conn}").as_str())
    );

    // The extra info is released with the function, once.
    drop(fx);
    assert_eq!(EXTRA_DROPS.load(Ordering::SeqCst), 1);
}
