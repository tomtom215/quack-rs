// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Regression tests for table functions, casts, replacement scans, SQL macros,
//! config options and catalog lookups against a real `DuckDB`.

use super::Fixture;
use quack_rs::query::query;
use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::TypeId;

// ─── Typed table functions: one bind, many inits ─────────────────────────────

/// Shared scan for both `count_down` variants: emits `remaining, …, 1`.
fn count_down_scan(remaining: &mut i64, chunk: &quack_rs::data_chunk::DataChunk) {
    if *remaining <= 0 {
        // SAFETY: end of stream.
        unsafe { chunk.set_size(0) };
        return;
    }
    // SAFETY: column 0 is BIGINT and row 0 is in range.
    unsafe {
        chunk.writer(0).write_i64(0, *remaining);
        chunk.set_size(1);
    }
    *remaining -= 1;
}

/// Registers `cd_state(n)` (built with `with_state`) and `cd_bind_init(n)`
/// (built with `with_bind_init`); both emit `n, n-1, …, 1`.
fn register_count_downs(con: libduckdb_sys::duckdb_connection) {
    #[derive(Clone)]
    struct State {
        remaining: i64,
    }
    let with_state = TableFunctionBuilder::new("cd_state")
        .param(TypeId::BigInt)
        .with_state::<State, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            // SAFETY: parameter 0 was declared above.
            let n = unsafe { bind.get_parameter_value(0) }.as_i64_or(0);
            Ok(State { remaining: n })
        })
        .scan(|state, chunk| {
            count_down_scan(&mut state.remaining, chunk);
            Ok(())
        })
        .build()
        .expect("build cd_state");
    // SAFETY: `con` is open.
    unsafe { with_state.register(con) }.expect("register cd_state");

    /// Deliberately not `Clone`: `with_bind_init` must not need it.
    struct Cursor {
        remaining: i64,
    }
    let with_bind_init = TableFunctionBuilder::new("cd_bind_init")
        .param(TypeId::BigInt)
        .with_bind_init(
            |bind| {
                bind.add_result_column("n", TypeId::BigInt);
                // SAFETY: parameter 0 was declared above.
                Ok(unsafe { bind.get_parameter_value(0) }.as_i64_or(0))
            },
            |n: &i64| Ok(Cursor { remaining: *n }),
        )
        .scan(|cursor, chunk| {
            count_down_scan(&mut cursor.remaining, chunk);
            Ok(())
        })
        .build()
        .expect("build cd_bind_init");
    // SAFETY: `con` is open.
    unsafe { with_bind_init.register(con) }.expect("register cd_bind_init");
}

/// `DuckDB` reuses one bind-data object for every init of a bound plan. A
/// prepared statement binds once and initialises once per `EXECUTE`; before
/// the fix the second `EXECUTE` failed with "bind state already consumed"
/// because `init` moved the state out of the bind data.
#[test]
fn a_typed_table_function_survives_repeated_execution_of_a_prepared_statement() {
    let fx = Fixture::open();
    register_count_downs(fx.con());

    for function in ["cd_state", "cd_bind_init"] {
        fx.query(&format!(
            "PREPARE p_{function} AS SELECT sum(n) FROM {function}(3)"
        ));
        for round in 0..3 {
            assert_eq!(
                fx.scalar(&format!("EXECUTE p_{function}"), |r, i| unsafe {
                    r.read_i128(i)
                }),
                Some(6),
                "{function}: EXECUTE round {round}"
            );
        }
    }
}

/// A recursive CTE re-initialises the table function it references on every
/// iteration, against the same bind data.
#[test]
fn a_typed_table_function_survives_reinitialisation_in_a_recursive_cte() {
    let fx = Fixture::open();
    register_count_downs(fx.con());

    for function in ["cd_state", "cd_bind_init"] {
        // Every iteration re-scans `{function}(2)`; each scan must see both rows.
        let sql = format!(
            "WITH RECURSIVE r(i) AS (SELECT 0 UNION ALL \
             SELECT i + 1 FROM r WHERE i < 3 AND (SELECT count(*) FROM {function}(2)) = 2) \
             SELECT max(i) FROM r"
        );
        assert_eq!(
            fx.scalar(&sql, |r, i| unsafe { r.read_i32(i) }),
            Some(3),
            "{function}"
        );
    }
}

