// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The entry point runs the registration closure in one transaction
//! (`Registration::Transactional`, the default): a `LOAD` that fails part-way
//! leaves no catalog entry behind and can be retried, a successful one commits
//! for every connection, and a `LOAD` inside the host's own transaction
//! neither waits on it nor depends on how it ends. Driven through the
//! in-process loader of `entry_point_inprocess`.

use std::sync::mpsc;
use std::time::Duration;

use quack_rs::connection::{Connection, Registrar};
use quack_rs::entry_point::{init_extension, init_extension_v2};
use quack_rs::error::ExtensionError;
use quack_rs::sql_macro::SqlMacro;

use super::entry_point_inprocess::{access, answer_fn, call, Loader};
use super::Fixture;

fn count(fx: &Fixture, sql: &str) -> i64 {
    // SAFETY: every query here returns one BIGINT.
    fx.scalar(sql, |r, i| unsafe { r.read_i64(i) })
        .unwrap_or_else(|| panic!("{sql} returned NULL"))
}

/// Functions, macros and tables in the database whose name starts with
/// `prefix`.
fn leftovers(fx: &Fixture, prefix: &str) -> i64 {
    count(
        fx,
        &format!(
            "SELECT (SELECT count(*) FROM duckdb_functions() \
                     WHERE function_name LIKE '{prefix}%') \
                  + (SELECT count(*) FROM duckdb_tables() WHERE table_name LIKE '{prefix}%')"
        ),
    )
}

/// A scalar function, a scalar macro, a table macro and a table, then a
/// failure if `fail`.
///
/// # Safety
///
/// `con` wraps a live connection.
unsafe fn register_four(con: &Connection, prefix: &str, fail: bool) -> Result<(), ExtensionError> {
    // SAFETY: per this function's contract.
    unsafe {
        con.register_scalar(answer_fn(&format!("{prefix}_scalar")))?;
        con.register_sql_macro(SqlMacro::scalar(
            &format!("{prefix}_macro"),
            &["x"],
            "x + 1",
        )?)?;
        con.register_sql_macro(SqlMacro::table(
            &format!("{prefix}_tmacro"),
            &["n"],
            "SELECT * FROM range(n)",
        )?)?;
        con.execute(&format!("CREATE TABLE {prefix}_table AS SELECT 7 AS i"))?;
    }
    if fail {
        return Err(ExtensionError::new("refused: configuration is missing"));
    }
    Ok(())
}

/// What `register_four` created, read from the fixture's own connection — a
/// different one from the registration connection, so this also shows the
/// commit is visible to the rest of the database.
fn assert_all_four(fx: &Fixture, prefix: &str) {
    assert_eq!(call(fx, &format!("{prefix}_scalar")), Some(42));
    assert_eq!(count(fx, &format!("SELECT {prefix}_macro(1)::BIGINT")), 2);
    assert_eq!(
        count(fx, &format!("SELECT count(*) FROM {prefix}_tmacro(3)")),
        3
    );
    assert_eq!(
        count(fx, &format!("SELECT i::BIGINT FROM {prefix}_table")),
        7
    );
}

#[test]
fn a_failed_load_leaves_nothing_behind_and_can_be_retried() {
    let fx = Fixture::open();
    let access = access();
    let loader = Loader::serving(fx.db());
    // SAFETY: `loader` and `access` outlive the call and are what an entry
    // point is given.
    let accepted = unsafe {
        init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
            register_four(con, "ep_txn", true)
        })
    };
    assert!(!accepted);
    assert_eq!(
        loader.error().as_deref(),
        Some("refused: configuration is missing")
    );
    assert_eq!(
        leftovers(&fx, "ep_txn"),
        0,
        "the failed LOAD left entries behind"
    );

    // The retry registers the same names, which a partial first LOAD would
    // have made it refuse.
    let loader = Loader::serving(fx.db());
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
            register_four(con, "ep_txn", false)
        })
    };
    assert!(accepted, "retry: {:?}", loader.error());
    assert_eq!(loader.error(), None);
    assert_eq!(leftovers(&fx, "ep_txn"), 4);
    assert_all_four(&fx, "ep_txn");
}

