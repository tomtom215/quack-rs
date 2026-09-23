// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Function lifecycle contracts pinned against the linked `DuckDB`: which rows
//! an aggregate's `update` receives, and how the planner treats a registered
//! function's null handling.

use super::Fixture;
use quack_rs::aggregate::callbacks::FinalizeFn;
use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};
use quack_rs::data_chunk::DataChunk;
use quack_rs::types::{NullHandling, TypeId};
use quack_rs::vector::VectorWriter;

// ─── An aggregate that records what `update` was handed ─────────────────────

/// Counts every row `update` receives, and separately the valid ones.
#[derive(Default)]
struct RowCounts {
    rows: i64,
    valid: i64,
}
impl AggregateState for RowCounts {}

quack_rs::aggregate_update_callback!(row_counts_update, |_info, input, states| {
    // SAFETY: DuckDB passes a valid one-column BIGINT chunk and one state per row.
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if let Some(s) = unsafe { FfiState::<RowCounts>::with_state_mut(*states.add(row)) } {
            s.rows += 1;
            if unsafe { reader.is_valid(row) } {
                s.valid += 1;
            }
        }
    }
});

quack_rs::aggregate_combine_callback!(row_counts_combine, |_info, source, target, count| {
    for i in 0..count as usize {
        // SAFETY: DuckDB passes `count` valid state pointers in each array.
        let Some((rows, valid)) = (unsafe { FfiState::<RowCounts>::with_state(*source.add(i)) })
            .map(|s| (s.rows, s.valid))
        else {
            continue;
        };
        if let Some(t) = unsafe { FfiState::<RowCounts>::with_state_mut(*target.add(i)) } {
            t.rows += rows;
            t.valid += valid;
        }
    }
});

// Finalizes to the number of rows `update` saw, NULL rows included.
quack_rs::aggregate_finalize_callback!(
    rows_seen_finalize,
    |_info, source, result, count, offset| {
        // SAFETY: `result` is the BIGINT output vector for this batch.
        let mut writer = unsafe { VectorWriter::from_vector(result) };
        for i in 0..count as usize {
            let row = offset as usize + i;
            match unsafe { FfiState::<RowCounts>::with_state(*source.add(i)) } {
                Some(s) => unsafe { writer.write_i64(row, s.rows) },
                None => unsafe { writer.set_null(row) },
            }
        }
    }
);

// Finalizes to the number of valid rows: a `count(x)` that returns 0, not
// NULL, for empty input.
quack_rs::aggregate_finalize_callback!(valid_finalize, |_info, source, result, count, offset| {
    // SAFETY: `result` is the BIGINT output vector for this batch.
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        let row = offset as usize + i;
        match unsafe { FfiState::<RowCounts>::with_state(*source.add(i)) } {
            Some(s) => unsafe { writer.write_i64(row, s.valid) },
            None => unsafe { writer.set_null(row) },
        }
    }
});

