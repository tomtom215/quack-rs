// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! A `COPY … FROM` reader must not declare result columns.
//!
//! `CCopyFromBind` hands the reader's bind the `INSERT`'s own list of expected
//! types, and `duckdb_bind_add_result_column` appends to it, so each declared
//! column widens every chunk the `INSERT` receives past the table's width. A
//! release `DuckDB` drops the extra column; one built with assertions fails
//! `chunk.ColumnCount() == types.size()` in `RowGroupCollection::Append` and
//! invalidates the database (upstream item 37). A typed reader's bind declares
//! columns as a matter of course, so the typed builder refuses it here.

use quack_rs::copy_function::CopyFunctionBuilder;
use quack_rs::query::OwnedConnection;
use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::TypeId;

use super::Fixture;

#[derive(Clone)]
struct Once {
    done: bool,
}

#[test]
fn a_typed_copy_from_reader_that_declares_a_column_is_refused() {
    let fx = Fixture::open();
    let reader = TableFunctionBuilder::new("declares")
        .param(TypeId::Varchar)
        .with_state::<Once, _>(|bind| {
            // What a typed table function's bind normally does.
            bind.add_result_column("a", TypeId::BigInt);
            Ok(Once { done: false })
        })
        .scan(|state, chunk| {
            if !state.done {
                state.done = true;
                for c in 0..chunk.column_count() {
                    // SAFETY: every column is BIGINT; row 0 is in capacity.
                    unsafe { chunk.writer(c).write_i64(0, 5) };
                }
                // SAFETY: one row, within capacity.
                unsafe { chunk.set_size(1) };
            }
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: the fixture initialised the dispatch table.
    let handle = unsafe { reader.build_handle() }.expect("build_handle");
    let copy = CopyFunctionBuilder::try_new("declares")
        .expect("name")
        .copy_from(handle)
        .expect("copy_from");
    // SAFETY: the fixture's connection is open.
    unsafe { copy.register(fx.con()) }.expect("register");

    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    con.execute("CREATE TABLE t(a BIGINT)").expect("create");
    let err = con
        .execute("COPY t FROM '/dev/null' (FORMAT declares)")
        .expect_err("a reader that declares a column is refused");
    assert!(
        err.as_str().contains("must not declare result columns"),
        "{err}"
    );
    // Nothing was inserted, and the database is still usable.
    let mut result = con.query("SELECT count(*)::BIGINT FROM t").expect("select");
    let chunk = result.next_chunk().expect("fetch").expect("one row");
    // SAFETY: one BIGINT column, one row.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 0);
}
