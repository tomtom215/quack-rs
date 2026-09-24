// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;
use crate::types::{NullHandling, TypeId};
use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};

// Callback stubs shared by the function-set tests. They are never invoked --
// these tests only assert on what the builder records -- but the builder's
// setters are typed, so real `extern "C"` items are needed.
unsafe extern "C" fn ss(_: duckdb_function_info) -> idx_t {
    0
}
unsafe extern "C" fn si(_: duckdb_function_info, _: duckdb_aggregate_state) {}
unsafe extern "C" fn su(
    _: duckdb_function_info,
    _: duckdb_data_chunk,
    _: *mut duckdb_aggregate_state,
) {
}
unsafe extern "C" fn sc(
    _: duckdb_function_info,
    _: *mut duckdb_aggregate_state,
    _: *mut duckdb_aggregate_state,
    _: idx_t,
) {
}
unsafe extern "C" fn sf(
    _: duckdb_function_info,
    _: *mut duckdb_aggregate_state,
    _: duckdb_vector,
    _: idx_t,
    _: idx_t,
) {
}
unsafe extern "C" fn sd(_: *mut duckdb_aggregate_state, _: idx_t) {}

// Verify that AggregateFunctionBuilder stores name correctly
#[test]
fn builder_stores_name() {
    let b = AggregateFunctionBuilder::new("my_func");
    assert_eq!(b.name.to_str().unwrap(), "my_func");
}

#[test]
fn builder_stores_params() {
    let b = AggregateFunctionBuilder::new("f")
        .param(TypeId::BigInt)
        .param(TypeId::Varchar);
    assert_eq!(b.params.len(), 2);
    assert_eq!(b.params[0], TypeId::BigInt);
    assert_eq!(b.params[1], TypeId::Varchar);
}

#[test]
fn builder_stores_return_type() {
    let b = AggregateFunctionBuilder::new("f").returns(TypeId::BigInt);
    assert_eq!(b.return_type, Some(TypeId::BigInt));
}

#[test]
fn function_set_builder_stores_overloads() {
    let b = AggregateFunctionSetBuilder::new("retention")
        .returns(TypeId::BigInt)
        .overloads(2..=4, |n, builder| {
            (0..n)
                .fold(builder, |b, _| b.param(TypeId::Boolean))
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf)
        });

    // overloads(2..=4) = 3 overloads (n=2, n=3, n=4)
    assert_eq!(b.overloads.len(), 3);
    assert_eq!(b.overloads[0].params.len(), 2);
    assert_eq!(b.overloads[1].params.len(), 3);
    assert_eq!(b.overloads[2].params.len(), 4);
}

#[test]
fn register_missing_return_type_returns_error() {
    let b = AggregateFunctionBuilder::new("f");
    // We can't call register with a null connection, but we can verify
    // the error path for missing return type by inspecting the error.
    // In a real integration test, we'd call register(con) with a live connection.
    // Here we verify the builder stores None for return_type.
    assert!(b.return_type.is_none());
}

#[test]
fn function_set_builder_name() {
    let b = AggregateFunctionSetBuilder::new("my_set");
    assert_eq!(b.name.to_str().unwrap(), "my_set");
}

#[test]
fn overload_builder_params() {
    let ob = AggregateOverloadBuilder::new()
        .param(TypeId::Boolean)
        .param(TypeId::Boolean)
        .param(TypeId::BigInt);
    assert_eq!(ob.params.len(), 3);
}

#[test]
fn overload_builder_default_matches_new() {
    let d = AggregateOverloadBuilder::default();
    assert_eq!(d.params.len(), 0);
    assert!(d.return_type.is_none());
    assert!(d.return_logical.is_none());
    assert_eq!(d.null_handling, NullHandling::DefaultNullHandling);
}

