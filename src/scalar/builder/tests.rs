// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;
use crate::types::TypeId;
use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};

#[test]
fn builder_stores_name() {
    let b = ScalarFunctionBuilder::new("my_scalar");
    assert_eq!(b.name.to_str().unwrap(), "my_scalar");
}

#[test]
fn builder_stores_params() {
    let b = ScalarFunctionBuilder::new("f")
        .param(TypeId::BigInt)
        .param(TypeId::Varchar);
    assert_eq!(b.params.len(), 2);
    assert_eq!(b.params[0], TypeId::BigInt);
    assert_eq!(b.params[1], TypeId::Varchar);
}

#[test]
fn builder_stores_return_type() {
    let b = ScalarFunctionBuilder::new("f").returns(TypeId::Double);
    assert_eq!(b.return_type, Some(TypeId::Double));
}

#[test]
fn builder_missing_return_type() {
    let b = ScalarFunctionBuilder::new("f");
    assert!(b.return_type.is_none());
}

#[test]
fn builder_missing_function() {
    let b = ScalarFunctionBuilder::new("f");
    assert!(b.function.is_none());
}

#[test]
fn builder_stores_function() {
    unsafe extern "C" fn my_func(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

    let b = ScalarFunctionBuilder::new("f").function(my_func);
    assert!(b.function.is_some());
}

#[test]
fn try_new_valid_name() {
    let b = ScalarFunctionBuilder::try_new("word_count");
    assert!(b.is_ok());
}

#[test]
fn try_new_empty_rejected() {
    assert!(ScalarFunctionBuilder::try_new("").is_err());
}

#[test]
fn try_new_accepts_mixed_case_and_rejects_names_needing_quotes() {
    // DuckDB ships mixed-case functions and accepts registering one, so
    // rejecting them here made a legal name unregisterable.
    assert!(ScalarFunctionBuilder::try_new("MyFunc").is_ok());
    assert!(ScalarFunctionBuilder::try_new("my-func").is_err());
    assert!(ScalarFunctionBuilder::try_new("1func").is_err());
}

#[test]
fn try_new_hyphen_rejected() {
    assert!(ScalarFunctionBuilder::try_new("my-func").is_err());
}

// --- ScalarFunctionSetBuilder tests ---

#[test]
fn set_builder_stores_name() {
    let b = ScalarFunctionSetBuilder::new("my_set");
    assert_eq!(b.name.to_str().unwrap(), "my_set");
}

#[test]
fn set_builder_stores_overloads() {
    unsafe extern "C" fn f1(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
    unsafe extern "C" fn f2(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

    let b = ScalarFunctionSetBuilder::new("my_add")
        .overload(
            ScalarOverloadBuilder::new()
                .param(TypeId::Integer)
                .param(TypeId::Integer)
                .returns(TypeId::Integer)
                .function(f1),
        )
        .overload(
            ScalarOverloadBuilder::new()
                .param(TypeId::Double)
                .param(TypeId::Double)
                .returns(TypeId::Double)
                .function(f2),
        );

    assert_eq!(b.overloads.len(), 2);
    assert_eq!(b.overloads[0].params.len(), 2);
    assert_eq!(b.overloads[1].params.len(), 2);
}

#[test]
fn set_try_new_valid_name() {
    assert!(ScalarFunctionSetBuilder::try_new("my_add").is_ok());
}

#[test]
fn set_try_new_empty_rejected() {
    assert!(ScalarFunctionSetBuilder::try_new("").is_err());
}

#[test]
fn overload_builder_default() {
    let ob = ScalarOverloadBuilder::default();
    assert_eq!(ob.params, [] as [TypeId; 0]);
    assert!(ob.return_type.is_none());
    assert!(ob.function.is_none());
}

#[test]
fn overload_builder_stores_fields() {
    unsafe extern "C" fn f(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

    let ob = ScalarOverloadBuilder::new()
        .param(TypeId::BigInt)
        .returns(TypeId::Varchar)
        .function(f);
    assert_eq!(ob.params.len(), 1);
    assert_eq!(ob.return_type, Some(TypeId::Varchar));
    assert!(ob.function.is_some());
}

#[test]
fn overload_builder_mirrors_the_single_builders_stability_setter() {
    let ob = ScalarOverloadBuilder::new();
    assert!(!ob.volatile);
    assert!(ob.volatile().volatile);
}

#[cfg(feature = "duckdb-1-5")]
#[test]
fn overload_builder_mirrors_the_single_builders_bind_and_init_setters() {
    unsafe extern "C" fn b(_: crate::scalar::RawScalarBindInfo) {}
    unsafe extern "C" fn i(_: crate::scalar::RawScalarInitInfo) {}

    let ob = ScalarOverloadBuilder::new();
    assert!(ob.bind.is_none() && ob.init.is_none());
    let ob = ob.bind(b).init(i);
    assert!(ob.bind.is_some() && ob.init.is_some());
}

/// Every overload is checked for a return type and a callback before any
/// `DuckDB` call, and the error names the overload. A null connection proves
/// it: reaching `DuckDB` at all would panic (no dispatch table) or crash.
#[test]
fn an_incomplete_overload_is_reported_by_index_before_touching_duckdb() {
    unsafe extern "C" fn f(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

    let missing_function = ScalarFunctionSetBuilder::new("s")
        .overload(
            ScalarOverloadBuilder::new()
                .returns(TypeId::BigInt)
                .function(f),
        )
        .overload(
            ScalarOverloadBuilder::new()
                .param(TypeId::Double)
                .returns(TypeId::BigInt),
        );
    // SAFETY: validation fails before `con` is used.
    let err = unsafe { missing_function.register(std::ptr::null_mut()) }
        .expect_err("overload 1 has no callback");
    assert!(err.as_str().contains("overload 1"), "{err}");
    assert!(err.as_str().contains("function"), "{err}");

    let missing_return = ScalarFunctionSetBuilder::new("s").overload(
        ScalarOverloadBuilder::new()
            .param(TypeId::Double)
            .function(f),
    );
    // SAFETY: as above.
    let err = unsafe { missing_return.register(std::ptr::null_mut()) }
        .expect_err("overload 0 has no return type");
    assert!(err.as_str().contains("overload 0"), "{err}");
    assert!(err.as_str().contains("return type"), "{err}");

    // SAFETY: as above.
    let err = unsafe { ScalarFunctionSetBuilder::new("s").register(std::ptr::null_mut()) }
        .expect_err("no overloads");
    assert!(err.as_str().contains("no overloads"), "{err}");
}

/// An overload's `extra_info` is freed exactly once when validation rejects
/// the set, whichever overload failed.
#[test]
fn a_rejected_set_frees_every_overloads_extra_info_once() {
    use std::os::raw::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static FREED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn free_it(p: *mut c_void) {
        // SAFETY: allocated below by `Box::into_raw`.
        drop(unsafe { Box::from_raw(p.cast::<u8>()) });
        FREED.fetch_add(1, Ordering::SeqCst);
    }
    unsafe extern "C" fn f(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
    let data = || Box::into_raw(Box::new(7_u8)).cast::<c_void>();

    // SAFETY: `free_it` frees exactly what `data` allocates.
    let set = unsafe {
        ScalarFunctionSetBuilder::new("s")
            .overload(
                ScalarOverloadBuilder::new()
                    .returns(TypeId::BigInt)
                    .function(f)
                    .extra_info(data(), Some(free_it)),
            )
            .overload(ScalarOverloadBuilder::new().extra_info(data(), Some(free_it)))
    };
    // SAFETY: validation fails before `con` is used.
    assert!(unsafe { set.register(std::ptr::null_mut()) }.is_err());
    assert_eq!(FREED.load(Ordering::SeqCst), 2);
}