/// An error from the `with_bind_init` init closure reaches SQL.
#[test]
fn a_with_bind_init_init_error_is_reported() {
    use quack_rs::error::ExtensionError;

    let fx = Fixture::open();
    let builder = TableFunctionBuilder::new("init_fails")
        .with_bind_init(
            |bind| {
                bind.add_result_column("n", TypeId::BigInt);
                Ok(())
            },
            |()| Err::<(), _>(ExtensionError::new("init refused to start")),
        )
        .scan(|(), chunk| {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register init_fails");

    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT * FROM init_fails()") }
        .expect_err("the init error must surface");
    assert!(err.as_str().contains("init refused to start"), "{err}");
}

// ─── Typed builder and projection pushdown ───────────────────────────────────

/// The typed builder used to offer `projection_pushdown(true)`, after which
/// `SELECT b FROM two_cols()` returned column `a`'s value: `DuckDB` handed the
/// scan a one-column chunk and the closure could not know that column 0 was
/// now `b`. The method is gone (a `compile_fail` doctest on the `typed`
/// module pins that); this checks the typed scan always sees the declared
/// schema, so selecting a subset of columns returns the right values.
#[test]
fn a_typed_table_function_returns_the_selected_column_not_the_first() {
    let fx = Fixture::open();
    let builder = TableFunctionBuilder::new("two_cols")
        .with_state(|bind| {
            bind.add_result_column("a", TypeId::BigInt);
            bind.add_result_column("b", TypeId::BigInt);
            Ok(false)
        })
        .scan(|done: &mut bool, chunk| {
            if *done {
                // SAFETY: end of stream.
                unsafe { chunk.set_size(0) };
                return Ok(());
            }
            assert_eq!(
                chunk.column_count(),
                2,
                "the scan sees every declared column"
            );
            // SAFETY: both BIGINT columns exist and row 0 is in range.
            unsafe {
                chunk.writer(0).write_i64(0, 1);
                chunk.writer(1).write_i64(0, 2);
                chunk.set_size(1);
            }
            *done = true;
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register two_cols");

    assert_eq!(
        fx.scalar("SELECT b FROM two_cols()", |r, i| unsafe { r.read_i64(i) }),
        Some(2)
    );
    assert_eq!(
        fx.scalar("SELECT a FROM two_cols()", |r, i| unsafe { r.read_i64(i) }),
        Some(1)
    );
}

// ─── BindInfo::add_result_column with an unusable type ───────────────────────

/// `duckdb_bind_add_result_column` silently drops a column whose type contains
/// `ANY` or `INVALID`, shifting every later column index by one. Before the fix
/// `SELECT *` here returned one column (`b`) and the scan's write to column 1
/// went nowhere; now the bind fails with a message naming the column.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_result_column_of_type_any_is_a_bind_error_not_a_dropped_column() {
    let fx = Fixture::open();

    let builder = TableFunctionBuilder::new("any_col")
        .with_state::<u8, _>(|bind| {
            bind.add_result_column("a", TypeId::Any);
            bind.add_result_column("b", TypeId::BigInt);
            Ok(0)
        })
        .scan(|_, chunk| {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register any_col");

    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT * FROM any_col()") }
        .expect_err("a column DuckDB would drop must fail the bind");
    assert!(err.as_str().contains("'a'"), "{err}");

    // Nested: LIST(ANY) is dropped by DuckDB just the same.
    let nested = TableFunctionBuilder::new("any_list_col")
        .with_state::<u8, _>(|bind| {
            bind.add_result_column_with_type(
                "xs",
                &quack_rs::types::LogicalType::list(TypeId::Any),
            );
            Ok(0)
        })
        .scan(|_, chunk| {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { nested.register(fx.con()) }.expect("register any_list_col");
    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT * FROM any_list_col()") }
        .expect_err("LIST(ANY) must fail the bind");
    assert!(err.as_str().contains("'xs'"), "{err}");
}

/// A composite id such as `TypeId::List` cannot be built from the id alone.
/// `add_result_column` used to panic on it (fatal from a raw `extern "C"`
/// bind callback); it is now a bind error naming the column.
#[test]
fn a_composite_result_column_type_id_is_a_bind_error() {
    let fx = Fixture::open();
    let builder = TableFunctionBuilder::new("composite_col")
        .with_state::<u8, _>(|bind| {
            bind.add_result_column("xs", TypeId::List);
            Ok(0)
        })
        .scan(|_, chunk| {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register composite_col");
    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT * FROM composite_col()") }
        .expect_err("a composite id must fail the bind");
    let message = err.as_str();
    assert!(message.contains("'xs'"), "{message}");
    assert!(
        !message.contains("panicked"),
        "no panic any more: {message}"
    );
}

// ─── Replacement scans: an empty error message ───────────────────────────────

quack_rs::replacement_scan_callback!(empty_error_scan, |info, table_name, _data| {
    // SAFETY: DuckDB passes a valid NUL-terminated identifier.
    let name = unsafe { std::ffi::CStr::from_ptr(table_name) }.to_string_lossy();
    if name.ends_with(".empty_err") {
        // SAFETY: `info` is the pointer DuckDB passed in.
        unsafe { quack_rs::replacement_scan::ReplacementScanInfo::new(info) }.set_error("");
    } else if name.ends_with(".nul_err") {
        // SAFETY: as above.
        unsafe { quack_rs::replacement_scan::ReplacementScanInfo::new(info) }.set_error("\0hidden");
    }
});

/// `DuckDB` only raises a replacement-scan error when the message is
/// non-empty, so before the fix `set_error("")` was ignored and the query fell
/// through to "table does not exist".
#[test]
fn an_empty_replacement_scan_error_is_still_an_error() {
    use quack_rs::replacement_scan::ReplacementScanBuilder;

    let fx = Fixture::open();
    // SAFETY: `db` is open; no extra data.
    unsafe {
        ReplacementScanBuilder::register(fx.db(), empty_error_scan, std::ptr::null_mut(), None);
    }
    for table in ["'x.empty_err'", "'x.nul_err'"] {
        // SAFETY: `con` is open.
        let err = unsafe { query(fx.con(), &format!("SELECT * FROM {table}")) }
            .expect_err("the query must fail");
        assert!(
            err.as_str().contains("Error in replacement scan"),
            "{table}: the replacement scan's error must be raised, got: {err}"
        );
    }
}

// ─── Cast functions: extra_info on a rejected registration ───────────────────

#[cfg(feature = "duckdb-1-5")]
mod cast_extra_info {
    use std::os::raw::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::Fixture;
    use quack_rs::cast::CastFunctionBuilder;
    use quack_rs::types::{LogicalType, TypeId};

    static FREED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn count_free(ptr: *mut c_void) {
        if !ptr.is_null() {
            // SAFETY: `ptr` came from `Box::into_raw` below.
            drop(unsafe { Box::from_raw(ptr.cast::<u64>()) });
            FREED.fetch_add(1, Ordering::SeqCst);
        }
    }

    quack_rs::cast_callback!(never_cast, |_info, _count, _input, _output| { true });

    /// `duckdb_register_cast_function` returns `DuckDBError` for a source type
    /// containing `INVALID`/`ANY` *before* it creates the object that owns
    /// `extra_info`, so nothing frees it. Before the fix the builder had
    /// already marked it transferred, so the destructor ran zero times.
    #[test]
    fn a_rejected_cast_registration_frees_its_extra_info_exactly_once() {
        let fx = Fixture::open();
        let before = FREED.load(Ordering::SeqCst);

        let data = Box::into_raw(Box::new(7_u64)).cast::<c_void>();
        // LIST(ANY): DuckDB refuses a cast whose source type contains ANY.
        let bad_source = LogicalType::list(TypeId::Any);
        // SAFETY: `con` is open; `data` is freed by `count_free`.
        let result = unsafe {
            CastFunctionBuilder::new_logical(bad_source, LogicalType::new(TypeId::BigInt))
                .function(never_cast)
                .extra_info(data, Some(count_free))
                .register(fx.con())
        };
        assert!(result.is_err(), "a source type containing ANY is rejected");
        assert_eq!(
            FREED.load(Ordering::SeqCst) - before,
            1,
            "extra_info must be freed exactly once"
        );
    }
}

// ─── SQL macros: quoted identifiers ──────────────────────────────────────────

/// `SqlMacro` now emits its name and parameters as quoted identifiers. That
/// fixes keyword names (before the fix `CREATE OR REPLACE MACRO order(select)`
/// was a parser error), and must not change how the macro is called: `DuckDB`
/// resolves quoted identifiers case-insensitively, so a macro registered as
/// `"MyMacro"("X")` is still callable as `mymacro`, `MYMACRO`, `MyMacro`.
///
/// No keyword *name* is registered here: `validate_function_name` is being
/// tightened separately to reject keywords, and parameter names go through the
/// same validator.
#[test]
fn a_quoted_sql_macro_is_still_called_case_insensitively() {
    use quack_rs::sql_macro::SqlMacro;

    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        SqlMacro::scalar("MyMacro", &["X"], "x + 1")
            .expect("valid names")
            .register(fx.con())
            .expect("register MyMacro");
        SqlMacro::table("MyRows", &["N"], "SELECT * FROM range(n)")
            .expect("valid names")
            .register(fx.con())
            .expect("register MyRows");
    }
    for call in [
        "mymacro(41)",
        "MYMACRO(41)",
        "MyMacro(41)",
        "mymacro(x := 41)",
    ] {
        let sql = format!("SELECT {call}::BIGINT");
        assert_eq!(
            fx.scalar(&sql, |r, i| unsafe { r.read_i64(i) }),
            Some(42),
            "{call}"
        );
    }
    assert_eq!(
        fx.scalar("SELECT count(*) FROM myrows(3)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(3)
    );
}

// ─── Config options: a default that does not cast ────────────────────────────

/// Before the fix this aborted the whole test process ("Rust cannot catch
/// foreign exceptions"): `duckdb_config_option_set_default_value` casts with
/// a throwing cast and no `try`/`catch`. It is now an `Err`, and a valid
/// typed default still registers.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_config_option_default_that_does_not_cast_is_an_error_not_an_abort() {
    use quack_rs::config_option::ConfigOptionBuilder;

    let fx = Fixture::open();
    for (name, ty, default) in [
        ("tc_bad_bigint", TypeId::BigInt, "abc"),
        ("tc_bad_bool", TypeId::Boolean, "maybe"),
        ("tc_bad_date", TypeId::Date, "not a date"),
        ("tc_overflow_tinyint", TypeId::TinyInt, "1000"),
    ] {
        // SAFETY: `con` is open.
        let err = unsafe {
            ConfigOptionBuilder::try_new(name)
                .expect("name")
                .option_type(ty)
                .default_value(default)
                .expect("default")
                .register(fx.con())
        }
        .expect_err("an uncastable default must be refused");
        assert!(err.as_str().contains("cannot be cast"), "{name}: {err}");
    }

    // SAFETY: `con` is open.
    unsafe {
        ConfigOptionBuilder::try_new("tc_good_bigint")
            .expect("name")
            .option_type(TypeId::BigInt)
            .default_value("42")
            .expect("default")
            .register(fx.con())
    }
    .expect("a castable default registers");
    assert_eq!(
        fx.scalar("SELECT current_setting('tc_good_bigint')", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(42)
    );
}

// ─── Catalog lookups in catalogs other than DuckDB's own ─────────────────────

/// Lookups are refused in a catalog that is not `DuckDB`'s own: for one an
/// extension provides, `duckdb_catalog_get_entry` runs the extension's
/// `LookupSchema` (and starts its transaction) with no `try`, so an exception
/// there would abort the process. `DuckDB`'s own catalogs (the database's,
/// `temp` and `system`) are all `DuckCatalog`s of type `"duckdb"` and are
/// still looked up. No extension catalog can be attached offline, so the
/// refusal itself is covered by the unit tests in `src/catalog.rs`.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn catalog_lookups_run_in_every_catalog_duckdb_itself_provides() {
    use quack_rs::catalog::CatalogEntryType;
    use quack_rs::client_context::ClientContext;

    let fx = Fixture::open();
    fx.query("CREATE TABLE tc_own (id INTEGER)");
    fx.query("CREATE TEMP TABLE tc_temp (id INTEGER)");
    // SAFETY: `con` is open.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    fx.query("BEGIN TRANSACTION");
    for (catalog_name, schema, name, entry_type) in [
        (c"memory", c"main", c"tc_own", CatalogEntryType::Table),
        (c"temp", c"main", c"tc_temp", CatalogEntryType::Table),
        (
            c"system",
            c"information_schema",
            c"tables",
            CatalogEntryType::View,
        ),
    ] {
        // SAFETY: inside a transaction.
        let catalog = unsafe { ctx.catalog(catalog_name) }.expect("catalog");
        assert_eq!(catalog.type_name(), Some("duckdb"), "{catalog_name:?}");
        // SAFETY: catalog and context are valid and a transaction is active.
        let entry = unsafe { catalog.get_entry(ctx.as_raw(), schema, name, entry_type) }
            .expect("a lookup in DuckDB's own catalog is not refused")
            .expect("the entry exists");
        assert_eq!(entry.name(), name.to_str().ok(), "{catalog_name:?}");
    }
    fx.query("COMMIT");
}

// ─── Catalog lookups of non-schema entry types ───────────────────────────────

/// Before the fix each of these lookups aborted the process: `DuckDB` throws
/// "Unsupported catalog type in schema" from inside `duckdb_catalog_get_entry`
/// with no `try`/`catch`. They are now refused without calling `DuckDB`, and
/// a supported lookup in the same transaction still works.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn catalog_lookup_of_a_non_schema_entry_type_is_refused() {
    use quack_rs::catalog::{CatalogEntry, CatalogEntryType};
    use quack_rs::client_context::ClientContext;

    let fx = Fixture::open();
    fx.query("CREATE TABLE tc_probe (id INTEGER)");
    // SAFETY: `con` is open.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    fx.query("BEGIN TRANSACTION");
    // SAFETY: inside a transaction.
    let catalog = unsafe { ctx.catalog(c"memory") }.expect("the memory catalog");

    for entry_type in [
        CatalogEntryType::Schema,
        CatalogEntryType::Database,
        CatalogEntryType::PreparedStatement,
        CatalogEntryType::Invalid,
    ] {
        // SAFETY: catalog and context are valid and a transaction is active.
        let entry = unsafe { catalog.get_entry(ctx.as_raw(), c"main", c"main", entry_type) };
        assert!(entry.is_err(), "{entry_type:?}");
        // SAFETY: as above.
        let entry = unsafe {
            CatalogEntry::lookup(
                catalog.as_raw(),
                ctx.as_raw(),
                c"main",
                c"memory",
                entry_type,
            )
        };
        assert!(entry.is_err(), "{entry_type:?}");
    }

    // SAFETY: as above.
    let table =
        unsafe { catalog.get_entry(ctx.as_raw(), c"main", c"tc_probe", CatalogEntryType::Table) }
            .expect("a supported lookup is not refused")
            .expect("a supported lookup still works");
    assert_eq!(table.name(), Some("tc_probe"));

    drop(table);
    drop(catalog);
    fx.query("COMMIT");
}

/// TBL-2: a `TYPE` lookup of `inet` (or `json`, or an ICU collation name)
/// made `DuckDB` autoload the owning extension on a miss, and a failed
/// autoload threw through `duckdb_catalog_get_entry`, aborting the process
/// (probe t10: "Rust cannot catch foreign exceptions"). With autoload on the
/// lookup is now refused; with it off, it runs and simply misses.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn catalog_lookup_of_an_autoloadable_name_is_refused_while_autoload_is_on() {
    use quack_rs::catalog::CatalogEntryType;
    use quack_rs::client_context::ClientContext;

    let fx = Fixture::open();
    fx.query("SET autoinstall_known_extensions = false");
    fx.query("SET autoload_known_extensions = true");
    fx.query("CREATE TYPE tc_mood AS ENUM ('ok')");
    // SAFETY: `con` is open.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    fx.query("BEGIN TRANSACTION");
    // SAFETY: inside a transaction.
    let catalog = unsafe { ctx.catalog(c"memory") }.expect("the memory catalog");
    let lookup = |name: &std::ffi::CStr, ty| {
        // SAFETY: catalog and context are valid and a transaction is active.
        unsafe { catalog.get_entry(ctx.as_raw(), c"main", name, ty) }
    };

    for (name, ty) in [
        (c"inet", CatalogEntryType::Type),
        (c"JSON", CatalogEntryType::Type),
        (c"de", CatalogEntryType::Collation),
    ] {
        let err = lookup(name, ty).expect_err("must be refused while autoload is on");
        assert!(
            err.as_str().contains("autoload_known_extensions"),
            "{name:?}: {err}"
        );
    }
    // Ordinary names are unaffected.
    assert!(lookup(c"tc_mood", CatalogEntryType::Type)
        .expect("not refused")
        .is_some());
    assert!(lookup(c"tc_no_such_type", CatalogEntryType::Type)
        .expect("not refused")
        .is_none());
    fx.query("COMMIT");

    // With autoload off, no autoload is attempted and the lookup just misses.
    fx.query("SET autoload_known_extensions = false");
    fx.query("BEGIN TRANSACTION");
    assert!(lookup(c"inet", CatalogEntryType::Type)
        .expect("not refused with autoload off")
        .is_none());
    assert!(lookup(c"de", CatalogEntryType::Collation)
        .expect("not refused with autoload off")
        .is_none());
    drop(catalog);
    fx.query("COMMIT");
}

// ─── Typed table functions contain a panic payload whose `Drop` panics ──────

/// A panic payload whose own destructor panics.
struct TablePayloadBomb;
impl Drop for TablePayloadBomb {
    fn drop(&mut self) {
        panic!("table panic payload destructor deliberately exploded");
    }
}

/// The typed table trampolines copied the panic message out of the caught
/// payload and then dropped the payload at the end of the `if let`, inside the
/// `extern "C" fn` and outside any guard. A payload whose `Drop` panics
/// therefore aborted the process (`panic_cannot_unwind`) instead of failing the
/// query — the same defect the callback macros had, in the one set of
/// trampolines that does not go through them. Covers bind and scan.
#[test]
fn a_typed_table_panic_payload_whose_drop_panics_becomes_a_sql_error() {
    let fx = Fixture::open();
    let bind_bomb = TableFunctionBuilder::new("tc_bind_bomb")
        .with_state::<u8, _>(|_bind| std::panic::panic_any(TablePayloadBomb))
        .scan(|_state, _chunk| Ok(()))
        .build()
        .expect("build tc_bind_bomb");
    let scan_bomb = TableFunctionBuilder::new("tc_scan_bomb")
        .with_state::<u8, _>(|bind| {
            bind.add_result_column("x", TypeId::BigInt);
            Ok(0)
        })
        .scan(|_state, _chunk| std::panic::panic_any(TablePayloadBomb))
        .build()
        .expect("build tc_scan_bomb");
    // SAFETY: `con` is open.
    unsafe {
        bind_bomb.register(fx.con()).expect("register tc_bind_bomb");
        scan_bomb.register(fx.con()).expect("register tc_scan_bomb");
    }

    for sql in [
        "SELECT * FROM tc_bind_bomb()",
        "SELECT * FROM tc_scan_bomb()",
    ] {
        // SAFETY: `con` is open.
        let err =
            unsafe { query(fx.con(), sql) }.expect_err("the panic must surface as a SQL error");
        assert!(err.as_str().contains("panicked"), "{sql}: {err}");
    }

    // The connection survives both.
    // SAFETY: `con` is open.
    assert!(unsafe { query(fx.con(), "SELECT 1") }.is_ok());
}

// ─── Third audit (table area) regressions ────────────────────────────────────

/// TBL-1: `ClientContext::config_option` on a setting whose value is `NULL`
/// aborted the process — `duckdb_get_varchar` throws on a `NULL` value from
/// inside the C API (probes t08 and t29: "Rust cannot catch foreign
/// exceptions"). It is now `None`.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn reading_a_null_config_option_is_none_not_an_abort() {
    use quack_rs::client_context::ClientContext;
    use quack_rs::config_option::ConfigOptionBuilder;

    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        ConfigOptionBuilder::try_new("tc_nullable")
            .expect("name")
            .option_type(TypeId::BigInt)
            .default_value("5")
            .expect("default")
            .register(fx.con())
    }
    .expect("register");
    // SAFETY: `con` is open for the rest of the test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    assert_eq!(ctx.config_option(c"tc_nullable").as_deref(), Some("5"));
    fx.query("SET tc_nullable = NULL");
    assert_eq!(ctx.config_option(c"tc_nullable"), None);
    // A built-in setting that is NULL until set.
    assert_eq!(ctx.config_option(c"enable_profiling"), None);
}

/// TBL-3: a default that SQL `TRY_CAST` accepts but `DuckDB`'s built-in
/// `DefaultCastAs` does not — a `TIMESTAMPTZ` with a zone name, which only
/// ICU's cast understands — passed quack-rs's old check and then aborted
/// inside `duckdb_config_option_set_default_value` (probe t09). The default is
/// now converted by SQL and handed over already typed, so it registers (and
/// without ICU, where SQL cannot read it either, is refused), and every
/// supported type stores exactly what `TRY_CAST` produces.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_config_option_default_is_converted_by_sql_not_inside_the_c_api() {
    use quack_rs::config_option::ConfigOptionBuilder;

    let fx = Fixture::open();
    let icu = fx.scalar(
        "SELECT loaded FROM duckdb_extensions() WHERE extension_name = 'icu'",
        |r, i| unsafe { r.read_bool(i) },
    ) == Some(true);
    // The prebuilt library has ICU compiled in. `bundled-test` compiles
    // DuckDB with libduckdb-sys's `cc` build, which does not (its `icu`
    // feature needs the `bundled-cmake` build).
    if !cfg!(feature = "bundled-test") {
        assert!(
            icu,
            "this regression needs ICU's VARCHAR -> TIMESTAMPTZ cast"
        );
    }
    let zoned = "2020-01-01 10:00:00 America/New_York";
    if !icu {
        // Without ICU, SQL cannot read a zone name either, so `TRY_CAST` is
        // NULL and the default is refused before the C API sees it.
        // SAFETY: `con` is open.
        let refused = unsafe {
            ConfigOptionBuilder::try_new("tc_typed_default_zoned")
                .expect("name")
                .option_type(TypeId::TimestampTz)
                .default_value(zoned)
                .expect("default")
                .register(fx.con())
        };
        assert!(refused.is_err(), "{zoned:?} without ICU");
    }
    for (i, (ty, default)) in [
        (TypeId::TimestampTz, zoned),
        (TypeId::Boolean, "true"),
        (TypeId::TinyInt, "-7"),
        (TypeId::SmallInt, "300"),
        (TypeId::Integer, "-70000"),
        (TypeId::BigInt, "42"),
        (TypeId::UTinyInt, "250"),
        (TypeId::USmallInt, "65000"),
        (TypeId::UInteger, "4000000000"),
        (TypeId::UBigInt, "18446744073709551615"),
        (TypeId::HugeInt, "-170141183460469231731687303715884105728"),
        (TypeId::UHugeInt, "340282366920938463463374607431768211455"),
        (TypeId::Float, "1.5"),
        (TypeId::Double, "1e308"),
        (TypeId::Date, "2024-02-29"),
        (TypeId::Time, "12:34:56.789"),
        (TypeId::TimeTz, "12:34:56+02"),
        (TypeId::Timestamp, "2024-02-29 12:34:56.123456"),
        (TypeId::TimestampS, "2024-02-29 12:34:56"),
        (TypeId::TimestampMs, "2024-02-29 12:34:56.123"),
        (TypeId::TimestampNs, "2024-02-29 12:34:56.123456789"),
        (TypeId::Interval, "1 year 2 days 3 hours"),
        (TypeId::Varchar, "plain text"),
        (TypeId::Varchar, ""),
        (TypeId::Blob, "\\xAA\\x00b"),
        (TypeId::Uuid, "11111111-2222-3333-4444-555555555555"),
        (TypeId::Bit, "0101100111"),
    ]
    .into_iter()
    .enumerate()
    .filter(|&(_, (_, default))| icu || default != zoned)
    {
        let name = format!("tc_typed_default_{i}");
        // SAFETY: `con` is open.
        unsafe {
            ConfigOptionBuilder::try_new(&name)
                .expect("name")
                .option_type(ty)
                .default_value(default)
                .expect("default")
                .register(fx.con())
        }
        .unwrap_or_else(|e| panic!("{ty:?} default {default:?}: {e}"));
        let expected = fx.scalar(
            &format!(
                "SELECT TRY_CAST('{}' AS {})::VARCHAR",
                default.replace('\'', "''"),
                ty.sql_name()
            ),
            |r, i| unsafe { r.read_str(i).to_owned() },
        );
        let stored = fx.scalar(
            &format!("SELECT current_setting('{name}')::VARCHAR"),
            |r, i| unsafe { r.read_str(i).to_owned() },
        );
        assert!(expected.is_some(), "{ty:?}: {default:?} must convert");
        assert_eq!(stored, expected, "{ty:?} default {default:?}");
        let same_type = fx.scalar(
            &format!(
                "SELECT typeof(current_setting('{name}')) = typeof(TRY_CAST(NULL AS {}))",
                ty.sql_name()
            ),
            |r, i| unsafe { r.read_bool(i) },
        );
        assert_eq!(
            same_type,
            Some(true),
            "{ty:?}: the setting keeps the option's type"
        );
    }
}

/// TBL-14: an extension option could take the name of a built-in setting
/// (`threads`), shadowing it for `current_setting`.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_config_option_may_not_take_a_built_in_settings_name() {
    use quack_rs::config_option::ConfigOptionBuilder;

    let fx = Fixture::open();
    for name in ["threads", "THREADS", "worker_threads"] {
        // SAFETY: `con` is open.
        let err = unsafe {
            ConfigOptionBuilder::try_new(name)
                .expect("name")
                .option_type(TypeId::BigInt)
                .default_value("1")
                .expect("default")
                .register(fx.con())
        }
        .expect_err("a built-in setting's name must be refused");
        assert!(err.as_str().contains("already exists"), "{name}: {err}");
    }
    assert!(fx
        .scalar("SELECT current_setting('threads')", |r, i| unsafe {
            r.read_i64(i)
        })
        .is_some_and(|n| n >= 1));
}

/// TBL-23: an option with no default reads as "unrecognized configuration
/// parameter", and `ANY` / `SQLNULL` options failed with a parser or catalog
/// error from quack-rs's own cast check.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn config_options_without_a_default_or_with_a_pseudo_type_are_refused_clearly() {
    use quack_rs::config_option::ConfigOptionBuilder;

    let fx = Fixture::open();
    // SAFETY: `con` is open.
    let err = unsafe {
        ConfigOptionBuilder::try_new("tc_no_default")
            .expect("name")
            .option_type(TypeId::BigInt)
            .register(fx.con())
    }
    .expect_err("an option without a default must be refused");
    assert!(err.as_str().contains("no default value"), "{err}");

    for ty in [TypeId::Any, TypeId::SqlNull] {
        // SAFETY: `con` is open.
        let err = unsafe {
            ConfigOptionBuilder::try_new("tc_pseudo")
                .expect("name")
                .option_type(ty)
                .default_value("1")
                .expect("default")
                .register(fx.con())
        }
        .expect_err("a pseudo-type option must be refused");
        assert!(
            err.as_str()
                .contains("cannot be the type of a config option"),
            "{ty:?}: {err}"
        );
    }
}

/// TBL-10: `FileFlag::CreateNew` mapped to `FILE_FLAGS_EXCLUSIVE_CREATE`
/// alone, which only means something together with `FILE_FLAGS_FILE_CREATE`:
/// it neither created a missing file nor refused an existing one.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn file_flag_create_new_creates_or_refuses() {
    use quack_rs::client_context::ClientContext;
    use quack_rs::file_system::{FileFlag, FileOpenOptions, FileSystem};

    let fx = Fixture::open();
    // SAFETY: `con` is open for the whole test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    let dir = std::env::temp_dir().join(format!("quack_rs_tc_create_new_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let existing = dir.join("existing.bin");
    std::fs::write(&existing, b"keep me").expect("seed file");
    let fresh = dir.join("fresh.bin");
    let _ = std::fs::remove_file(&fresh);
    let open = |path: &std::path::Path| {
        let options = FileOpenOptions::new();
        options.set_flag(FileFlag::Write, true);
        options.set_flag(FileFlag::CreateNew, true);
        let c_path = std::ffi::CString::new(path.to_str().expect("utf-8 path")).expect("no NUL");
        fs.open(&c_path, &options).map(drop)
    };
    assert!(open(&existing).is_err(), "an existing file must be refused");
    assert_eq!(std::fs::read(&existing).expect("still there"), b"keep me");
    assert!(open(&fresh).is_ok(), "a missing file must be created");
    assert!(fresh.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The error message from running `sql`, which must fail.
fn error_of(fx: &Fixture, sql: &str) -> String {
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), sql) }
        .map(drop)
        .expect_err(sql)
        .as_str()
        .to_owned()
}

