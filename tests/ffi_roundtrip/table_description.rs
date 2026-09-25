// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `TableDescription`'s column accessors on an index `DuckDB` cannot hold.
//!
//! From 1.5.0 `duckdb_table_description_get_column_name`,
//! `_get_column_type` and `duckdb_column_has_default` convert the index to
//! an `optional_idx`, whose constructor throws `InternalException` for
//! `idx_t(-1)` (`optional_idx.hpp`), outside any `try`
//! (`table_description-c.cpp`). Before the fix `column_name(u64::MAX)`
//! aborted the process; every out-of-range index now returns `None`.

use quack_rs::table_description::TableDescription;

use super::Fixture;

#[test]
fn every_out_of_range_column_index_is_none_not_an_abort() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE td_probe (a INTEGER DEFAULT 1, b VARCHAR)");
    // SAFETY: `con` is open and the table exists.
    let desc = unsafe { TableDescription::create(fx.con(), "main", "td_probe") }.expect("describe");
    #[cfg(feature = "duckdb-1-5")]
    assert_eq!(desc.column_count(), 2);
    assert_eq!(desc.column_name(1).as_deref(), Some("b"));
    assert_eq!(desc.column_has_default(0), Some(true));
    for index in [2, 1 << 40, u64::MAX - 1, u64::MAX] {
        assert_eq!(desc.column_name(index), None, "{index}");
        assert_eq!(desc.column_has_default(index), None, "{index}");
        #[cfg(feature = "duckdb-1-5")]
        assert!(desc.column_type(index).is_none(), "{index}");
    }
}
