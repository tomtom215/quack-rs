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
/// now converted by SQL and handed over already typed, so it registers, and
/// every supported type stores exactly what `TRY_CAST` produces.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_config_option_default_is_converted_by_sql_not_inside_the_c_api() {
    use quack_rs::config_option::ConfigOptionBuilder;

    let fx = Fixture::open();
    assert_eq!(
        fx.scalar(
            "SELECT loaded FROM duckdb_extensions() WHERE extension_name = 'icu'",
            |r, i| unsafe { r.read_bool(i) }
        ),
        Some(true),
        "this regression needs ICU's VARCHAR -> TIMESTAMPTZ cast"
    );
    for (i, (ty, default)) in [
        (TypeId::TimestampTz, "2020-01-01 10:00:00 America/New_York"),
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
