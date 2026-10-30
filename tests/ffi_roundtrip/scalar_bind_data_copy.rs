// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ScalarBindInfo::set_bind_data_copy` and `ScalarBindInfo::get_argument`.
//!
//! First: when `DuckDB` copies a bound scalar
//! function (the planner does, to rewrite filters), the copy callback decides
//! what the copy's bind data is.
//!
//! Requires `duckdb-1-5` (declared on the `mod` line).

use std::os::raw::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{
    RawScalarBindInfo, ScalarBindData, ScalarBindInfo, ScalarFunctionBuilder, ScalarFunctionInfo,
};
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::Fixture;

static BINDS: AtomicUsize = AtomicUsize::new(0);
static COPIES: AtomicUsize = AtomicUsize::new(0);
static DROPS: AtomicUsize = AtomicUsize::new(0);

struct Data {
    factor: i64,
}

unsafe extern "C" fn destroy(ptr: *mut c_void) {
    if !ptr.is_null() {
        DROPS.fetch_add(1, Ordering::SeqCst);
        // SAFETY: allocated by `Box::into_raw` in `bind` or `copy`.
        drop(unsafe { Box::from_raw(ptr.cast::<Data>()) });
    }
}

unsafe extern "C" fn copy(ptr: *mut c_void) -> *mut c_void {
    COPIES.fetch_add(1, Ordering::SeqCst);
    // SAFETY: DuckDB passes the pointer `bind` stored, still live.
    let factor = unsafe { &*ptr.cast::<Data>() }.factor;
    Box::into_raw(Box::new(Data { factor })).cast()
}

unsafe extern "C" fn bind(info: RawScalarBindInfo) {
    BINDS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    // SAFETY: a fresh Box, freed once by `destroy`; `copy` makes its own.
    unsafe {
        bind.set_bind_data(
            Box::into_raw(Box::new(Data { factor: 3 })).cast(),
            Some(destroy),
        );
        bind.set_bind_data_copy(Some(copy));
    }
}

quack_rs::scalar_callback!(times_factor, |info, input, output| {
    // SAFETY: the bind data was set by `bind`, or made by `copy` from it.
    let factor = unsafe {
        ScalarFunctionInfo::new(info)
            .get_bind_data()
            .cast::<Data>()
            .as_ref()
            .expect("bind data (a copy callback that returned null?)")
            .factor
    };
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row) * factor) };
    }
});

fn counts() -> (usize, usize, usize) {
    (
        BINDS.load(Ordering::SeqCst),
        COPIES.load(Ordering::SeqCst),
        DROPS.load(Ordering::SeqCst),
    )
}

#[test]
fn a_copied_function_gets_the_bind_data_the_copy_callback_made() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        ScalarFunctionBuilder::try_new("times_three")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .bind(bind)
            .function(times_factor)
            .register(fx.con())
    }
    .expect("register times_three");

    // A plain projection answers with the factor `bind` stored. (A release
    // DuckDB copies nothing here; one built with assertions copies every
    // planned expression to verify it, so the number of copies is not asserted.)
    assert_eq!(
        fx.scalar(
            "SELECT sum(times_three(i))::BIGINT FROM range(5) t(i)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(30)
    );

    // Filters over the function's result are pushed down, which copies the
    // bound expression. Every copy must carry the factor over: 3 * i for
    // i in 2..=33 is what lies in (3, 100).
    let before = counts();
    let sql = "SELECT sum(v)::BIGINT, count(*) FROM \
               (SELECT times_three(i) AS v FROM range(50) t(i)) WHERE v > 3 AND v < 100";
    let mut result = fx.query(sql);
    let chunk = result.next_chunk().expect("fetch").expect("one row");
    // SAFETY: (BIGINT, BIGINT), one row.
    let (sum, count) = unsafe { (chunk.reader(0).read_i64(0), chunk.reader(1).read_i64(0)) };
    assert_eq!((sum, count), (3 * (2..=33).sum::<i64>(), 32), "{sql}");
    drop(chunk);
    drop(result);
    assert!(
        COPIES.load(Ordering::SeqCst) > before.1,
        "the filter pushdown copies the bound function: {sql}"
    );

    // Every allocation, the bound ones and the copies, is freed exactly once
    // when the database closes.
    drop(fx);
    let (binds, copies, drops) = counts();
    assert_eq!(drops, binds + copies, "{binds} binds + {copies} copies");
}

// ─── The raw argument handle ─────────────────────────────────────────────────