#[test]
fn a_failed_load_through_the_raw_entry_point_or_a_panic_rolls_back_too() {
    let fx = Fixture::open();
    let access = access();
    let loader = Loader::serving(fx.db());
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension(loader.info(), &raw const access, "v1.2.0", |con| {
            answer_fn("ep_txn_raw_scalar").register(con)?;
            Err(ExtensionError::new("raw closure refused"))
        })
    };
    assert!(!accepted);
    assert_eq!(loader.error().as_deref(), Some("raw closure refused"));
    assert_eq!(leftovers(&fx, "ep_txn_raw"), 0);

    let loader = Loader::serving(fx.db());
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
            register_four(con, "ep_txn_panic", false)?;
            panic!("boom after registering")
        })
    };
    assert!(!accepted);
    assert!(
        loader
            .error()
            .is_some_and(|e| e.contains("boom after registering")),
        "{:?}",
        loader.error()
    );
    assert_eq!(leftovers(&fx, "ep_txn_panic"), 0);

    let loader = Loader::serving(fx.db());
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension(loader.info(), &raw const access, "v1.2.0", |con| {
            answer_fn("ep_txn_raw_scalar").register(con)
        })
    };
    assert!(accepted, "retry: {:?}", loader.error());
    assert_eq!(call(&fx, "ep_txn_raw_scalar"), Some(42));
}

/// Until the `LOAD` commits, what the closure created is visible on the
/// registration connection only — not to a connection the closure opens
/// itself, nor to the rest of the database.
#[test]
fn what_the_closure_creates_is_visible_elsewhere_only_once_the_load_commits() {
    let fx = Fixture::open();
    let access = access();
    let loader = Loader::serving(fx.db());
    let tables = "SELECT count(*) FROM duckdb_tables() WHERE table_name = 'ep_txn_seen_table'";
    let mut seen = None;
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
            con.execute("CREATE TABLE ep_txn_seen_table AS SELECT 1 AS i")?;
            let own = con
                .query(tables)?
                .next_chunk()?
                .map(|c| c.reader(0).read_i64(0));
            let other = con.open_connection()?;
            let theirs = other
                .query(tables)?
                .next_chunk()?
                .map(|c| c.reader(0).read_i64(0));
            seen = Some((own, theirs));
            Ok(())
        })
    };
    assert!(accepted, "{:?}", loader.error());
    assert_eq!(
        seen,
        Some((Some(1), Some(0))),
        "(registration connection, another one)"
    );
    assert_eq!(count(&fx, tables), 1, "committed with the LOAD");
}

/// Fails the test binary instead of letting it hang, should a `LOAD` ever wait
/// on the host's transaction.
struct Watchdog(Option<mpsc::Sender<()>>);

impl Watchdog {
    fn arm(what: &'static str) -> Self {
        let (done, wait) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            if wait.recv_timeout(Duration::from_secs(60)) == Err(mpsc::RecvTimeoutError::Timeout) {
                eprintln!("{what}: still waiting after 60 s, aborting");
                std::process::abort();
            }
        });
        Self(Some(done))
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            let _ = done.send(());
        }
    }
}

/// `LOAD` runs on the host's connection, possibly inside the host's
/// transaction; the registration transaction is a separate one on its own
/// connection, so it commits without waiting for the host's to end, and the
/// host rolling back does not undo it.
#[test]
fn a_load_inside_the_hosts_open_transaction_commits_independently() {
    let fx = Fixture::open();
    let access = access();
    for (round, host_ends_with) in ["COMMIT", "ROLLBACK"].into_iter().enumerate() {
        let prefix = format!("ep_txn_host{round}");
        fx.query("BEGIN TRANSACTION");
        // The host's transaction has written to the database and read the
        // system catalog before the LOAD.
        fx.query(&format!("CREATE TABLE {prefix}_hosts(i INTEGER)"));
        assert_eq!(count(&fx, "SELECT abs(-3)::BIGINT"), 3);
        let loader = Loader::serving(fx.db());
        let accepted = {
            let _watchdog = Watchdog::arm("LOAD inside the host's transaction");
            // SAFETY: as above.
            unsafe {
                init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
                    register_four(con, &prefix, false)
                })
            }
        };
        assert!(accepted, "{host_ends_with}: {:?}", loader.error());
        // The host's transaction is unaffected and still usable.
        assert_eq!(
            count(&fx, &format!("SELECT count(*) FROM {prefix}_hosts")),
            0
        );
        fx.query(host_ends_with);
        assert_all_four(&fx, &prefix);
        let hosts = count(
            &fx,
            &format!("SELECT count(*) FROM duckdb_tables() WHERE table_name = '{prefix}_hosts'"),
        );
        assert_eq!(
            hosts,
            i64::from(host_ends_with == "COMMIT"),
            "{host_ends_with}"
        );
    }
}