fn register_row_counter(fx: &Fixture, name: &str, handling: NullHandling, finalize: FinalizeFn) {
    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        AggregateFunctionBuilder::try_new(name)
            .expect("valid name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<RowCounts>::size_callback)
            .init(FfiState::<RowCounts>::init_callback)
            .update(row_counts_update)
            .combine(row_counts_combine)
            .finalize(finalize)
            .destructor(FfiState::<RowCounts>::destroy_callback)
            .null_handling(handling)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

fn i64_at(fx: &Fixture, sql: &str) -> Option<i64> {
    // SAFETY: the query returns BIGINT in column 0; `scalar` checks validity.
    fx.scalar(sql, |r, i| unsafe { r.read_i64(i) })
}

/// Every row of the chunk reaches `update`, NULL rows included, under
/// **either** null handling: `CAPIAggregateUpdate` flattens the inputs and
/// passes the whole chunk, and nothing filters by validity first.
///
/// quack-rs documented the opposite for `DefaultNullHandling` until the third
/// audit. If a future `DuckDB` starts filtering, this fails and the
/// `NullHandling` documentation (and pitfall L12) must be revisited.
#[test]
fn aggregate_update_receives_null_rows_under_either_null_handling() {
    let fx = Fixture::open();
    register_row_counter(
        &fx,
        "rows_seen_default",
        NullHandling::DefaultNullHandling,
        rows_seen_finalize,
    );
    register_row_counter(
        &fx,
        "rows_seen_special",
        NullHandling::SpecialNullHandling,
        rows_seen_finalize,
    );
    fx.query(
        "CREATE TABLE agg_nulls AS SELECT i::BIGINT AS g, \
         CASE WHEN i % 3 = 0 THEN i::BIGINT END AS x FROM range(9) t(i)",
    );
    // Sanity: 9 rows, of which 3 are non-NULL.
    assert_eq!(i64_at(&fx, "SELECT count(x) FROM agg_nulls"), Some(3));

    for name in ["rows_seen_default", "rows_seen_special"] {
        assert_eq!(
            i64_at(&fx, &format!("SELECT {name}(x) FROM agg_nulls")),
            Some(9),
            "{name}: update must have been handed all 9 rows, 6 of them NULL"
        );
        // The same through the hash aggregate: 3 groups of 3 rows, 1 non-NULL each.
        assert_eq!(
            i64_at(
                &fx,
                &format!(
                    "SELECT sum(c)::BIGINT FROM \
                     (SELECT {name}(x) AS c FROM agg_nulls GROUP BY g % 3)"
                )
            ),
            Some(9),
            "{name}: grouped update must have been handed all 9 rows"
        );
    }
}

/// What the aggregate null-handling setting does *not* fix: in a correlated
/// subquery, an outer row with no matching inner rows gets NULL — the
/// decorrelated plan never finalizes an empty state for it — and `DuckDB`
/// rewrites that NULL to 0 only for its own `count` / `count(*)`
/// (`flatten_dependent_join.cpp`). A count-like aggregate therefore answers
/// NULL there under both settings.
///
/// `SpecialNullHandling` is read for an aggregate only by that decorrelator
/// (`BoundAggregateExpression::PropagatesNullValues`), to choose an INNER or
/// LEFT join; none of these shapes answers differently under the two
/// settings. This pins both facts for the `NullHandling` documentation.
#[test]
fn a_count_like_aggregate_in_a_correlated_subquery_is_null_for_an_unmatched_row() {
    let fx = Fixture::open();
    register_row_counter(
        &fx,
        "valid_default",
        NullHandling::DefaultNullHandling,
        valid_finalize,
    );
    register_row_counter(
        &fx,
        "valid_special",
        NullHandling::SpecialNullHandling,
        valid_finalize,
    );
    fx.query("CREATE TABLE outer_t AS SELECT * FROM (VALUES (1::BIGINT), (2), (3)) v(k)");
    fx.query(
        "CREATE TABLE inner_t AS SELECT * FROM \
         (VALUES (1::BIGINT, 10::BIGINT), (1, NULL), (2, NULL)) v(k, x)",
    );

    // Per outer key k = 1, 2, 3: k = 2 matches only a NULL, k = 3 matches nothing.
    let per_key = |select: &str| -> Vec<Option<i64>> {
        (1..=3)
            .map(|k| {
                i64_at(
                    &fx,
                    &format!(
                        "SELECT {} FROM outer_t WHERE k = {k}",
                        select.replace('$', "outer_t")
                    ),
                )
            })
            .collect()
    };

    for name in ["valid_default", "valid_special"] {
        // Uncorrelated, an empty input finalizes a fresh state: 0.
        assert_eq!(
            i64_at(&fx, &format!("SELECT {name}(x) FROM inner_t WHERE k = 99")),
            Some(0),
            "{name}"
        );
        // Correlated: the unmatched key 3 is NULL, not 0.
        assert_eq!(
            per_key(&format!(
                "(SELECT {name}(x) FROM inner_t WHERE inner_t.k = $.k)"
            )),
            vec![Some(1), Some(0), None],
            "{name}"
        );
        // Arithmetic over the aggregate inside the subquery changes nothing.
        assert_eq!(
            per_key(&format!(
                "(SELECT {name}(x) + 100 FROM inner_t WHERE inner_t.k = $.k)"
            )),
            vec![Some(101), Some(100), None],
            "{name}"
        );
        // `coalesce` inside the subquery sees the NULL, not a finalized 0.
        assert_eq!(
            per_key(&format!(
                "(SELECT coalesce({name}(x), -7) FROM inner_t WHERE inner_t.k = $.k)"
            )),
            vec![Some(1), Some(0), Some(-7)],
            "{name}"
        );
        // The documented mitigation.
        assert_eq!(
            per_key(&format!(
                "coalesce((SELECT {name}(x) FROM inner_t WHERE inner_t.k = $.k), 0)"
            )),
            vec![Some(1), Some(0), Some(0)],
            "{name}"
        );
    }

    // DuckDB's own `count` is special-cased to 0.
    assert_eq!(
        per_key("(SELECT count(x) FROM inner_t WHERE inner_t.k = $.k)"),
        vec![Some(1), Some(0), Some(0)]
    );
}

// ─── Typed scalar construction ──────────────────────────────────────────────

/// A deliberately wrong out-of-crate `ScalarValue` naming a composite type.
#[derive(Clone, Copy)]
struct NotReallyDecimal(i64);

// SAFETY: never read — registration must reject the signature before any
// chunk reaches the closure. That rejection is what the test checks.
unsafe impl quack_rs::scalar::ScalarValue for NotReallyDecimal {
    fn type_id() -> TypeId {
        TypeId::Decimal
    }
    unsafe fn read(reader: &quack_rs::vector::VectorReader, row: usize) -> Self {
        // SAFETY: forwarded; unreachable in this test.
        Self(unsafe { reader.read_i64(row) })
    }
}

/// `map1` no longer validates slot types at build time (that needed a live
/// `DuckDB` for no benefit); `register` still rejects a composite `TypeId`
/// before allocating a `DuckDB` handle, naming the slot.
#[test]
fn a_composite_scalar_value_is_rejected_at_registration_naming_the_slot() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();
    let builder = ScalarFunctionBuilder::map1("not_really_decimal", |d: NotReallyDecimal| d.0)
        .expect("building makes no DuckDB call and cannot see the type");
    // SAFETY: `con` is open.
    let err = unsafe { builder.register(fx.con()) }.expect_err("DECIMAL needs width and scale");
    let msg = err.as_str();
    assert!(msg.contains("scalar function parameter 0"), "{msg}");
    assert!(msg.contains("DECIMAL"), "{msg}");
    // Nothing was registered.
    assert_eq!(
        i64_at(
            &fx,
            "SELECT count(*) FROM duckdb_functions() WHERE function_name = 'not_really_decimal'"
        ),
        Some(0)
    );
}

// ─── Name collisions ────────────────────────────────────────────────────────

/// The collision rules documented on `ScalarFunctionBuilder::register` and
/// `AggregateFunctionBuilder::register`. `DuckDB` registers both with
/// `ALTER_ON_CONFLICT`; a scalar merges into an existing scalar entry with
/// `override = true`, and an aggregate cannot be altered at all.
#[test]
fn scalar_collisions_replace_silently_and_aggregate_collisions_fail() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();
    // SAFETY (all `register` calls below): `con` is open.
    let register_scalar = |name: &str, add: i64| unsafe {
        ScalarFunctionBuilder::map1(name, move |x: i64| x + add)
            .expect("valid name")
            .register(fx.con())
    };

    // Same name and signature twice: no error, and the second one wins.
    register_scalar("twice_scalar", 1).expect("first registration");
    register_scalar("twice_scalar", 100).expect("an identical signature is not an error");
    assert_eq!(i64_at(&fx, "SELECT twice_scalar(1::BIGINT)"), Some(101));

    // The same for a built-in: `abs(BIGINT) -> BIGINT` is replaced.
    register_scalar("abs", 1000).expect("replacing a built-in is not an error");
    assert_eq!(i64_at(&fx, "SELECT abs(-5::BIGINT)"), Some(995));

    // A scalar cannot take an aggregate's name.
    assert!(register_scalar("sum", 1).is_err(), "sum is an aggregate");

    // An aggregate cannot be registered twice, nor over a scalar's name.
    register_row_counter(
        &fx,
        "twice_agg",
        NullHandling::DefaultNullHandling,
        valid_finalize,
    );
    let again = |name: &str| unsafe {
        AggregateFunctionBuilder::try_new(name)
            .expect("valid name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<RowCounts>::size_callback)
            .init(FfiState::<RowCounts>::init_callback)
            .update(row_counts_update)
            .combine(row_counts_combine)
            .finalize(valid_finalize)
            .destructor(FfiState::<RowCounts>::destroy_callback)
            .register(fx.con())
    };
    assert!(
        again("twice_agg").is_err(),
        "an aggregate name is taken once registered"
    );
    assert!(again("upper").is_err(), "upper is a scalar");
    // The first registration is untouched.
    assert_eq!(
        i64_at(&fx, "SELECT twice_agg(i::BIGINT) FROM range(4) t(i)"),
        Some(4)
    );
}

// ─── Overload builders mirror the single-function builders ──────────────────

mod overload_fns {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use quack_rs::aggregate::{AggregateFunctionInfo, FfiState};
    use quack_rs::data_chunk::DataChunk;
    use quack_rs::vector::VectorWriter;

    use super::RowCounts;

    /// A per-row counter, so re-evaluation is observable.
    pub static TICKS: AtomicI64 = AtomicI64::new(0);

    quack_rs::scalar_callback!(tick, |_info, input, output| {
        // SAFETY: DuckDB passes a valid chunk and a BIGINT output vector.
        let chunk = unsafe { DataChunk::from_raw(input) };
        let mut writer = unsafe { VectorWriter::from_vector(output) };
        for row in 0..chunk.size() {
            unsafe { writer.write_i64(row, TICKS.fetch_add(1, Ordering::SeqCst)) };
        }
    });

    // Sums every BIGINT column of the chunk: the fixed parameter and varargs.
    quack_rs::scalar_callback!(sum_columns, |_info, input, output| {
        // SAFETY: DuckDB passes a valid chunk of BIGINT columns.
        let chunk = unsafe { DataChunk::from_raw(input) };
        let mut writer = unsafe { VectorWriter::from_vector(output) };
        for row in 0..chunk.size() {
            let total = (0..chunk.column_count())
                .map(|col| unsafe { chunk.reader(col).read_i64(row) })
                .sum::<i64>();
            unsafe { writer.write_i64(row, total) };
        }
    });

    #[cfg(feature = "duckdb-1-5")]
    pub mod bound {
        use quack_rs::data_chunk::DataChunk;
        use quack_rs::scalar::{
            ScalarBindData, ScalarBindInfo, ScalarFunctionInfo, ScalarInitInfo, ScalarLocalState,
        };
        use quack_rs::vector::VectorWriter;

        /// Folds the constant second argument into bind data.
        pub unsafe extern "C" fn bind(info: libduckdb_sys::duckdb_bind_info) {
            // SAFETY: DuckDB passes a valid bind info.
            let bind = unsafe { ScalarBindInfo::new(info) };
            // SAFETY: the overload declares two parameters.
            let factor = unsafe { bind.argument(1) }
                .filter(quack_rs::expression::Expression::is_foldable)
                .and_then(|expr| {
                    // SAFETY: inside a bind callback, so the context is live.
                    let ctx = unsafe { bind.get_client_context() };
                    expr.fold(&ctx).ok().and_then(|v| v.as_i64())
                })
                .unwrap_or(1);
            ScalarBindData::set(&bind, factor);
        }

        /// Counts chunks per thread.
        pub unsafe extern "C" fn init(info: libduckdb_sys::duckdb_init_info) {
            // SAFETY: DuckDB passes a valid init info.
            let init = unsafe { ScalarInitInfo::new(info) };
            ScalarLocalState::set(&init, 0_u64);
        }

        quack_rs::scalar_callback!(scale, |info, input, output| {
            // SAFETY: DuckDB passes a valid function info; `bind` stored an
            // i64 and `init` a u64, and nothing else stored either.
            let fninfo = unsafe { ScalarFunctionInfo::new(info) };
            let factor = unsafe { ScalarBindData::<i64>::get(&fninfo) }.copied();
            let chunks = unsafe { ScalarLocalState::<u64>::get_mut(&fninfo) };
            let (Some(factor), Some(chunks)) = (factor, chunks) else {
                fninfo.set_error("bind data or local state missing");
                return;
            };
            *chunks += 1;
            // SAFETY: argument 0 is BIGINT and the output is BIGINT.
            let chunk = unsafe { DataChunk::from_raw(input) };
            let reader = unsafe { chunk.reader(0) };
            let mut writer = unsafe { VectorWriter::from_vector(output) };
            for row in 0..chunk.size() {
                unsafe { writer.write_i64(row, reader.read_i64(row) * factor) };
            }
        });
    }

    /// Frees counted by the aggregate overload's `extra_info` destructor.
    pub static AGG_INFO_FREES: AtomicUsize = AtomicUsize::new(0);

    pub unsafe extern "C" fn free_offset(ptr: *mut std::os::raw::c_void) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: allocated by `Box::into_raw(Box::new(i64))` in the test.
        drop(unsafe { Box::from_raw(ptr.cast::<i64>()) });
        AGG_INFO_FREES.fetch_add(1, Ordering::SeqCst);
    }

    // Finalizes to `valid + *extra_info`.
    quack_rs::aggregate_finalize_callback!(
        offset_finalize,
        |info, source, result, count, offset| {
            // SAFETY: DuckDB passes a valid function info whose extra_info is the
            // boxed i64 the test attached to this overload.
            let offset_by = unsafe {
                let raw = AggregateFunctionInfo::new(info).get_extra_info();
                (!raw.is_null()).then(|| *raw.cast::<i64>())
            };
            let mut writer = unsafe { VectorWriter::from_vector(result) };
            for i in 0..count as usize {
                let row = offset as usize + i;
                let state = unsafe { FfiState::<RowCounts>::with_state(*source.add(i)) };
                match (state, offset_by) {
                    (Some(s), Some(by)) => unsafe { writer.write_i64(row, s.valid + by) },
                    _ => unsafe { writer.set_null(row) },
                }
            }
        }
    );
}

