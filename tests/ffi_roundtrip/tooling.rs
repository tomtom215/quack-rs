// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Checks of quack-rs's tooling tables against the linked `DuckDB`.

use super::Fixture;
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
