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