/// `ScalarOverloadBuilder::volatile` and `varargs` reach `DuckDB` per overload.
#[test]
fn scalar_overloads_carry_their_own_volatility_and_varargs() {
    use quack_rs::scalar::{ScalarFunctionSetBuilder, ScalarOverloadBuilder};

    let fx = Fixture::open();
    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        ScalarFunctionSetBuilder::try_new("tick_set")
            .expect("valid name")
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .returns(TypeId::BigInt)
                    .function(overload_fns::tick)
                    .volatile(),
            )
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Varchar)
                    .returns(TypeId::BigInt)
                    .function(overload_fns::tick),
            )
            .register(fx.con())
            .expect("register tick_set");
        ScalarFunctionSetBuilder::try_new("sum_set")
            .expect("valid name")
            // (BIGINT) and (BIGINT, BIGINT...) differ in DuckDB's eyes, so the
            // duplicate-overload check must not reject them.
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .returns(TypeId::BigInt)
                    .function(overload_fns::tick),
            )
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .varargs(TypeId::BigInt)
                    .returns(TypeId::BigInt)
                    .function(overload_fns::sum_columns),
            )
            .register(fx.con())
            .expect("register sum_set");
    }

    // Volatile overload: re-evaluated per row despite a constant argument.
    assert_eq!(
        i64_at(
            &fx,
            "SELECT count(DISTINCT tick_set(1::BIGINT)) FROM range(50)"
        ),
        Some(50)
    );
    // The other overload is not volatile, so the constant call is folded once.
    assert_eq!(
        i64_at(&fx, "SELECT count(DISTINCT tick_set('a')) FROM range(50)"),
        Some(1)
    );
    // Varargs overload.
    assert_eq!(
        i64_at(&fx, "SELECT sum_set(1::BIGINT, 2::BIGINT, 39::BIGINT)"),
        Some(42)
    );
}