/// TBL-11: a failed fold carried the exception's raw JSON
/// (`{"exception_type":"Conversion",...}`) as its message and always the
/// type `InvalidInput`.
#[cfg(feature = "duckdb-1-5")]
mod fold_errors {
    use super::{Fixture, TypeId};
    use quack_rs::error_data::DuckDbErrorType;
    use quack_rs::scalar::{ScalarBindInfo, ScalarFunctionBuilder};
    use std::sync::{Mutex, PoisonError};

    static SEEN: Mutex<Vec<(DuckDbErrorType, String)>> = Mutex::new(Vec::new());

    unsafe extern "C" fn fold_bind(info: quack_rs::scalar::RawScalarBindInfo) {
        // SAFETY: `info` is the live bind info.
        let bind = unsafe { ScalarBindInfo::new(info) };
        // SAFETY: argument 0 was declared.
        let Some(expr) = (unsafe { bind.argument(0) }) else {
            return;
        };
        // SAFETY: used only inside this callback.
        let ctx = unsafe { bind.get_client_context() };
        if let Err(e) = expr.fold(&ctx) {
            SEEN.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((e.error_type(), e.message().unwrap_or_default()));
        }
    }
    quack_rs::scalar_callback!(fold_exec, |_info, input, output| {
        // SAFETY: `input` and `output` are the live vectors.
        let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
        let mut writer = unsafe { quack_rs::vector::VectorWriter::from_vector(output) };
        for row in 0..chunk.size() {
            // SAFETY: `row` is in range.
            unsafe { writer.write_i64(row, 0) };
        }
    });

