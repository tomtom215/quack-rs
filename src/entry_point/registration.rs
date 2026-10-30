// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! How the entry point runs the registration closure: in one transaction on
//! the registration connection, or with every statement committing on its own.
//! See "Registration is transactional" in the [`entry_point`](super) docs.

use libduckdb_sys::{duckdb_connection, duckdb_database};

use crate::abi::AbiPolicy;
use crate::error::ExtensionError;

/// How the entry point runs the registration closure.
///
/// `#[non_exhaustive]`: a `match` outside quack-rs needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Registration {
    /// Run the closure inside `BEGIN TRANSACTION` on the registration
    /// connection, `COMMIT` when it returns `Ok`, and `ROLLBACK` when it
    /// returns `Err` or panics. A failed `LOAD` then leaves no catalog entry
    /// behind (scalar, aggregate and table functions, macros, types, copy
    /// functions, tables) and can be retried; casts, replacement scans and
    /// configuration options are not catalog entries and stay registered. The
    /// default.
    #[default]
    Transactional,
    /// Let every registration and statement commit on its own, as quack-rs did
    /// before 0.20: what the closure registered before a failure stays
    /// registered, and retrying the `LOAD` in the same database fails at the
    /// first name it registers again. Use it when the closure must keep going
    /// after a failed statement, or must control transactions on the
    /// registration connection itself.
    Autocommit,
}

/// What an entry point is configured with: the [`AbiPolicy`] and the
/// [`Registration`] mode.
///
/// The middle argument of [`entry_point!`][crate::entry_point!] and
/// [`entry_point_v2!`][crate::entry_point_v2!] is either an [`AbiPolicy`],
/// which keeps the default registration mode, or `options = ` one of these;
/// [`init_extension_with_options`][super::init_extension_with_options] and
/// [`init_extension_v2_with_options`][super::init_extension_v2_with_options]
/// take one for an entry point written by hand.
///
/// ```rust,no_run
/// use quack_rs::abi::AbiPolicy;
/// use quack_rs::entry_point::{EntryPointOptions, Registration};
///
/// quack_rs::entry_point_v2!(
///     my_ext_init_c_api,
///     options = EntryPointOptions::new()
///         .abi_policy(AbiPolicy::Strict)
///         .registration(Registration::Autocommit),
///     |_con| Ok(())
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct EntryPointOptions {
    abi_policy: AbiPolicy,
    registration: Registration,
}

impl EntryPointOptions {
    /// The defaults: [`AbiPolicy::Strict`] and [`Registration::Transactional`].
    pub const fn new() -> Self {
        Self {
            abi_policy: AbiPolicy::Strict,
            registration: Registration::Transactional,
        }
    }

    /// Sets the [`AbiPolicy`].
    pub const fn abi_policy(mut self, policy: AbiPolicy) -> Self {
        self.abi_policy = policy;
        self
    }

    /// Sets the [`Registration`] mode.
    pub const fn registration(mut self, registration: Registration) -> Self {
        self.registration = registration;
        self
    }

    /// The configured [`AbiPolicy`].
    #[must_use]
    pub const fn get_abi_policy(&self) -> AbiPolicy {
        self.abi_policy
    }

    /// The configured [`Registration`] mode.
    #[must_use]
    pub const fn get_registration(&self) -> Registration {
        self.registration
    }
}

impl From<AbiPolicy> for EntryPointOptions {
    /// `policy`, with the default [`Registration`] mode.
    fn from(policy: AbiPolicy) -> Self {
        Self::new().abi_policy(policy)
    }
}

/// Runs `register` on the registration connection `con` of database `db` as
/// `mode` says, with a panic in it turned into an error.
///
/// # Errors
///
/// The closure's error; or, in [`Registration::Transactional`], `DuckDB`'s
/// error when the transaction cannot begin or commit.
///
/// # Safety
///
/// `con` must be an open connection to `db` that nothing else uses during the
/// call, and `db` must stay valid for it.
pub(super) unsafe fn run<F>(
    con: duckdb_connection,
    db: duckdb_database,
    mode: Registration,
    register: F,
) -> Result<(), ExtensionError>
where
    F: FnOnce() -> Result<(), ExtensionError>,
{
    // Only the copy-format check needs the database (see `ScopeState`).
    #[cfg(not(feature = "duckdb-1-5"))]
    let _ = db;
    if mode == Registration::Autocommit {
        return super::catch_registration_panic(register);
    }
    // SAFETY: `con` is open per this function's contract.
    unsafe { crate::query::execute(con, "BEGIN TRANSACTION") }.map_err(|e| {
        ExtensionError::new(format!(
            "could not begin the transaction the extension registers in: {e}"
        ))
    })?;
    let result = {
        #[cfg(feature = "duckdb-1-5")]
        let _scope = Scope::enter(con, db);
        super::catch_registration_panic(register)
    };
    match result {
        // SAFETY: as above.
        Ok(()) => unsafe { crate::query::execute(con, "COMMIT") }
            .map(drop)
            .map_err(|e| {
                ExtensionError::new(format!(
                    "the extension's registrations could not be committed, so none of them took \
                     effect: {e}"
                ))
            }),
        Err(error) => {
            // A failed rollback leaves the transaction open; disconnecting
            // the connection, which the caller does next, rolls it back.
            // SAFETY: as above.
            let _ = unsafe { crate::query::execute(con, "ROLLBACK") };
            Err(error)
        }
    }
}