/// `ScalarOverloadBuilder::bind` and `init` reach `DuckDB` per overload.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn scalar_overloads_carry_their_own_bind_and_init() {
    use quack_rs::scalar::{ScalarFunctionSetBuilder, ScalarOverloadBuilder};

    let fx = Fixture::open();
    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        ScalarFunctionSetBuilder::try_new("scale_set")
            .expect("valid name")
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Varchar)
                    .returns(TypeId::BigInt)
                    .function(overload_fns::tick),
            )
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .param(TypeId::BigInt)
                    .returns(TypeId::BigInt)
                    .bind(overload_fns::bound::bind)
                    .init(overload_fns::bound::init)
                    .function(overload_fns::bound::scale),
            )
            .register(fx.con())
            .expect("register scale_set");
    }
    fx.query("CREATE TABLE scale_in AS SELECT i::BIGINT AS i FROM range(10) t(i)");
    assert_eq!(
        i64_at(&fx, "SELECT sum(scale_set(i, 3))::BIGINT FROM scale_in"),
        Some(45 * 3)
    );
}

/// `AggregateOverloadBuilder::extra_info` reaches the overload's callbacks and
/// is freed exactly once — by `DuckDB`, both when the set is dropped with the
/// database and when `DuckDB` refuses the registration.
#[test]
fn an_aggregate_overloads_extra_info_reaches_its_callbacks_and_is_freed_once() {
    use quack_rs::aggregate::{AggregateFunctionSetBuilder, AggregateOverloadBuilder};
    use std::sync::atomic::Ordering;

    let offset_by = |by: i64| Box::into_raw(Box::new(by)).cast::<std::os::raw::c_void>();
    let set = |name: &str, by: i64| {
        let overload = |param: TypeId, by: i64| {
            // SAFETY: `free_offset` frees exactly what `offset_by` allocates.
            unsafe {
                AggregateOverloadBuilder::new()
                    .param(param)
                    .returns(TypeId::BigInt)
                    .state_size(FfiState::<RowCounts>::size_callback)
                    .init(FfiState::<RowCounts>::init_callback)
                    .update(row_counts_update)
                    .combine(row_counts_combine)
                    .finalize(overload_fns::offset_finalize)
                    .destructor(FfiState::<RowCounts>::destroy_callback)
                    .extra_info(offset_by(by), Some(overload_fns::free_offset))
            }
        };
        AggregateFunctionSetBuilder::try_new(name)
            .expect("valid name")
            .overload(overload(TypeId::BigInt, by))
            .overload(overload(TypeId::Varchar, by * 2))
    };

    let before = overload_fns::AGG_INFO_FREES.load(Ordering::SeqCst);
    {
        let fx = Fixture::open();
        // SAFETY: `con` is open.
        unsafe { set("offset_agg", 1000).register(fx.con()) }.expect("register offset_agg");
        assert_eq!(
            i64_at(&fx, "SELECT offset_agg(i::BIGINT) FROM range(5) t(i)"),
            Some(1005)
        );
        assert_eq!(
            i64_at(&fx, "SELECT offset_agg(i::VARCHAR) FROM range(5) t(i)"),
            Some(2005)
        );

        // `sum` is taken: DuckDB refuses the set after both overloads handed
        // their extra_info over, and frees both when the set is destroyed.
        let during = overload_fns::AGG_INFO_FREES.load(Ordering::SeqCst);
        // SAFETY: `con` is open.
        assert!(unsafe { set("sum", 1).register(fx.con()) }.is_err());
        assert_eq!(
            overload_fns::AGG_INFO_FREES.load(Ordering::SeqCst) - during,
            2,
            "a refused set frees each overload's extra_info exactly once"
        );
    }
    // Closing the database frees the registered set's two.
    assert_eq!(
        overload_fns::AGG_INFO_FREES.load(Ordering::SeqCst) - before,
        4
    );
}