// The callback setters are `const fn`, which makes their cargo-mutants mutants
// *unviable* rather than caught: the replacement `Default::default()` cannot be
// called in a const context, so it fails to compile and the mutation gate is
// structurally silent about them. That is a property of the gate, not evidence
// the setters work -- so assert it directly.
#[test]
fn every_overload_setter_stores_into_its_own_field() {
    let ob = AggregateOverloadBuilder::new()
        .state_size(ss)
        .init(si)
        .update(su)
        .combine(sc)
        .finalize(sf)
        .destructor(sd)
        .null_handling(NullHandling::SpecialNullHandling);

    // Six setters, six fields, every one `Some`. That is complete: a setter
    // cannot store into another callback's field, because `StateSizeFn`,
    // `StateInitFn`, `UpdateFn`, `CombineFn`, `FinalizeFn` and `DestroyFn` are
    // six distinct `extern "C"` signatures and any cross-wiring fails to
    // compile. If a setter wrote to the wrong field, another would be `None`.
    //
    // An earlier version also compared each stored pointer against its input
    // with `std::ptr::fn_addr_eq`. That was unsound, not just redundant: the
    // function's own documentation says pointers to the same function may
    // compare unequal, because codegen may emit more than one address for it.
    // Miri models that and failed the `state_size` comparison while the native
    // build happened to pass -- caught by the Miri job this branch repaired.
    assert!(ob.state_size.is_some(), "state_size");
    assert!(ob.init.is_some(), "init");
    assert!(ob.update.is_some(), "update");
    assert!(ob.combine.is_some(), "combine");
    assert!(ob.finalize.is_some(), "finalize");
    assert!(ob.destructor.is_some(), "destructor");
    assert_eq!(ob.null_handling, NullHandling::SpecialNullHandling);
}

#[test]
fn a_setter_left_unset_stays_none() {
    // The complement of the test above: nothing is set by default, so the
    // assertions there cannot pass vacuously.
    let ob = AggregateOverloadBuilder::new();
    assert!(ob.state_size.is_none());
    assert!(ob.init.is_none());
    assert!(ob.update.is_none());
    assert!(ob.combine.is_none());
    assert!(ob.finalize.is_none());
    assert!(ob.destructor.is_none());
    assert_eq!(ob.null_handling, NullHandling::DefaultNullHandling);
}

#[test]
fn the_set_builder_reports_its_own_name() {
    assert_eq!(
        AggregateFunctionSetBuilder::new("retention").name(),
        "retention"
    );
    assert_eq!(
        AggregateFunctionSetBuilder::try_new("word_count")
            .expect("valid name")
            .name(),
        "word_count"
    );
}

#[test]
fn overload_builder_stores_its_own_return_type() {
    let ob = AggregateOverloadBuilder::new()
        .param(TypeId::Integer)
        .returns(TypeId::Integer);
    assert_eq!(ob.return_type, Some(TypeId::Integer));
}

// Issue #121: overloads in one set may return different types, because DuckDB
// resolves an aggregate overload from parameter types and arity alone.
#[test]
fn a_set_keeps_a_distinct_return_type_per_overload() {
    let b = AggregateFunctionSetBuilder::new("my_agg")
        .overload(
            AggregateOverloadBuilder::new()
                .param(TypeId::Integer)
                .returns(TypeId::Integer)
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf),
        )
        .overload(
            AggregateOverloadBuilder::new()
                .param(TypeId::Varchar)
                .returns(TypeId::Varchar)
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf),
        );

    assert_eq!(b.overloads.len(), 2);
    assert_eq!(b.overloads[0].params, vec![TypeId::Integer]);
    assert_eq!(b.overloads[0].return_type, Some(TypeId::Integer));
    assert_eq!(b.overloads[1].params, vec![TypeId::Varchar]);
    assert_eq!(b.overloads[1].return_type, Some(TypeId::Varchar));
    // No set-level default was needed.
    assert!(b.return_type.is_none());
    assert!(b.return_logical.is_none());
}

#[test]
fn overload_and_overloads_can_be_mixed_and_keep_insertion_order() {
    let b = AggregateFunctionSetBuilder::new("mixed")
        .returns(TypeId::BigInt)
        .overload(AggregateOverloadBuilder::new().param(TypeId::Varchar))
        .overloads(2..=3, |n, builder| {
            (0..n).fold(builder, |b, _| b.param(TypeId::Boolean))
        });

    assert_eq!(b.overloads.len(), 3);
    assert_eq!(b.overloads[0].params, vec![TypeId::Varchar]);
    assert_eq!(b.overloads[1].params.len(), 2);
    assert_eq!(b.overloads[2].params.len(), 3);
    // The `overloads` members inherit the set-level default: none of their own.
    assert!(b.overloads[1].return_type.is_none());
}