/// The return type of each argument `get_argument` handed out, by position.
static ARGUMENT_TYPES: std::sync::Mutex<Vec<Option<TypeId>>> = std::sync::Mutex::new(Vec::new());

unsafe extern "C" fn record_raw_arguments(info: RawScalarBindInfo) {
    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    for index in 0..bind.argument_count() {
        // SAFETY: `index` is below the argument count, and this runs on an
        // engine that copies arguments safely (the test checks first).
        let mut expr = unsafe { bind.get_argument(index) };
        assert!(!expr.is_null(), "argument {index}");
        // SAFETY: `expr` is a live expression handle, owned here; the return
        // type is a fresh handle `LogicalType` takes over.
        let ty = unsafe {
            quack_rs::types::LogicalType::from_raw(libduckdb_sys::duckdb_expression_return_type(
                expr,
            ))
        };
        // SAFETY: `ty` is a valid logical type.
        let id = unsafe { ty.try_get_type_id() };
        ARGUMENT_TYPES.lock().expect("lock").push(id);
        // SAFETY: the handle is the caller's to destroy, once.
        unsafe { libduckdb_sys::duckdb_destroy_expression(&raw mut expr) };
    }
}

quack_rs::scalar_callback!(first_of_three, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row)) };
    }
});

#[test]
fn get_argument_hands_out_each_argument_as_an_owned_expression() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        ScalarFunctionBuilder::try_new("raw_args")
            .expect("name")
            .param(TypeId::BigInt)
            .param(TypeId::Varchar)
            .param(TypeId::Double)
            .returns(TypeId::BigInt)
            .bind(record_raw_arguments)
            .function(first_of_three)
            .register(fx.con())
    }
    .expect("register raw_args");
    // The callback calls the raw handle, which has no guard to refuse on the
    // engines that abort on a subquery argument: it only runs on the others.
    if !super::engine_copies_arguments_safely(&fx) {
        return;
    }
    ARGUMENT_TYPES.lock().expect("lock").clear();
    assert_eq!(
        fx.scalar(
            "SELECT raw_args(i, 'x', 1.5::DOUBLE) FROM range(1) t(i)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(0)
    );
    let seen = ARGUMENT_TYPES.lock().expect("lock").clone();
    // The first argument is a column of BIGINT (range's type) on every
    // engine; the other two are as written, or cast to the declared type. Each
    // bind sees the three in order; a DuckDB built with assertions plans the
    // call more than once (`Planner::Optimize` verifies the plan under
    // `#ifdef DEBUG`), so the triple can repeat.
    let expected = [
        Some(TypeId::BigInt),
        Some(TypeId::Varchar),
        Some(TypeId::Double),
    ];
    assert!(
        !seen.is_empty() && seen.chunks(3).all(|bind| bind == expected),
        "{seen:?}"
    );
}

// ─── A typed bind data whose `Clone` panics (FFI-6) ──────────────────────────

/// Typed bind data whose copy always fails.
struct CloneBomb(i64);

impl Clone for CloneBomb {
    fn clone(&self) -> Self {
        panic!("CloneBomb::clone deliberately exploded (factor {})", self.0)
    }
}

unsafe extern "C" fn bomb_bind(info: RawScalarBindInfo) {
    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    ScalarBindData::set(&bind, CloneBomb(3));
}

// `i * factor`, with `get`: a copy without its bind data would answer 0.
quack_rs::scalar_callback!(bomb_get, |info, input, output| {
    // SAFETY: DuckDB passes a valid function info.
    let fninfo = unsafe { ScalarFunctionInfo::new(info) };
    // SAFETY: `bomb_bind` stored a `CloneBomb`, and nothing else did.
    let factor = unsafe { ScalarBindData::<CloneBomb>::get(&fninfo) }.map_or(0, |b| b.0);
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row) * factor) };
    }
});

// The same with `try_get`, which hands the failure to the callback.
quack_rs::scalar_callback!(bomb_try_get, |info, input, output| {
    // SAFETY: DuckDB passes a valid function info.
    let fninfo = unsafe { ScalarFunctionInfo::new(info) };
    // SAFETY: as above.
    let factor = match unsafe { ScalarBindData::<CloneBomb>::try_get(&fninfo) } {
        Ok(found) => found.map_or(0, |b| b.0),
        Err(e) => {
            fninfo.set_error(&format!("bomb_try_get: {e}"));
            return;
        }
    };
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row) * factor) };
    }
});