// ─── Error messages with an interior NUL ────────────────────────────────────

mod nul_errors {
    use quack_rs::aggregate::AggregateFunctionInfo;
    use quack_rs::scalar::ScalarFunctionInfo;

    quack_rs::scalar_callback!(scalar_fails, |info, _input, _output| {
        // SAFETY: DuckDB passes a valid function info.
        unsafe { ScalarFunctionInfo::new(info) }.set_error("scalar head\0scalar tail");
    });

    quack_rs::aggregate_finalize_callback!(agg_fails, |info, _source, _result, _count, _offset| {
        // SAFETY: DuckDB passes a valid function info.
        unsafe { AggregateFunctionInfo::new(info) }.set_error("agg head\0agg tail");
    });

    #[cfg(feature = "duckdb-1-5")]
    pub unsafe extern "C" fn bind_fails(info: libduckdb_sys::duckdb_bind_info) {
        // SAFETY: DuckDB passes a valid bind info.
        unsafe { quack_rs::scalar::ScalarBindInfo::new(info) }.set_error("bind head\0bind tail");
    }

    #[cfg(feature = "duckdb-1-5")]
    pub unsafe extern "C" fn init_fails(info: libduckdb_sys::duckdb_init_info) {
        // SAFETY: DuckDB passes a valid init info.
        unsafe { quack_rs::scalar::ScalarInitInfo::new(info) }.set_error("init head\0init tail");
    }
}