    #[test]
    fn a_failed_fold_reports_the_message_and_type_not_json() {
        let fx = Fixture::open();
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe {
            ScalarFunctionBuilder::try_new("tc_fold")
                .expect("name")
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .bind(fold_bind)
                .function(fold_exec)
                .register(fx.con())
                .expect("register");
        }
        if !crate::inspects_arguments_or_refuses(&fx, "SELECT tc_fold(1::BIGINT)") {
            return;
        }
        for (sql, ty, message) in [
            (
                "SELECT tc_fold('abc'::BIGINT)",
                DuckDbErrorType::Conversion,
                "Could not convert string 'abc' to INT64",
            ),
            (
                "SELECT tc_fold(9223372036854775807 + 1)",
                DuckDbErrorType::OutOfRange,
                "Overflow in addition of INT64 (9223372036854775807 + 1)!",
            ),
        ] {
            SEEN.lock().unwrap_or_else(PoisonError::into_inner).clear();
            let _ = super::error_of(&fx, sql);
            let seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner).clone();
            assert_eq!(seen, vec![(ty, message.to_owned())], "{sql}");
        }
    }
}

/// TBL-8: a scalar macro body ending in a `--` comment swallowed the closing
/// parenthesis quack-rs appends, and a body holding a statement separator ran
/// every statement in it.
#[test]
fn sql_macro_bodies_with_comments_work_and_extra_statements_are_refused() {
    use quack_rs::sql_macro::SqlMacro;

    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        SqlMacro::scalar("tc_plus_one", &["x"], "x + 1 -- plus one")
            .expect("valid names")
            .register(fx.con())
    }
    .expect("a trailing line comment must not break the macro");
    assert_eq!(
        fx.scalar("SELECT tc_plus_one(41)::BIGINT", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(42)
    );

    fx.query("CREATE TABLE tc_victim(i INTEGER)");
    // SAFETY: `con` is open.
    let err = unsafe {
        SqlMacro::scalar("tc_inject", &[], "1); DROP TABLE tc_victim; SELECT (1")
            .expect("valid names")
            .register(fx.con())
    }
    .expect_err("a body holding extra statements must be refused");
    assert!(err.as_str().contains("statement"), "{err}");
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = 'tc_victim'",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(1),
        "the table must survive"
    );
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM duckdb_functions() WHERE function_name = 'tc_inject'",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(0),
        "nothing is created"
    );
}