#[test]
fn overloads_closure_can_set_a_per_arity_return_type() {
    let b = AggregateFunctionSetBuilder::new("per_arity").overloads(1..=2, |n, builder| {
        let builder = (0..n).fold(builder, |b, _| b.param(TypeId::Integer));
        if n == 1 {
            builder.returns(TypeId::Integer)
        } else {
            builder.returns(TypeId::BigInt)
        }
    });

    assert_eq!(b.overloads[0].return_type, Some(TypeId::Integer));
    assert_eq!(b.overloads[1].return_type, Some(TypeId::BigInt));
    // Building a `LogicalType` needs a live DuckDB dispatch table, so the
    // `returns_logical` override is covered in `tests/ffi_roundtrip.rs`.
}

#[test]
fn an_empty_set_is_rejected_before_any_return_type_check() {
    // Registration needs a live connection, so assert the precondition the
    // error path keys off: no overloads and no return type at all.
    let b = AggregateFunctionSetBuilder::new("empty");
    assert_eq!(b.overloads.len(), 0);
    assert!(b.return_type.is_none());
}

#[test]
fn the_deprecated_alias_still_names_the_same_type() {
    #[allow(deprecated)]
    let ob: crate::aggregate::builder::OverloadBuilder =
        AggregateOverloadBuilder::new().param(TypeId::Boolean);
    assert_eq!(ob.params.len(), 1);
}

#[test]
fn try_new_valid_name() {
    assert!(AggregateFunctionBuilder::try_new("word_count").is_ok());
}

#[test]
fn try_new_empty_rejected() {
    assert!(AggregateFunctionBuilder::try_new("").is_err());
}

#[test]
fn try_new_accepts_mixed_case_and_rejects_names_needing_quotes() {
    // DuckDB ships mixed-case functions and accepts registering one, so
    // rejecting them here made a legal name unregisterable.
    assert!(AggregateFunctionBuilder::try_new("MyFunc").is_ok());
    assert!(AggregateFunctionBuilder::try_new("my-func").is_err());
    assert!(AggregateFunctionBuilder::try_new("1func").is_err());
}

#[test]
fn try_new_hyphen_rejected() {
    assert!(AggregateFunctionBuilder::try_new("my-func").is_err());
}

#[test]
fn set_try_new_valid_name() {
    assert!(AggregateFunctionSetBuilder::try_new("retention").is_ok());
}

#[test]
fn set_try_new_empty_rejected() {
    assert!(AggregateFunctionSetBuilder::try_new("").is_err());
}

/// Same up-front validation as the scalar set: the overload index is named
/// and no `DuckDB` call is made (a null connection would crash otherwise).
#[test]
fn an_incomplete_aggregate_overload_is_reported_by_index_before_touching_duckdb() {
    let set = AggregateFunctionSetBuilder::new("s")
        .overload(
            AggregateOverloadBuilder::new()
                .returns(TypeId::BigInt)
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf),
        )
        .overload(
            AggregateOverloadBuilder::new()
                .param(TypeId::Double)
                .returns(TypeId::BigInt)
                .state_size(ss)
                .init(si)
                .update(su)
                .finalize(sf),
        );
    // SAFETY: validation fails before `con` is used.
    let err = unsafe { set.register(std::ptr::null_mut()) }.expect_err("no combine");
    assert!(err.as_str().contains("overload 1"), "{err}");
    assert!(err.as_str().contains("combine"), "{err}");

    // SAFETY: as above.
    let err = unsafe { AggregateFunctionSetBuilder::new("s").register(std::ptr::null_mut()) }
        .expect_err("no overloads");
    assert!(err.as_str().contains("no overloads"), "{err}");

    let no_return = AggregateFunctionSetBuilder::new("s").overload(
        AggregateOverloadBuilder::new()
            .state_size(ss)
            .init(si)
            .update(su)
            .combine(sc)
            .finalize(sf),
    );
    // SAFETY: as above.
    let err = unsafe { no_return.register(std::ptr::null_mut()) }.expect_err("no return type");
    assert!(err.as_str().contains("overload 0"), "{err}");
}