fn query_error(fx: &Fixture, sql: &str) -> String {
    // SAFETY: `con` is open.
    match unsafe { quack_rs::query::query(fx.con(), sql) } {
        Ok(_) => panic!("{sql} should have failed"),
        Err(e) => e.as_str().to_owned(),
    }
}

/// Every `set_error` wrapper replaces an interior NUL with `?` and keeps the
/// rest of the message, like the callback macros' panic path
/// (`callback::message_to_c_string`). They used to truncate at the NUL,
/// silently dropping everything after it.
#[test]
fn set_error_keeps_the_text_after_an_interior_nul() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();
    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("nul_scalar")
            .expect("valid name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(nul_errors::scalar_fails)
            .register(fx.con())
            .expect("register nul_scalar");
        AggregateFunctionBuilder::try_new("nul_agg")
            .expect("valid name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<RowCounts>::size_callback)
            .init(FfiState::<RowCounts>::init_callback)
            .update(row_counts_update)
            .combine(row_counts_combine)
            .finalize(nul_errors::agg_fails)
            .destructor(FfiState::<RowCounts>::destroy_callback)
            .register(fx.con())
            .expect("register nul_agg");
    }
    let err = query_error(&fx, "SELECT nul_scalar(i::BIGINT) FROM range(3) t(i)");
    assert!(err.contains("scalar head?scalar tail"), "{err}");
    let err = query_error(&fx, "SELECT nul_agg(i::BIGINT) FROM range(3) t(i)");
    assert!(err.contains("agg head?agg tail"), "{err}");

    #[cfg(feature = "duckdb-1-5")]
    {
        // SAFETY: `con` is open; every callback matches its declared signature.
        unsafe {
            ScalarFunctionBuilder::try_new("nul_bind")
                .expect("valid name")
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .bind(nul_errors::bind_fails)
                .function(nul_errors::scalar_fails)
                .register(fx.con())
                .expect("register nul_bind");
            ScalarFunctionBuilder::try_new("nul_init")
                .expect("valid name")
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .init(nul_errors::init_fails)
                .function(nul_errors::scalar_fails)
                .register(fx.con())
                .expect("register nul_init");
        }
        let err = query_error(&fx, "SELECT nul_bind(i::BIGINT) FROM range(3) t(i)");
        assert!(err.contains("bind head?bind tail"), "{err}");
        let err = query_error(&fx, "SELECT nul_init(i::BIGINT) FROM range(3) t(i)");
        assert!(err.contains("init head?init tail"), "{err}");
    }
}