/// TBL-6: `projection_pushdown(true)` set on the raw builder *before*
/// `with_state` survived into the typed function, so `SELECT b` returned
/// column `a`'s value. `build` now refuses it.
#[test]
fn the_typed_builder_refuses_projection_pushdown_set_beforehand() {
    let err = TableFunctionBuilder::new("tc_pushdown")
        .projection_pushdown(true)
        .with_state::<u8, _>(|bind| {
            bind.add_result_column("a", TypeId::BigInt);
            Ok(0)
        })
        .scan(|_state, chunk| {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect_err("projection pushdown must be refused");
    assert!(err.as_str().contains("projection_pushdown"), "{err}");

    let err = TableFunctionBuilder::new("tc_pushdown_bi")
        .projection_pushdown(true)
        .with_bind_init(|_bind| Ok(()), |(): &()| Ok(0_u8))
        .scan(|_state, _chunk| Ok(()))
        .build()
        .expect_err("projection pushdown must be refused");
    assert!(err.as_str().contains("projection_pushdown"), "{err}");
}

/// `build` returns the raw builder, which still offered `projection_pushdown`:
/// switched on after `build`, it reached `DuckDB`, which handed the scan a
/// one-column chunk for `SELECT b`, and the value the closure wrote as column
/// `a` came back as `b` (111 here). Registering such a builder is now refused.
#[test]
fn projection_pushdown_on_a_built_typed_table_function_is_refused() {
    let fx = Fixture::open();
    let builder = TableFunctionBuilder::new("tc_pushdown_after")
        .with_state(|bind| {
            bind.add_result_column("a", TypeId::BigInt);
            bind.add_result_column("b", TypeId::BigInt);
            Ok(false)
        })
        .scan(|done: &mut bool, chunk| {
            if *done {
                // SAFETY: end of stream.
                unsafe { chunk.set_size(0) };
                return Ok(());
            }
            // SAFETY: row 0 is in range of every column the chunk has, and
            // column 1 is written only when it exists.
            unsafe {
                chunk.writer(0).write_i64(0, 111);
                if chunk.column_count() > 1 {
                    chunk.writer(1).write_i64(0, 222);
                }
                chunk.set_size(1);
            }
            *done = true;
            Ok(())
        })
        .build()
        .expect("build")
        .projection_pushdown(true);
    // SAFETY: `con` is open.
    let err = unsafe { builder.register(fx.con()) }
        .expect_err("projection pushdown on a typed function must be refused");
    assert!(err.as_str().contains("projection_pushdown"), "{err}");
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM duckdb_functions() WHERE function_name = 'tc_pushdown_after'",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(0),
        "nothing is registered"
    );
}

/// TBL-9: a bind that declares no result columns made `DuckDB` raise an
/// `INTERNAL Error` ("Table function must return at least one column") with
/// a C++ stack trace. The typed bind now reports it as an ordinary bind error.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_typed_bind_that_declares_no_columns_is_an_ordinary_bind_error() {
    let fx = Fixture::open();
    let builder = TableFunctionBuilder::new("tc_no_columns")
        .with_state::<u8, _>(|_bind| Ok(0))
        .scan(|_state, chunk| {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register");
    let err = error_of(&fx, "SELECT * FROM tc_no_columns()");
    assert!(!err.contains("INTERNAL"), "{err}");
    assert!(err.contains("declared no result columns"), "{err}");
}

/// TBL-24: an empty bind error reached the user as `Binder Error: ` with no
/// text. Both the raw `BindInfo::set_error("")` and a typed bind returning
/// `Err("")` now report a placeholder.
#[test]
fn an_empty_table_function_error_message_is_replaced_by_a_placeholder() {
    unsafe extern "C" fn empty_bind(info: libduckdb_sys::duckdb_bind_info) {
        // SAFETY: `info` is the live bind info.
        let bind = unsafe { quack_rs::table::BindInfo::new(info) };
        bind.add_result_column("a", TypeId::BigInt);
        bind.set_error("");
    }
    unsafe extern "C" fn no_init(_info: libduckdb_sys::duckdb_init_info) {}
    unsafe extern "C" fn no_scan(
        _info: libduckdb_sys::duckdb_function_info,
        out: libduckdb_sys::duckdb_data_chunk,
    ) {
        // SAFETY: `out` is the live output chunk.
        unsafe { libduckdb_sys::duckdb_data_chunk_set_size(out, 0) };
    }

    let fx = Fixture::open();
    // SAFETY: `con` is open and the callbacks match their signatures.
    unsafe {
        TableFunctionBuilder::new("tc_empty_raw")
            .bind(empty_bind)
            .init(no_init)
            .scan(no_scan)
            .register(fx.con())
            .expect("register tc_empty_raw");
    }
    let typed = TableFunctionBuilder::new("tc_empty_typed")
        .with_state::<u8, _>(|bind| {
            bind.add_result_column("a", TypeId::BigInt);
            Err(quack_rs::error::ExtensionError::new(""))
        })
        .scan(|_state, _chunk| Ok(()))
        .build()
        .expect("build");
    // SAFETY: `con` is open.
    unsafe { typed.register(fx.con()) }.expect("register tc_empty_typed");
    for sql in [
        "SELECT * FROM tc_empty_raw()",
        "SELECT * FROM tc_empty_typed()",
    ] {
        let err = error_of(&fx, sql);
        assert!(err.contains("without a message"), "{sql}: {err}");
    }
}

/// TBL-20: `InitInfo::projected_column_index` returned 0 — "the first
/// declared column" — for a position past the projection (probe t03). It now
/// returns `None` there.
mod projected_column_index {
    use super::{query, Fixture, TableFunctionBuilder, TypeId};
    use quack_rs::table::{BindInfo, FfiInitData, InitInfo};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, PoisonError};

    static SEEN: Mutex<Vec<Vec<Option<usize>>>> = Mutex::new(Vec::new());

    unsafe extern "C" fn bind(info: libduckdb_sys::duckdb_bind_info) {
        // SAFETY: `info` is the live bind info.
        let bind = unsafe { BindInfo::new(info) };
        bind.add_result_column("a", TypeId::BigInt);
        bind.add_result_column("b", TypeId::BigInt);
        bind.add_result_column("c", TypeId::BigInt);
    }
    unsafe extern "C" fn init(info: libduckdb_sys::duckdb_init_info) {
        // SAFETY: `info` is the live init info.
        let init = unsafe { InitInfo::new(info) };
        let count = init.projected_column_count();
        SEEN.lock().unwrap_or_else(PoisonError::into_inner).push(
            (0..=count + 1)
                .map(|k| init.projected_column_index(k))
                .collect(),
        );
        // SAFETY: `info` is the live init info.
        unsafe { FfiInitData::<AtomicBool>::set(info, AtomicBool::new(false)) };
    }
    unsafe extern "C" fn scan(
        info: libduckdb_sys::duckdb_function_info,
        out: libduckdb_sys::duckdb_data_chunk,
    ) {
        // SAFETY: init data was set in `init`.
        let done = unsafe { FfiInitData::<AtomicBool>::get(info) };
        // SAFETY: `out` is the live output chunk.
        let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(out) };
        if done.is_none_or(|d| d.swap(true, Ordering::SeqCst)) {
            // SAFETY: end of stream.
            unsafe { chunk.set_size(0) };
            return;
        }
        for col in 0..chunk.column_count() {
            // SAFETY: every projected column is BIGINT and row 0 is in range.
            unsafe { chunk.writer(col).write_i64(0, 0) };
        }
        // SAFETY: one row was written.
        unsafe { chunk.set_size(1) };
    }

    #[test]
    fn an_out_of_range_projection_position_is_none() {
        let fx = Fixture::open();
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe {
            TableFunctionBuilder::new("tc_projected")
                .bind(bind)
                .init(init)
                .scan(scan)
                .projection_pushdown(true)
                .register(fx.con())
        }
        .expect("register");
        SEEN.lock().unwrap_or_else(PoisonError::into_inner).clear();
        // SAFETY: `con` is open.
        unsafe { query(fx.con(), "SELECT c, a FROM tc_projected()") }.expect("query");
        assert_eq!(
            *SEEN.lock().unwrap_or_else(PoisonError::into_inner),
            vec![vec![Some(2), Some(0), None, None]]
        );
    }
}