/// A set-level return type is enough on its own: an overload without one of
/// its own inherits it, and the completeness check accepts the set.
#[test]
fn a_set_level_return_type_completes_an_overload_without_one() {
    let set = AggregateFunctionSetBuilder::new("s")
        .returns(TypeId::BigInt)
        .overload(
            AggregateOverloadBuilder::new()
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf),
        );
    assert!(set.check_parts().is_ok(), "{:?}", set.check_parts());
}

/// `AggregateOverloadBuilder::extra_info` mirrors
/// `AggregateFunctionBuilder::extra_info`, including ownership: an overload
/// that never reaches `DuckDB` frees its allocation exactly once.
#[test]
fn an_aggregate_overloads_extra_info_is_freed_once_when_the_set_is_rejected() {
    use std::os::raw::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static FREED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn free_it(p: *mut c_void) {
        // SAFETY: allocated below by `Box::into_raw`.
        drop(unsafe { Box::from_raw(p.cast::<u8>()) });
        FREED.fetch_add(1, Ordering::SeqCst);
    }
    let data = || Box::into_raw(Box::new(7_u8)).cast::<c_void>();

    // SAFETY: `free_it` frees exactly what `data` allocates.
    let complete = unsafe {
        AggregateOverloadBuilder::new()
            .returns(TypeId::BigInt)
            .state_size(ss)
            .init(si)
            .update(su)
            .combine(sc)
            .finalize(sf)
            .extra_info(data(), Some(free_it))
    };
    // SAFETY: as above.
    let incomplete = unsafe { AggregateOverloadBuilder::new().extra_info(data(), Some(free_it)) };
    let set = AggregateFunctionSetBuilder::new("s")
        .overload(complete)
        .overload(incomplete);
    // SAFETY: validation fails before `con` is used.
    assert!(unsafe { set.register(std::ptr::null_mut()) }.is_err());
    assert_eq!(FREED.load(Ordering::SeqCst), 2);

    // Dropping an unregistered overload frees it too.
    // SAFETY: as above.
    drop(unsafe { AggregateOverloadBuilder::new().extra_info(data(), Some(free_it)) });
    assert_eq!(FREED.load(Ordering::SeqCst), 3);
}

/// `ffi_state::<T>()` installs a size, init and destroy callback that all
/// describe `T`: checked by running them, since Rust does not promise that
/// two uses of a function's address compare equal (Miri makes them differ).
#[test]
fn ffi_state_installs_all_three_state_callbacks_for_one_type() {
    use crate::aggregate::callbacks::{DestroyFn, StateInitFn, StateSizeFn};
    use crate::aggregate::{AggregateState, FfiState};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DROPS: AtomicUsize = AtomicUsize::new(0);
    #[derive(Default)]
    struct Wide([u64; 8]);
    impl Drop for Wide {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl AggregateState for Wide {}

    let run = |size: Option<StateSizeFn>, init: Option<StateInitFn>, destroy: Option<DestroyFn>| {
        let (size, init, destroy) = (
            size.expect("size"),
            init.expect("init"),
            destroy.expect("destroy"),
        );
        // SAFETY: `FfiState`'s size callback does not read its argument.
        let bytes = unsafe { size(std::ptr::null_mut()) };
        assert_eq!(bytes as usize, FfiState::<Wide>::size());
        let word = std::mem::size_of::<usize>();
        let mut buffer = vec![0_usize; FfiState::<Wide>::size().div_ceil(word)];
        let mut state: libduckdb_sys::duckdb_aggregate_state = buffer.as_mut_ptr().cast();
        // SAFETY: `state` is `size()` writable, word-aligned bytes, and
        // `Wide::default` does not panic, so `info` is never read.
        unsafe { init(std::ptr::null_mut(), state) };
        // SAFETY: `state` was just initialised by an `FfiState` callback.
        let wide = unsafe { FfiState::<Wide>::with_state(state) }.expect("a Wide was built");
        assert_eq!(wide.0, [0; 8]);
        let before = DROPS.load(Ordering::SeqCst);
        // SAFETY: one initialised state of `size()` word-aligned bytes.
        unsafe { destroy(&raw mut state, 1) };
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            before + 1,
            "the Wide was dropped"
        );
    };

    let single = AggregateFunctionBuilder::new("f").ffi_state::<Wide>();
    run(single.state_size, single.init, single.destructor);
    let overload = AggregateOverloadBuilder::new().ffi_state::<Wide>();
    run(overload.state_size, overload.init, overload.destructor);
}
