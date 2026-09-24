// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Checks of quack-rs's tooling tables against the linked `DuckDB`.

use super::Fixture;
use quack_rs::types::TypeId;
use quack_rs::validate::{validate_function_name, DUCKDB_RESERVED_KEYWORDS};

/// `validate_function_name` rejects exactly `DuckDB`'s reserved keywords. The
/// list is pinned in the crate, so a `DuckDB` release that reserves a new
/// word (or releases one) must fail here rather than let an uncallable name
/// through — or reject a callable one.
#[test]
fn the_reserved_keyword_list_matches_the_linked_duckdb() {
    let fx = Fixture::open();
    let live = fx
        .scalar(
            "SELECT string_agg(keyword_name, ',' ORDER BY keyword_name) \
             FROM duckdb_keywords() WHERE keyword_category = 'reserved'",
            // SAFETY: row 0 of a VARCHAR column, checked valid by `scalar`.
            |r, i| unsafe { r.read_str(i).to_owned() },
        )
        .expect("DuckDB reports reserved keywords");
    assert_eq!(
        live,
        DUCKDB_RESERVED_KEYWORDS.join(","),
        "DUCKDB_RESERVED_KEYWORDS is out of date for this DuckDB"
    );
}

/// The reason the list exists: a function named after a reserved keyword
/// cannot be called without quotes, while a non-reserved keyword can.
#[test]
fn a_reserved_keyword_is_uncallable_unquoted_and_a_non_reserved_one_is_fine() {
    let fx = Fixture::open();
    // SAFETY: `fx.con()` is open for the fixture's lifetime.
    let run = |sql: &str| unsafe { quack_rs::query::query(fx.con(), sql) };

    run("CREATE MACRO \"order\"(x) AS x + 1").expect("quoted definition works");
    assert!(
        run("SELECT order(1)").is_err(),
        "a reserved keyword must not be callable unquoted"
    );
    assert!(validate_function_name("order").is_err());

    run("CREATE MACRO \"similar\"(x) AS x + 1").expect("quoted definition works");
    let two = fx.scalar("SELECT similar(1)", |r, i| unsafe { r.read_i32(i) });
    assert_eq!(two, Some(2));
    assert!(validate_function_name("similar").is_ok());
}

/// The fourth audit's T9. Only reserved keywords were refused, but 53
/// non-reserved ones cannot be called as a function either (`between(1)` and
/// `values(1)` do not parse; `coalesce(x)` is the COALESCE operator, so a
/// function of that name is never reached), and 79 cannot be a macro
/// parameter the body refers to (`CREATE MACRO m(left) AS left + 1` fails).
/// This repeats the measurement behind both lists for every keyword the
/// linked `DuckDB` reports, and requires each validator to accept exactly
/// the keywords that work.
#[test]
fn the_keyword_validators_agree_with_the_linked_duckdb_for_every_keyword() {
    use quack_rs::validate::validate_parameter_name;

    let fx = Fixture::open();
    let keywords = fx
        .scalar(
            "SELECT string_agg(keyword_name, ',' ORDER BY keyword_name) FROM duckdb_keywords()",
            // SAFETY: row 0 of a VARCHAR column, checked valid by `scalar`.
            |r, i| unsafe { r.read_str(i).to_owned() },
        )
        .expect("DuckDB reports its keywords");
    // Whether `setup` succeeds and `call` then returns the INTEGER 42.
    let answers = |setup: &str, call: &str| {
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let run = |sql: &str| unsafe { quack_rs::query::query(fx.con(), sql) };
        if run(setup).is_err() {
            return false;
        }
        let Ok(mut result) = run(call) else {
            return false;
        };
        let Ok(Some(chunk)) = result.next_chunk() else {
            return false;
        };
        let Some(ty) = result.column_logical_type(0) else {
            return false;
        };
        // SAFETY: `ty` is a live, owned logical type.
        if unsafe { ty.get_type_id() } != TypeId::Integer || chunk.size() != 1 {
            return false;
        }
        // SAFETY: one INTEGER row, read only if valid.
        unsafe { chunk.reader(0).is_valid(0) && chunk.reader(0).read_i32(0) == 42 }
    };
    let mut disagreements = Vec::new();
    for keyword in keywords.split(',') {
        // Callable with some argument count from 0 to 3: `nullif` fails with
        // one argument and works with two.
        let callable = (0..4).any(|n: usize| {
            let params: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
            let body = if n == 0 { "42" } else { "p0 + 1" };
            answers(
                &format!(
                    "CREATE OR REPLACE MACRO \"{keyword}\"({}) AS {body}",
                    params.join(", ")
                ),
                &format!("SELECT {keyword}({})", vec!["41"; n].join(", ")),
            )
        });
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let _ = unsafe {
            quack_rs::query::query(fx.con(), &format!("DROP MACRO IF EXISTS \"{keyword}\""))
        };
        if callable != validate_function_name(keyword).is_ok() {
            disagreements.push(format!("function name {keyword}: callable = {callable}"));
        }
        let referenceable = answers(
            &format!("CREATE OR REPLACE MACRO kw_param_probe({keyword}) AS {keyword} + 1"),
            "SELECT kw_param_probe(41)",
        );
        if referenceable != validate_parameter_name(keyword).is_ok() {
            disagreements.push(format!(
                "parameter name {keyword}: referenceable = {referenceable}"
            ));
        }
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}