/// TBL-12: the documented shape of `CopyBindInfo::options` — upper-cased
/// names, SQL `NULL` when there are none, a `NULL` field for a valueless
/// option (probe t15).
#[cfg(feature = "duckdb-1-5")]
mod copy_options_shape {
    use super::Fixture;
    use quack_rs::copy_function::{CopyBindInfo, CopyFunctionBuilder};
    use std::sync::{Mutex, PoisonError};

    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

    quack_rs::copy_bind_callback!(record_options, |info| {
        // SAFETY: `info` is the live bind info.
        let bind = unsafe { CopyBindInfo::new(info) };
        let shape = match bind.options() {
            None => "no handle".to_owned(),
            Some(o) if o.is_sql_null() => "SQL NULL".to_owned(),
            Some(o) => {
                // DuckDB builds the STRUCT from a hash map: field order is
                // not specified, so sort.
                let mut fields = o
                    .struct_field_names()
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        let null = o.struct_child(i).is_none_or(|c| c.is_sql_null());
                        format!("{n}{}", if null { "=NULL" } else { "" })
                    })
                    .collect::<Vec<_>>();
                fields.sort();
                fields.join(",")
            }
        };
        SEEN.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(shape);
        bind.set_error("stop after bind");
    });
    quack_rs::copy_sink_callback!(no_sink, |_info, _chunk| {});
    quack_rs::copy_finalize_callback!(no_finalize, |_info| {});

    #[test]
    fn copy_options_arrive_upper_cased_and_null_when_absent() {
        let fx = Fixture::open();
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe {
            CopyFunctionBuilder::try_new("tc_opts")
                .expect("name")
                .bind(record_options)
                .sink(no_sink)
                .finalize(no_finalize)
                .register(fx.con())
        }
        .expect("register");
        for sql in [
            "COPY (SELECT 1) TO 'x' (FORMAT tc_opts)",
            "COPY (SELECT 1) TO 'x' (FORMAT tc_opts, compression 'zstd', header)",
        ] {
            let _ = super::error_of(&fx, sql);
        }
        assert_eq!(
            *SEEN.lock().unwrap_or_else(PoisonError::into_inner),
            vec!["SQL NULL".to_owned(), "COMPRESSION,HEADER=NULL".to_owned()]
        );
    }
}

