// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! What a bind callback learns about its arguments from `Expression`.

use std::sync::Mutex;

use quack_rs::scalar::{ScalarBindInfo, ScalarFunctionBuilder};
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::{inspects_arguments_or_refuses, Fixture};

/// Per argument: `is_null`, the return type's id, `is_foldable`, and the
/// folded value as a BIGINT.
type Seen = (bool, Option<TypeId>, bool, Option<i64>);

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn record_bind(info: libduckdb_sys::duckdb_bind_info) {
    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    // SAFETY: inside a bind callback, so the context is live.
    let ctx = unsafe { bind.get_client_context() };
    let mut seen = SEEN.lock().expect("lock");
    for index in 0..bind.argument_count() {
        // SAFETY: `index` is below the argument count.
        let expr = unsafe { bind.argument(index) }.expect("argument");
        let type_id = expr
            .return_type()
            // SAFETY: a live logical type DuckDB just returned.
            .and_then(|t| unsafe { t.try_get_type_id() });
        let folded = expr
            .is_foldable()
            .then(|| expr.fold(&ctx).ok().and_then(|v| v.as_i64()))
            .flatten();
        seen.push((expr.is_null(), type_id, expr.is_foldable(), folded));
    }
}

quack_rs::scalar_callback!(first_arg, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row)) };
    }
});

#[test]
fn a_bind_callback_sees_each_arguments_type_and_foldability() {
    let fx = Fixture::open();
    // SAFETY: the fixture's connection is open; the callbacks match.
    unsafe {
        ScalarFunctionBuilder::try_new("first_arg")
            .expect("name")
            .param(TypeId::BigInt)
            .param(TypeId::Varchar)
            .returns(TypeId::BigInt)
            .bind(record_bind)
            .function(first_arg)
            .register(fx.con())
            .expect("register first_arg");
    }
    if !inspects_arguments_or_refuses(&fx, "SELECT first_arg(1, 'x')") {
        return;
    }
    SEEN.lock().expect("lock").clear();
    let got = fx.scalar(
        "SELECT max(first_arg(i, 'x')) FROM range(3) t(i)",
        |r, i| unsafe { r.read_i64(i) },
    );
    assert_eq!(got, Some(2));
    let seen = SEEN.lock().expect("lock").clone();
    assert_eq!(
        seen.first().copied(),
        Some((false, Some(TypeId::BigInt), false, None)),
        "a column argument: {seen:?}"
    );
    assert_eq!(
        seen.get(1).copied(),
        Some((false, Some(TypeId::Varchar), true, None)),
        "a VARCHAR literal folds, but not to a BIGINT: {seen:?}"
    );
    SEEN.lock().expect("lock").clear();
    let got = fx.scalar("SELECT first_arg(40 + 2, 'x')", |r, i| unsafe {
        r.read_i64(i)
    });
    assert_eq!(got, Some(42));
    let seen = SEEN.lock().expect("lock").clone();
    // The argument as written, before the cast to the BIGINT parameter.
    assert_eq!(
        seen.first().copied(),
        Some((false, Some(TypeId::Integer), true, Some(42))),
        "a constant expression folds: {seen:?}"
    );
}