/// The registration transaction the entry point has open on this thread.
///
/// Only the copy-format check needs it (with `duckdb-1-5`): it probes with a
/// statement that fails on purpose, and on `DuckDB` 2.0 any failed statement
/// aborts the transaction it runs in, so inside this one it must run on a
/// connection of its own — which needs the database — and must be told the
/// formats registered in the still-uncommitted transaction, which that
/// connection cannot see.
#[cfg(feature = "duckdb-1-5")]
struct ScopeState {
    con: duckdb_connection,
    db: duckdb_database,
    copy_formats: Vec<String>,
}

#[cfg(feature = "duckdb-1-5")]
std::thread_local! {
    static SCOPE: core::cell::RefCell<Option<ScopeState>> =
        const { core::cell::RefCell::new(None) };
}

/// Marks a registration transaction as open on this thread until dropped,
/// restoring whatever was marked before (a nested `LOAD` of another extension
/// built with this same copy of quack-rs).
#[cfg(feature = "duckdb-1-5")]
struct Scope {
    previous: Option<ScopeState>,
}

#[cfg(feature = "duckdb-1-5")]
impl Scope {
    fn enter(con: duckdb_connection, db: duckdb_database) -> Self {
        let state = ScopeState {
            con,
            db,
            copy_formats: Vec::new(),
        };
        let previous = SCOPE.with(|scope| scope.replace(Some(state)));
        Self { previous }
    }
}

#[cfg(feature = "duckdb-1-5")]
impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        SCOPE.with(|scope| {
            scope.replace(previous);
        });
    }
}

/// When `con` is the connection of the registration transaction open on this
/// thread: its database, and whether a copy format named `name` (compared
/// case-insensitively) was registered in that transaction.
#[cfg(feature = "duckdb-1-5")]
pub fn copy_format_scope(con: duckdb_connection, name: &str) -> Option<(duckdb_database, bool)> {
    SCOPE.with(|scope| {
        scope.borrow().as_ref().filter(|s| s.con == con).map(|s| {
            let taken = s.copy_formats.iter().any(|f| f.eq_ignore_ascii_case(name));
            (s.db, taken)
        })
    })
}

/// Records a copy format registered on `con`, if `con` is the connection of
/// the registration transaction open on this thread.
#[cfg(feature = "duckdb-1-5")]
pub fn record_copy_format(con: duckdb_connection, name: &str) {
    SCOPE.with(|scope| {
        if let Some(s) = scope.borrow_mut().as_mut().filter(|s| s.con == con) {
            s.copy_formats.push(name.to_owned());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{EntryPointOptions, Registration};
    use crate::abi::AbiPolicy;

    #[test]
    fn options_default_to_strict_and_transactional() {
        let options = EntryPointOptions::default();
        assert_eq!(options, EntryPointOptions::new());
        assert_eq!(options.get_abi_policy(), AbiPolicy::Strict);
        assert_eq!(options.get_registration(), Registration::Transactional);
        assert_eq!(Registration::default(), Registration::Transactional);
    }

    #[test]
    fn a_policy_converts_to_options_with_the_default_registration() {
        let options = EntryPointOptions::from(AbiPolicy::Trust);
        assert_eq!(options.get_abi_policy(), AbiPolicy::Trust);
        assert_eq!(options.get_registration(), Registration::Transactional);
        let options = options.registration(Registration::Autocommit);
        assert_eq!(options.get_abi_policy(), AbiPolicy::Trust);
        assert_eq!(options.get_registration(), Registration::Autocommit);
    }

    /// The scope names only its own connection, and a nested scope restores
    /// the outer one when it ends.
    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn the_scope_tracks_one_connection_and_nests() {
        use super::{copy_format_scope, record_copy_format, Scope};
        let (outer_con, inner_con) = (0x10 as _, 0x20 as _);
        let (outer_db, inner_db) = (0x11 as _, 0x21 as _);
        assert_eq!(copy_format_scope(outer_con, "fmt"), None);
        {
            let _outer = Scope::enter(outer_con, outer_db);
            record_copy_format(outer_con, "MyFmt");
            record_copy_format(inner_con, "elsewhere");
            assert_eq!(
                copy_format_scope(outer_con, "myfmt"),
                Some((outer_db, true))
            );
            assert_eq!(
                copy_format_scope(outer_con, "elsewhere"),
                Some((outer_db, false))
            );
            assert_eq!(copy_format_scope(inner_con, "myfmt"), None);
            {
                let _inner = Scope::enter(inner_con, inner_db);
                assert_eq!(
                    copy_format_scope(inner_con, "myfmt"),
                    Some((inner_db, false))
                );
                assert_eq!(copy_format_scope(outer_con, "myfmt"), None);
            }
            assert_eq!(
                copy_format_scope(outer_con, "myfmt"),
                Some((outer_db, true))
            );
        }
        assert_eq!(copy_format_scope(outer_con, "myfmt"), None);
    }
}
