// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Lifecycle paths no other test reached, found by mutation testing:
//!
//! - `ScalarBindData::get_from_init` (replaced by `None`, every test passed);
//! - the `extra_info` destructors of the closure-based builders,
//!   `TypedScalar::destroy` and the typed table function's `destroy_extra`
//!   (replaced by `()`, every test passed: the leaked closure and everything
//!   it captured would only show to a leak checker, and the leak-check CI job
//!   suppresses every allocation that passes through `libduckdb.so`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use quack_rs::types::TypeId;

use super::Fixture;

/// Counts its own drops.
struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(feature = "duckdb-1-5")]
mod init_reads_bind_data {
    use quack_rs::data_chunk::DataChunk;
    use quack_rs::scalar::{
        RawScalarBindInfo, RawScalarInitInfo, ScalarBindData, ScalarBindInfo,
        ScalarFunctionBuilder, ScalarFunctionInfo, ScalarInitInfo, ScalarLocalState,
    };
    use quack_rs::types::TypeId;
    use quack_rs::vector::VectorWriter;

    use super::Fixture;

    unsafe extern "C" fn bind(info: RawScalarBindInfo) {
        // SAFETY: DuckDB passes a valid bind info.
        let bind = unsafe { ScalarBindInfo::new(info) };
        ScalarBindData::set(&bind, 4242_i64);
    }

    /// Copies the bind data into the thread's local state: the only reader of
    /// `get_from_init` in the suite.
    unsafe extern "C" fn init(info: RawScalarInitInfo) {
        // SAFETY: DuckDB passes a valid init info; `bind` stored an i64.
        let init = unsafe { ScalarInitInfo::new(info) };
        let seen = unsafe { ScalarBindData::<i64>::get_from_init(&init) }
            .copied()
            .unwrap_or(-1);
        ScalarLocalState::set(&init, seen);
    }

    quack_rs::scalar_callback!(exec, |info, input, output| {
        // SAFETY: DuckDB passes a valid function info; `init` stored an i64.
        let fninfo = unsafe { ScalarFunctionInfo::new(info) };
        let Some(seen) = (unsafe { ScalarLocalState::<i64>::get_mut(&fninfo) }).copied() else {
            fninfo.set_error("local state missing");
            return;
        };
        // SAFETY: the output is BIGINT.
        let chunk = unsafe { DataChunk::from_raw(input) };
        let mut writer = unsafe { VectorWriter::from_vector(output) };
        for row in 0..chunk.size() {
            unsafe { writer.write_i64(row, seen) };
        }
    });

    #[test]
    fn the_init_callback_reads_the_bind_data_the_bind_callback_stored() {
        let fx = Fixture::open();
        // SAFETY: `con` is open; every callback matches its declared signature.
        unsafe {
            ScalarFunctionBuilder::try_new("init_sees_bind")
                .expect("name")
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .bind(bind)
                .init(init)
                .function(exec)
                .register(fx.con())
                .expect("register");
        }
        assert_eq!(
            fx.scalar("SELECT init_sees_bind(1)", |r, i| unsafe { r.read_i64(i) }),
            Some(4242)
        );
    }
}

#[test]
fn a_typed_scalar_closure_and_what_it_captured_are_dropped_with_the_database() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let drops = Arc::new(AtomicUsize::new(0));
    let fx = Fixture::open();
    let guard = DropCounter(Arc::clone(&drops));
    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::map1("captures_a_guard", move |x: i64| {
            let _ = &guard;
            x + 1
        })
        .expect("build")
        .register(fx.con())
        .expect("register");
    }
    assert_eq!(
        fx.scalar("SELECT captures_a_guard(41)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(42)
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0, "still registered");
    drop(fx);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "closing the database must run the extra_info destructor exactly once"
    );
}

#[test]
fn a_typed_table_functions_closures_are_dropped_with_the_database() {
    use quack_rs::table::TableFunctionBuilder;

    let drops = Arc::new(AtomicUsize::new(0));
    let fx = Fixture::open();
    let guard = DropCounter(Arc::clone(&drops));
    let builder = TableFunctionBuilder::new("captures_a_guard_table")
        .with_state::<i64, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            Ok(1)
        })
        .scan(move |state, chunk| {
            let _ = &guard;
            if *state == 0 {
                // SAFETY: end of stream.
                unsafe { chunk.set_size(0) };
                return Ok(());
            }
            *state = 0;
            // SAFETY: one BIGINT column, one row.
            unsafe {
                chunk.writer(0).write_i64(0, 7);
                chunk.set_size(1);
            }
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register");
    assert_eq!(
        fx.scalar("SELECT n FROM captures_a_guard_table()", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(7)
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0, "still registered");
    drop(fx);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "closing the database must run the extra_info destructor exactly once"
    );
}

/// `abi::check` returns `StableOnly` for a build that reaches no unstable
/// slot and a real verdict for one that does; the engine the live suite runs
/// on is the one the bindings were generated from, so the verdict is
/// `Compatible`. Replacing the guard's condition by its negation left every
/// test green (the end-to-end ABI guard jobs load real extensions, which no
/// `cargo test` run sees).
#[test]
fn abi_check_gives_a_layout_verdict_exactly_when_the_unstable_api_is_used() {
    use quack_rs::abi::{check, uses_unstable_api, AbiCheck};

    let _fx = Fixture::open();
    // SAFETY: the fixture initialised the dispatch table.
    let verdict = unsafe { check() };
    if uses_unstable_api() {
        assert!(
            matches!(verdict, AbiCheck::Compatible { .. }),
            "an engine and bindings of one release agree on the layout: {verdict:?}"
        );
    } else {
        assert_eq!(verdict, AbiCheck::StableOnly);
    }
}