// ─── Scalar bind data and expression equality ───────────────────────────────

#[cfg(feature = "duckdb-1-5")]
mod bind_counter {
    use std::sync::atomic::{AtomicI64, Ordering};

    use quack_rs::data_chunk::DataChunk;
    use quack_rs::scalar::{ScalarBindData, ScalarBindInfo, ScalarFunctionInfo};
    use quack_rs::vector::VectorWriter;

    pub static BINDS: AtomicI64 = AtomicI64::new(0);

    /// Stores a fresh counter value on every bind: deliberately *not* a
    /// function of the arguments.
    pub unsafe extern "C" fn bind(info: libduckdb_sys::duckdb_bind_info) {
        // SAFETY: DuckDB passes a valid bind info.
        let bind = unsafe { ScalarBindInfo::new(info) };
        ScalarBindData::set(&bind, BINDS.fetch_add(1, Ordering::SeqCst));
    }

    // Writes the bind data into every row.
    quack_rs::scalar_callback!(write_bind, |info, input, output| {
        // SAFETY: DuckDB passes a valid function info; `bind` stored an i64.
        let fninfo = unsafe { ScalarFunctionInfo::new(info) };
        let Some(value) = (unsafe { ScalarBindData::<i64>::get(&fninfo) }).copied() else {
            fninfo.set_error("no bind data");
            return;
        };
        // SAFETY: the output is BIGINT.
        let chunk = unsafe { DataChunk::from_raw(input) };
        let mut writer = unsafe { VectorWriter::from_vector(output) };
        for row in 0..chunk.size() {
            unsafe { writer.write_i64(row, value) };
        }
    });
}