/// The reviewer's probe for FFI-6: filters over the function's result are
/// pushed down, which copies the bound call, and `CloneBomb::clone` panics.
/// The copy used to get no bind data, so the query answered `(NULL, 0)`
/// instead of `(4752, 32)` and the panic message was lost. The query now
/// fails with that message, through `get` and through `try_get`.
#[test]
fn a_panicking_bind_data_clone_fails_the_query_with_its_message() {
    let fx = Fixture::open();
    for (name, callback) in [
        ("bomb_get", bomb_get as quack_rs::scalar::ScalarFn),
        ("bomb_try_get", bomb_try_get),
    ] {
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe {
            ScalarFunctionBuilder::try_new(name)
                .expect("name")
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .bind(bomb_bind)
                .function(callback)
                .register(fx.con())
        }
        .unwrap_or_else(|e| panic!("register {name}: {e}"));
        let sql = format!(
            "SELECT sum(v)::BIGINT, count(*) FROM \
             (SELECT {name}(i) AS v FROM range(50) t(i)) WHERE v > 3 AND v < 100"
        );
        // SAFETY: the fixture's connection is open.
        let answer = unsafe { quack_rs::query::query(fx.con(), &sql) };
        let err = match answer {
            Err(e) => e.to_string(),
            Ok(mut result) => {
                let chunk = result.next_chunk().expect("fetch").expect("one row");
                // SAFETY: (BIGINT, BIGINT), one row; the sum may be NULL.
                let (sum, count) = unsafe {
                    let sum = chunk.reader(0);
                    (
                        sum.is_valid(0).then(|| sum.read_i64(0)),
                        chunk.reader(1).read_i64(0),
                    )
                };
                panic!("{name}: answered ({sum:?}, {count}) instead of failing")
            }
        };
        assert!(
            err.contains("CloneBomb::clone deliberately exploded (factor 3)")
                && err.contains("Clone panicked"),
            "{name}: {err}"
        );
    }
}

// ─── `ScalarBindData::set` twice in one bind (FFI-12) ────────────────────────

static TRACKED_MADE: AtomicUsize = AtomicUsize::new(0);
static TRACKED_DROPPED: AtomicUsize = AtomicUsize::new(0);

/// Bind data that counts every value made (clones included) and dropped.
struct Tracked(i64);

impl Tracked {
    fn new(factor: i64) -> Self {
        TRACKED_MADE.fetch_add(1, Ordering::SeqCst);
        Self(factor)
    }
}

impl Clone for Tracked {
    fn clone(&self) -> Self {
        Self::new(self.0)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        TRACKED_DROPPED.fetch_add(1, Ordering::SeqCst);
    }
}

unsafe extern "C" fn set_twice_bind(info: RawScalarBindInfo) {
    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    ScalarBindData::set(&bind, Tracked::new(1));
    ScalarBindData::set(&bind, Tracked::new(2));
}

quack_rs::scalar_callback!(set_twice_exec, |info, input, output| {
    // SAFETY: DuckDB passes a valid function info.
    let fninfo = unsafe { ScalarFunctionInfo::new(info) };
    // SAFETY: `set_twice_bind` stored `Tracked`s, and nothing else did.
    let factor = unsafe { ScalarBindData::<Tracked>::get(&fninfo) }.map_or(0, |t| t.0);
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row) * factor) };
    }
});

/// A second `set` in one bind replaces the first value, which is dropped:
/// `DuckDB` overwrites the pointer without running its destructor, so before
/// the fix one value per bind was leaked (reviewer `ffi`: 2 made, 1 dropped).
/// Every value made, copies included, is dropped once the database closes.
#[test]
fn setting_typed_bind_data_twice_drops_the_first_value() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        ScalarFunctionBuilder::try_new("set_twice")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .bind(set_twice_bind)
            .function(set_twice_exec)
            .register(fx.con())
    }
    .expect("register set_twice");
    // The second value is the one in effect, also in the pushed-down copies.
    assert_eq!(
        fx.scalar(
            "SELECT sum(set_twice(i))::BIGINT FROM range(5) t(i)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(20)
    );
    let sql = "SELECT sum(v)::BIGINT FROM \
               (SELECT set_twice(i) AS v FROM range(50) t(i)) WHERE v > 3 AND v < 100";
    assert_eq!(
        fx.scalar(sql, |r, i| unsafe { r.read_i64(i) }),
        Some(2 * (2..=49).sum::<i64>())
    );
    drop(fx);
    let (made, dropped) = (
        TRACKED_MADE.load(Ordering::SeqCst),
        TRACKED_DROPPED.load(Ordering::SeqCst),
    );
    assert!(made >= 4, "two binds, two values each: {made}");
    assert_eq!(dropped, made, "every value made is dropped");
}