/// TBL-5: after a panicking cast callback, `TRY_CAST` returned whatever the
/// output vector held (zeros or a previous chunk's values) instead of NULL:
/// `DuckDB` ignores a cast function's return value in TRY mode
/// (`execute_cast.cpp`), so only rows nulled explicitly become NULL.
mod try_cast_after_panic {
    use super::{Fixture, TypeId};
    use quack_rs::cast::CastFunctionBuilder;

    quack_rs::cast_callback!(tc_boom, |_info, _count, _input, _output| {
        panic!("tc_boom exploded")
    });

    #[test]
    fn a_panicking_cast_makes_every_try_cast_row_null() {
        let fx = Fixture::open();
        // SAFETY: `con` is open; the callback matches `CastFn`.
        unsafe {
            CastFunctionBuilder::new(TypeId::Blob, TypeId::Integer)
                .function(tc_boom)
                .register(fx.con())
        }
        .expect("register");
        assert_eq!(
            fx.scalar(
                "SELECT count(*) FROM (SELECT TRY_CAST(b AS INTEGER) AS v \
                 FROM (SELECT ('\\x0' || (i % 10)::VARCHAR)::BLOB AS b FROM range(3000) t(i))) \
                 WHERE v IS NULL",
                |r, i| unsafe { r.read_i64(i) }
            ),
            Some(3000),
            "every row of every chunk must be NULL"
        );
        let err = super::error_of(&fx, "SELECT CAST('\\x01'::BLOB AS INTEGER)");
        assert!(err.contains("tc_boom exploded"), "{err}");
    }
}

/// TBL-24: a cast that failed in normal mode with an empty message surfaced
/// as `Conversion Error: ` with no text.
mod cast_without_message {
    use super::{Fixture, TypeId};
    use quack_rs::cast::{CastFunctionBuilder, CastFunctionInfo};

    quack_rs::cast_callback!(tc_empty_message, |info, count, _input, output| {
        // SAFETY: `info` is the live cast info.
        let info = unsafe { CastFunctionInfo::new(info) };
        info.set_error("");
        for row in 0..count {
            // SAFETY: `output` is the live output vector and `row < count`.
            unsafe { info.set_row_error("", row, output) };
        }
        false
    });

    #[test]
    fn a_failed_cast_with_an_empty_message_still_says_something() {
        let fx = Fixture::open();
        // SAFETY: `con` is open; the callback matches `CastFn`.
        unsafe {
            CastFunctionBuilder::new(TypeId::Blob, TypeId::SmallInt)
                .function(tc_empty_message)
                .register(fx.con())
                .expect("register");
        }
        let err = super::error_of(&fx, "SELECT CAST('\\x01'::BLOB AS SMALLINT)");
        assert!(err.contains("without a message"), "{err}");
        // set_row_error nulls the rows it names, so TRY_CAST is NULL.
        assert_eq!(
            fx.scalar(
                "SELECT TRY_CAST('\\x01'::BLOB AS SMALLINT) IS NULL",
                |r, i| unsafe { r.read_bool(i) }
            ),
            Some(true)
        );
    }

    quack_rs::cast_callback!(tc_silent_false, |_info, _count, _input, _output| { false });

    quack_rs::cast_callback!(tc_own_message, |info, _count, _input, _output| {
        // SAFETY: `info` is the live cast info.
        unsafe { CastFunctionInfo::new(info) }.set_error("tc_own_message failed");
        false
    });

    /// TBL-24 remainder: a body that returns `false` without calling
    /// `set_error` at all also surfaced as `Conversion Error: ` with no text.
    /// A message the body does set must still win over the default.
    #[test]
    fn a_failed_cast_that_reports_nothing_still_says_something() {
        let fx = Fixture::open();
        // SAFETY: `con` is open; the callbacks match `CastFn`.
        unsafe {
            CastFunctionBuilder::new(TypeId::Blob, TypeId::SmallInt)
                .function(tc_silent_false)
                .register(fx.con())
                .expect("register");
            CastFunctionBuilder::new(TypeId::Blob, TypeId::TinyInt)
                .function(tc_own_message)
                .register(fx.con())
                .expect("register");
        }
        let err = super::error_of(&fx, "SELECT CAST('\\x01'::BLOB AS SMALLINT)");
        assert!(err.contains("without reporting an error message"), "{err}");
        let err = super::error_of(&fx, "SELECT CAST('\\x01'::BLOB AS TINYINT)");
        assert!(err.contains("tc_own_message failed"), "{err}");
        assert!(!err.contains("without reporting"), "{err}");
    }
}

/// A typed table function with one BIGINT column `v` that emits `value` once.
fn one_row_function(name: &str, value: i64) -> TableFunctionBuilder {
    TableFunctionBuilder::new(name)
        .with_state::<bool, _>(|bind| {
            bind.add_result_column("v", TypeId::BigInt);
            Ok(false)
        })
        .scan(move |done, chunk| {
            if *done {
                // SAFETY: end of stream.
                unsafe { chunk.set_size(0) };
            } else {
                *done = true;
                // SAFETY: column 0 is BIGINT and row 0 is in range.
                unsafe {
                    chunk.writer(0).write_i64(0, value);
                    chunk.set_size(1);
                }
            }
            Ok(())
        })
        .build()
        .expect("build one_row_function")
}