/// `CScalarFunctionBindData::Equals` compares only `extra_info` and the
/// callback pointer, so two identical calls are merged even though each was
/// bound separately and stored different bind data. Volatile calls are not
/// merged. Pins the `ScalarBindData` documentation: if `DuckDB` starts
/// comparing bind data, the first assertion fails.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn identical_calls_share_bind_data_unless_the_function_is_volatile() {
    use quack_rs::scalar::ScalarFunctionBuilder;
    use std::sync::atomic::Ordering;

    let fx = Fixture::open();
    for (name, volatile) in [("bind_ctr", false), ("bind_ctr_volatile", true)] {
        let builder = ScalarFunctionBuilder::try_new(name)
            .expect("valid name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .bind(bind_counter::bind)
            .function(bind_counter::write_bind);
        let builder = if volatile {
            builder.volatile()
        } else {
            builder
        };
        // SAFETY: `con` is open; the callbacks match the declared signature.
        unsafe { builder.register(fx.con()) }.unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    fx.query("CREATE TABLE bind_rows AS SELECT i::BIGINT AS i FROM range(3) t(i)");

    // Returns (first column, second column, binds during the query) for row 0.
    let run = |sql: &str| {
        let before = bind_counter::BINDS.load(Ordering::SeqCst);
        let mut result = fx.query(sql);
        let chunk = result.next_chunk().expect("fetch").expect("a chunk");
        // SAFETY: two valid BIGINT columns.
        let (a, b) = unsafe { (chunk.reader(0).read_i64(0), chunk.reader(1).read_i64(0)) };
        (a, b, bind_counter::BINDS.load(Ordering::SeqCst) - before)
    };

    let (a, b, binds) = run("SELECT bind_ctr(i), bind_ctr(i) FROM bind_rows");
    assert_eq!(binds, 2, "each call is bound on its own");
    assert_eq!(
        a, b,
        "yet the two calls are merged: one bind data answers for both"
    );

    let (a, b, binds) = run("SELECT bind_ctr_volatile(i), bind_ctr_volatile(i) FROM bind_rows");
    assert_eq!(binds, 2);
    assert_ne!(a, b, "volatile calls keep their own bind data");
}