/// Registering a copy function runs a probe that fails on purpose; on
/// `DuckDB` 2.0 a failed statement aborts the transaction it runs in, so
/// inside the registration transaction the probe runs on a connection of its
/// own. A name the same transaction already registered — which that
/// connection cannot see — is still refused, and a rolled-back copy function
/// can be registered again.
#[cfg(feature = "duckdb-1-5")]
mod copy {
    use quack_rs::connection::Registrar;
    use quack_rs::copy_function::CopyFunctionBuilder;
    use quack_rs::entry_point::{init_extension, init_extension_v2};
    use quack_rs::error::ExtensionError;

    use super::{access, count, Fixture, Loader};

    quack_rs::copy_bind_callback!(txn_bind, |info| {
        let _ = info;
    });
    quack_rs::copy_sink_callback!(txn_sink, |info, chunk| {
        let _ = (info, chunk);
    });
    quack_rs::copy_finalize_callback!(txn_finalize, |info| {
        let _ = info;
    });

    fn format(name: &str) -> CopyFunctionBuilder {
        CopyFunctionBuilder::try_new(name)
            .expect("a valid name")
            .bind(txn_bind)
            .sink(txn_sink)
            .finalize(txn_finalize)
    }

    /// Whether `COPY … (FORMAT name)` resolves on the fixture's connection:
    /// the statement always fails, at the missing column if the format
    /// exists and at the format ("does not exist") if not.
    fn resolves(fx: &Fixture, name: &str) -> bool {
        let sql = format!("COPY (SELECT \"ep_txn_no_such_column\") TO '' (FORMAT '{name}')");
        // SAFETY: the fixture's connection is open.
        let error = unsafe { quack_rs::query::query(fx.con(), &sql) }
            .err()
            .expect("the probe statement always fails");
        !error.as_str().contains("does not exist")
    }

    #[test]
    fn copy_functions_register_inside_the_registration_transaction() {
        let fx = Fixture::open();
        let access = access();
        assert!(!resolves(&fx, "ep_txn_fmt"));

        // Registered once, then again in the same transaction: the second is
        // refused from the transaction's own record, and the failed LOAD
        // rolls the first back.
        let loader = Loader::serving(fx.db());
        // SAFETY: as in `entry_point_txn`'s other tests.
        let accepted = unsafe {
            init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
                con.register_copy_function(format("ep_txn_fmt"))?;
                con.register_copy_function(format("EP_TXN_FMT"))
            })
        };
        assert!(!accepted);
        let error = loader.error().unwrap_or_default();
        assert!(
            error.contains("copy function 'EP_TXN_FMT' already exists"),
            "{error}"
        );
        assert!(
            !resolves(&fx, "ep_txn_fmt"),
            "the failed LOAD left the format"
        );

        // Once, through each entry point, in a LOAD that commits.
        for (name, raw) in [("ep_txn_fmt", false), ("ep_txn_raw_fmt", true)] {
            let loader = Loader::serving(fx.db());
            // SAFETY: as above.
            let accepted = unsafe {
                if raw {
                    init_extension(loader.info(), &raw const access, "v1.2.0", |con| {
                        format(name).register(con)
                    })
                } else {
                    init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
                        con.register_copy_function(format(name))
                    })
                }
            };
            assert!(accepted, "{name}: {:?}", loader.error());
            assert!(resolves(&fx, name), "{name}");
        }
        // The probe still sees what earlier LOADs committed.
        let loader = Loader::serving(fx.db());
        // SAFETY: as above.
        let accepted = unsafe {
            init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
                con.register_copy_function(format("ep_txn_fmt"))
                    .and_then(|()| Err::<(), _>(ExtensionError::new("not reached")))
            })
        };
        assert!(!accepted);
        let error = loader.error().unwrap_or_default();
        assert!(
            error.contains("copy function 'ep_txn_fmt' already exists"),
            "{error}"
        );
        assert_eq!(
            count(&fx, "SELECT 1::BIGINT"),
            1,
            "the fixture's connection still works"
        );
    }
}