/// TBL-7: registering a second table function under a taken name returned
/// `Ok` and was silently dropped (`ALTER_ON_CONFLICT` + `AddEntry` returns
/// `nullptr` for table functions), so the first function kept answering —
/// including when the "taken name" was a built-in such as `range`.
#[test]
fn registering_a_taken_table_function_name_is_an_error() {
    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe { one_row_function("tc_taken", 1).register(fx.con()) }.expect("first");
    // SAFETY: `con` is open.
    let err = unsafe { one_row_function("tc_taken", 2).register(fx.con()) }
        .expect_err("a second registration under the same name must fail");
    assert!(err.as_str().contains("already exists"), "{err}");
    // The first registration still answers.
    assert_eq!(
        fx.scalar("SELECT v FROM tc_taken()", |r, i| unsafe { r.read_i64(i) }),
        Some(1)
    );
    // Names are case-insensitive, and built-ins count.
    for taken in ["TC_TAKEN", "range", "read_csv"] {
        // SAFETY: `con` is open.
        let err = unsafe { one_row_function(taken, 3).register(fx.con()) }
            .expect_err("a taken name must fail");
        assert!(err.as_str().contains("already exists"), "{taken}: {err}");
    }
    assert_eq!(
        fx.scalar("SELECT count(*) FROM range(5)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(5)
    );
}

/// TBL-7: the same silent drop for copy functions — a second `tc_fmt`, or a
/// format named `csv`, returned `Ok` and was never called.
#[cfg(feature = "duckdb-1-5")]
mod taken_copy_function_name {
    use super::Fixture;
    use quack_rs::copy_function::CopyFunctionBuilder;

    quack_rs::copy_bind_callback!(tc_copy_bind, |_info| {});
    quack_rs::copy_sink_callback!(tc_copy_sink, |_info, _chunk| {});
    quack_rs::copy_finalize_callback!(tc_copy_finalize, |_info| {});

    fn register(fx: &Fixture, name: &str) -> Result<(), quack_rs::error::ExtensionError> {
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe {
            CopyFunctionBuilder::try_new(name)?
                .bind(tc_copy_bind)
                .sink(tc_copy_sink)
                .finalize(tc_copy_finalize)
                .register(fx.con())
        }
    }

    #[test]
    fn registering_a_taken_copy_function_name_is_an_error() {
        let fx = Fixture::open();
        register(&fx, "tc_fmt").expect("first registration");
        for taken in ["tc_fmt", "TC_FMT", "csv", "parquet"] {
            let err = register(&fx, taken).expect_err("a taken format name must fail");
            assert!(err.as_str().contains("already exists"), "{taken}: {err}");
        }
    }
}

/// TBL-7, the premise of refusing a taken name: table functions registered
/// through the C API live in the in-memory system catalog and never reach a
/// database file, so the next session starts without them and the extension
/// can register them again.
#[test]
fn a_registered_table_function_does_not_persist_into_a_database_file() {
    let _dispatch = quack_rs::testing::InMemoryDb::open().expect("dispatch table");
    let path =
        std::env::temp_dir().join(format!("quack_rs_tc_persist_{}.duckdb", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let c_path = std::ffi::CString::new(path.to_str().expect("utf-8 path")).expect("no NUL");
    let count_registered = |con: libduckdb_sys::duckdb_connection| {
        // SAFETY: `con` is open.
        let mut result = unsafe {
            query(
                con,
                "SELECT count(*) FROM duckdb_functions() WHERE function_name = 'tc_persist'",
            )
        }
        .expect("query");
        let chunk = result.next_chunk().expect("fetch").expect("a row");
        // SAFETY: one BIGINT row.
        unsafe { chunk.reader(0).read_i64(0) }
    };
    for session in 0..2 {
        let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
        let mut con: libduckdb_sys::duckdb_connection = std::ptr::null_mut();
        // SAFETY: standard open/connect of a file database; closed below.
        unsafe {
            assert_eq!(libduckdb_sys::duckdb_open(c_path.as_ptr(), &raw mut db), 0);
            assert_eq!(libduckdb_sys::duckdb_connect(db, &raw mut con), 0);
        }
        assert_eq!(
            count_registered(con),
            0,
            "session {session} starts without it"
        );
        // SAFETY: `con` is open.
        unsafe { one_row_function("tc_persist", 1).register(con) }
            .unwrap_or_else(|e| panic!("session {session}: {e}"));
        assert_eq!(count_registered(con), 1);
        // SAFETY: both handles were opened above and are closed once.
        unsafe {
            libduckdb_sys::duckdb_disconnect(&raw mut con);
            libduckdb_sys::duckdb_close(&raw mut db);
        }
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("duckdb.wal"));
}

/// The fourth audit's T1. A `CatalogEntry` held `DuckDB`'s reference to the
/// entry, valid only while the transaction that found it lasted, yet its
/// methods were safe. After `COMMIT`, `DROP TABLE` and a new `CREATE TABLE`,
/// `name()` returned the new table's name (valgrind: an invalid read in
/// `duckdb_catalog_entry_get_name`). The entry now copies its name and type
/// at lookup, so they stay what they were.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_catalog_entry_keeps_its_name_after_the_table_is_dropped() {
    use quack_rs::catalog::CatalogEntryType;
    use quack_rs::client_context::ClientContext;

    let fx = Fixture::open();
    let exec = |sql: &str| {
        // SAFETY: `con` is open.
        unsafe { quack_rs::query::query(fx.con(), sql) }.unwrap_or_else(|e| panic!("{sql}: {e}"));
    };
    exec("CREATE TABLE a_rather_long_table_name_to_spot(i INT)");
    exec("BEGIN");
    // SAFETY: `con` is open.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("context");
    // SAFETY: an explicit transaction is active.
    let catalog = unsafe { ctx.catalog(c"memory") }.expect("catalog");
    // SAFETY: as above; the catalog's database stays attached.
    let entry = unsafe {
        catalog.get_entry(
            ctx.as_raw(),
            c"main",
            c"a_rather_long_table_name_to_spot",
            CatalogEntryType::Table,
        )
    }
    .expect("lookup")
    .expect("the table exists");
    exec("COMMIT");
    exec("DROP TABLE a_rather_long_table_name_to_spot");
    exec("CREATE TABLE filler AS SELECT repeat('Z', 200) AS s FROM range(100)");
    assert_eq!(entry.name(), Some("a_rather_long_table_name_to_spot"));
    assert_eq!(entry.entry_type(), CatalogEntryType::Table);
}

/// The fourth audit's F8, pinned so the cast docs stay true: registering a
/// cast for a pair `DuckDB` already casts replaces the built-in one for the
/// whole database — every connection, and the implicit cast an `INSERT`
/// performs — and registering it again replaces it again, with no error.
mod cast_replaces_the_builtin {
    use super::{Fixture, TypeId};
    use quack_rs::cast::CastFunctionBuilder;
    use quack_rs::query::OwnedConnection;
    use quack_rs::vector::{VectorReader, VectorWriter};

    // A strict VARCHAR -> INTEGER cast: digits only, no whitespace.
    quack_rs::cast_callback!(strict_int, |_info, count, input, output| {
        let rows = usize::try_from(count).unwrap_or(0);
        let reader = unsafe { VectorReader::from_vector(input, rows) };
        let mut writer = unsafe { VectorWriter::new(output) };
        let mut ok = true;
        for row in 0..rows {
            let parsed = unsafe { reader.is_valid(row).then(|| reader.read_str(row)) }.map(|s| {
                s.bytes()
                    .all(|b| b.is_ascii_digit())
                    .then(|| s.parse::<i32>().ok())
                    .flatten()
            });
            match parsed {
                Some(Some(v)) => unsafe { writer.write_i32(row, v) },
                Some(None) => {
                    ok = false;
                    unsafe { writer.set_null(row) };
                }
                None => unsafe { writer.set_null(row) },
            }
        }
        ok
    });

    #[test]
    fn a_varchar_to_integer_cast_replaces_the_builtin_for_the_whole_database() {
        let fx = Fixture::open();
        assert_eq!(
            fx.scalar("SELECT ' 7'::INTEGER", |r, i| unsafe { r.read_i32(i) }),
            Some(7),
            "the built-in cast trims whitespace"
        );
        // SAFETY: `con` is open; the callback matches `CastFn`.
        let register = || unsafe {
            CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
                .function(strict_int)
                .register(fx.con())
        };
        register().expect("register");
        register().expect("a second registration is not refused either");
        assert!(super::error_of(&fx, "SELECT ' 7'::INTEGER").contains("Conversion Error"));
        // SAFETY: the fixture's database outlives the connection.
        let other = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
        assert!(
            other.query("SELECT ' 7'::INTEGER").is_err(),
            "another connection"
        );
        fx.query("CREATE TABLE cast_target (i INTEGER)");
        assert!(
            super::error_of(&fx, "INSERT INTO cast_target VALUES (' 7')")
                .contains("Conversion Error"),
            "an INSERT's implicit cast"
        );
    }
}
