// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Extension entry point helper.
//!
//! Provides [`init_extension`], the core helper called by the `entry_point!` macro.
//!
//! # Problem 1: Custom C entry point
//!
//! `DuckDB`'s Rust crate does not provide a safe way to obtain a raw
//! `duckdb_connection` handle for function registration. The prior approach
//! using `extract_raw_connection` relied on `Rc<RefCell<InnerConnection>>` layout,
//! causing SEGFAULTs. The correct approach is a hand-written C entry point that:
//!
//! 1. Calls `duckdb_rs_extension_api_init(info, access, "v1.2.0")`
//! 2. Calls `access.get_database(info)` to get a `duckdb_database`
//! 3. Calls `duckdb_connect(db, &mut raw_con)` to get a `duckdb_connection`
//! 4. Registers all functions
//! 5. Calls `duckdb_disconnect(&mut raw_con)`
//!
//! # Pitfall L3: No panic across FFI
//!
//! `init_extension` uses `Result` for all error propagation and never calls
//! `unwrap()` or `panic!()` inside an FFI callback.
//!
//! # Registration is not transactional
//!
//! Each `register` call commits to `DuckDB`'s catalog on its own; there is no
//! enclosing transaction the entry point could roll back. If the registration
//! closure registers some functions and then returns `Err` (or panics), the
//! `LOAD` fails — but the functions registered before the failure **stay
//! registered** and callable for the life of the database, while `DuckDB`
//! does not list the extension as loaded. Retrying the `LOAD` in the same
//! process then fails at the first function it re-registers: `DuckDB`
//! refuses an aggregate name registered twice, and quack-rs refuses a scalar
//! signature that already exists.
//!
//! So do every fallible thing that does not register — reading
//! configuration, validating settings, building lookup tables — **before**
//! the first `register` call, and treat an error after it as leaving the
//! database partially extended.
//!
//! # Usage
//!
//! Extension authors typically use the `entry_point!` macro,
//! which generates the required `#[no_mangle] extern "C"` function automatically.
//!
//! If you need full control over the entry point, you can call `init_extension`
//! directly:
//!
//! ```rust,no_run
//! use quack_rs::entry_point::init_extension;
//!
//! #[no_mangle]
//! pub unsafe extern "C" fn my_extension_init_c_api(
//!     info: libduckdb_sys::duckdb_extension_info,
//!     access: *const libduckdb_sys::duckdb_extension_access,
//! ) -> bool {
//!     unsafe {
//!         init_extension(info, access, quack_rs::DUCKDB_API_VERSION, |con| {
//!             // register functions with `con: libduckdb_sys::duckdb_connection`
//!             Ok(())
//!         })
//!     }
//! }
//! ```

use std::io::Write as _;

use libduckdb_sys::{
    duckdb_connect, duckdb_connection, duckdb_disconnect, duckdb_extension_access,
    duckdb_extension_info, duckdb_rs_extension_api_init, DuckDBSuccess,
};

use crate::abi::AbiPolicy;
use crate::connection::Connection;
use crate::error::ExtensionError;

/// Generates the `#[no_mangle] unsafe extern "C"` entry point for a `DuckDB` extension.
///
/// This macro eliminates the boilerplate of writing the entry point manually.
/// It emits a `#[no_mangle] pub unsafe extern "C"` function with the name you
/// supply, which `DuckDB` locates by symbol when loading the extension.
///
/// # Arguments
///
/// - `$fn_name`: The exact symbol name `DuckDB` will call (e.g.,
///   `my_extension_init_c_api`). `DuckDB` requires this to follow the
///   `{extension_name}_init_c_api` convention.
/// - `$register`: A closure or function of type
///   `fn(duckdb_connection) -> Result<(), ExtensionError>` that registers your
///   functions on the given connection.
///
/// # ABI policy
///
/// A three-argument form takes an [`AbiPolicy`] between
/// the name and the closure:
///
/// ```rust,no_run
/// use quack_rs::abi::AbiPolicy;
/// use quack_rs::error::ExtensionError;
///
/// quack_rs::entry_point!(my_ext_init_c_api, AbiPolicy::Trust, |_con| {
///     Ok::<(), ExtensionError>(())
/// });
/// ```
///
/// The default is [`AbiPolicy::Strict`], which
/// refuses to load when the running `DuckDB` does not provide the C API struct
/// layout this extension was compiled against. See [`crate::abi`].
///
/// Registration is **not transactional**: functions registered before the
/// closure fails stay registered. See the
/// [module documentation](mod@crate::entry_point#registration-is-not-transactional).
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::entry_point;
/// use quack_rs::error::ExtensionError;
///
/// fn register_functions(
///     _con: libduckdb_sys::duckdb_connection,
/// ) -> Result<(), ExtensionError> {
///     Ok(())
/// }
///
/// entry_point!(my_extension_init_c_api, |con| register_functions(con));
/// ```
#[macro_export]
macro_rules! entry_point {
    ($fn_name:ident, $register:expr) => {
        $crate::entry_point!($fn_name, $crate::abi::AbiPolicy::Strict, $register);
    };
    ($fn_name:ident, $policy:expr, $register:expr) => {
        /// DuckDB extension entry point (generated by `entry_point!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. `info` and `access` are provided by the DuckDB runtime.
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(
            info: ::libduckdb_sys::duckdb_extension_info,
            access: *const ::libduckdb_sys::duckdb_extension_access,
        ) -> bool {
            // The arguments are evaluated inside the entry point's panic guard.
            // SAFETY: `info` and `access` are the pointers DuckDB passed to this
            // entry point, valid for the call, which is the helper's contract.
            unsafe {
                $crate::entry_point::__entry_point(
                    info,
                    access,
                    $crate::DUCKDB_API_VERSION,
                    || $policy,
                    || $register,
                )
            }
        }
    };
}

/// Generates the `#[no_mangle] unsafe extern "C"` entry point using the
/// version-agnostic [`Connection`] facade.
///
/// This is the recommended alternative to [`entry_point!`]. The registration
/// callback receives a <code>&[Connection]</code> instead of a raw `duckdb_connection`,
/// giving access to the [`Registrar`][crate::connection::Registrar] trait and to
/// both the connection and database handles.
///
/// # Arguments
///
/// - `$fn_name`: The exact symbol name `DuckDB` will call.
/// - `$register`: A closure of type `fn(&Connection) -> Result<(), ExtensionError>`.
///
/// Registration is **not transactional**: functions registered before the
/// closure fails stay registered. See the
/// [module documentation](mod@crate::entry_point#registration-is-not-transactional).
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::connection::Registrar;
/// use quack_rs::error::ExtensionError;
/// use quack_rs::connection::Connection;
///
/// quack_rs::entry_point_v2!(my_extension_init_c_api, |con| {
///     // unsafe { con.register_scalar(builder)? };
///     Ok(())
/// });
/// ```
#[macro_export]
macro_rules! entry_point_v2 {
    ($fn_name:ident, $register:expr) => {
        $crate::entry_point_v2!($fn_name, $crate::abi::AbiPolicy::Strict, $register);
    };
    ($fn_name:ident, $policy:expr, $register:expr) => {
        /// DuckDB extension entry point (generated by `entry_point_v2!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. `info` and `access` are provided by the DuckDB runtime.
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(
            info: ::libduckdb_sys::duckdb_extension_info,
            access: *const ::libduckdb_sys::duckdb_extension_access,
        ) -> bool {
            // The arguments are evaluated inside the entry point's panic guard.
            // SAFETY: `info` and `access` are the pointers DuckDB passed to this
            // entry point, valid for the call, which is the helper's contract.
            unsafe {
                $crate::entry_point::__entry_point_v2(
                    info,
                    access,
                    $crate::DUCKDB_API_VERSION,
                    || $policy,
                    || $register,
                )
            }
        }
    };
}

/// Core entry point helper — sets up a connection and calls your registration closure.
///
/// This function encapsulates the correct initialization sequence for a `DuckDB`
/// loadable extension written in Rust:
///
/// 1. Calls `duckdb_rs_extension_api_init` with the given `api_version`.
/// 2. Extracts the `duckdb_database` via `access.get_database`.
/// 3. Opens a `duckdb_connection` via `duckdb_connect`.
/// 4. Calls `register(connection)`.
/// 5. Disconnects with `duckdb_disconnect`.
/// 6. On any error, reports via `access.set_error` and returns `false`.
///
/// Registration is **not transactional**: functions registered before the
/// closure fails stay registered. See the
/// [module documentation](mod@crate::entry_point#registration-is-not-transactional).
///
/// # Return value
///
/// Returns `true` if initialization succeeded, `false` on any error.
///
/// # Safety
///
/// - `info` must be the `duckdb_extension_info` passed by `DuckDB` to your entry point.
/// - `access` must be the `*const duckdb_extension_access` passed by `DuckDB`.
/// - Both pointers must remain valid for the duration of this call.
///
/// # Pitfall L3: No panic across FFI
///
/// This function never panics. All errors are reported via `access.set_error`
/// (when `access` itself is null, there is nowhere to report one, and the
/// function returns `false`).
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::entry_point::init_extension;
///
/// #[no_mangle]
/// pub unsafe extern "C" fn my_ext_init_c_api(
///     info: libduckdb_sys::duckdb_extension_info,
///     access: *const libduckdb_sys::duckdb_extension_access,
/// ) -> bool {
///     unsafe {
///         init_extension(info, access, quack_rs::DUCKDB_API_VERSION, |_con| {
///             // Register your functions here
///             Ok(())
///         })
///     }
/// }
/// ```
pub unsafe fn init_extension<F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    register: F,
) -> bool
where
    F: FnOnce(duckdb_connection) -> Result<(), ExtensionError>,
{
    // SAFETY: forwarded from this function's own contract.
    unsafe { init_extension_with_policy(info, access, api_version, AbiPolicy::default(), register) }
}

/// [`init_extension`] with an explicit [`AbiPolicy`].
///
/// Use this to opt out of the C API layout check — for example when the
/// extension binary is stamped `C_STRUCT_UNSTABLE`, so `DuckDB` already refuses
/// to load it into any release other than the one it was built for.
///
/// # Safety
///
/// Same invariants as [`init_extension`].
pub unsafe fn init_extension_with_policy<F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    policy: AbiPolicy,
    register: F,
) -> bool
where
    F: FnOnce(duckdb_connection) -> Result<(), ExtensionError>,
{
    // SAFETY: `init_extension_internal`'s `# Safety` is "same invariants as
    // `init_extension`", which this function's own `# Safety` ("same invariants as
    // `init_extension`") passes on unchanged: `info` and `access` are the pointers DuckDB
    // passed to the entry point, valid for the duration of this call.
    match unsafe { init_extension_internal(info, access, api_version, policy, register) } {
        Ok(result) => result,
        Err(e) => {
            // SAFETY: access is a valid pointer per the caller's contract.
            unsafe { report_error(info, access, &e) };
            false
        }
    }
}

/// Core entry point helper — sets up a [`Connection`] and calls your registration closure.
///
/// Identical to [`init_extension`] except that the callback receives a
/// <code>&[Connection]</code> instead of a raw `duckdb_connection`. [`Connection`]
/// implements [`Registrar`][crate::connection::Registrar] and also exposes the
/// `duckdb_database` handle for replacement scan registration.
///
/// Prefer this over [`init_extension`] for new extensions. The raw
/// `duckdb_connection` entry point is retained for backward compatibility.
///
/// Registration is **not transactional**: functions registered before the
/// closure fails stay registered. See the
/// [module documentation](mod@crate::entry_point#registration-is-not-transactional).
///
/// # Return value
///
/// Returns `true` if initialization succeeded, `false` on any error.
///
/// # Safety
///
/// - `info` must be the `duckdb_extension_info` passed by `DuckDB` to your entry point.
/// - `access` must be the `*const duckdb_extension_access` passed by `DuckDB`.
/// - Both pointers must remain valid for the duration of this call.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::entry_point::init_extension_v2;
/// use quack_rs::connection::Registrar;
///
/// #[no_mangle]
/// pub unsafe extern "C" fn my_ext_init_c_api(
///     info: libduckdb_sys::duckdb_extension_info,
///     access: *const libduckdb_sys::duckdb_extension_access,
/// ) -> bool {
///     unsafe {
///         init_extension_v2(info, access, quack_rs::DUCKDB_API_VERSION, |con| {
///             // unsafe { con.register_scalar(builder)?; }
///             Ok(())
///         })
///     }
/// }
/// ```
pub unsafe fn init_extension_v2<F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    register: F,
) -> bool
where
    F: FnOnce(&Connection) -> Result<(), crate::error::ExtensionError>,
{
    // SAFETY: forwarded from this function's own contract.
    unsafe {
        init_extension_v2_with_policy(info, access, api_version, AbiPolicy::default(), register)
    }
}

/// [`init_extension_v2`] with an explicit [`AbiPolicy`].
///
/// See [`init_extension_with_policy`].
///
/// # Safety
///
/// Same invariants as [`init_extension_v2`].
pub unsafe fn init_extension_v2_with_policy<F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    policy: AbiPolicy,
    register: F,
) -> bool
where
    F: FnOnce(&Connection) -> Result<(), crate::error::ExtensionError>,
{
    // SAFETY: `init_extension_v2_internal`'s `# Safety` is "same invariants as
    // `init_extension_v2`", which this function's own `# Safety` ("same invariants as
    // `init_extension_v2`") passes on unchanged: `info` and `access` are the pointers
    // DuckDB passed to the entry point, valid for the duration of this call.
    match unsafe { init_extension_v2_internal(info, access, api_version, policy, register) } {
        Ok(result) => result,
        Err(e) => {
            // SAFETY: access is a valid pointer per the caller's contract.
            unsafe { report_error(info, access, &e) };
            false
        }
    }
}

/// Internal implementation of [`init_extension_v2`] using `?` for error propagation.
///
/// # Safety
///
/// Same invariants as [`init_extension_v2`].
unsafe fn init_extension_v2_internal<F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    policy: AbiPolicy,
    register: F,
) -> Result<bool, crate::error::ExtensionError>
where
    F: FnOnce(&Connection) -> Result<(), crate::error::ExtensionError>,
{
    // Step 1: Initialize the DuckDB C API.
    //
    // PITFALL P2: Use the C API version, not the DuckDB release version.
    // DuckDB v1.4.x and v1.5.x use C API version v1.2.0.
    //
    // `duckdb_rs_extension_api_init` builds a CString from `api_version` and
    // unwraps; an interior NUL would panic across the C entry point.
    if api_version.contains('\0') {
        return Err(crate::error::ExtensionError::new(
            "api_version must not contain an interior NUL byte",
        ));
    }

    // SAFETY: `access` is the pointer DuckDB passed; the helper only reads it.
    unsafe { check_access(access)? };

    // SAFETY: info and access are valid pointers provided by DuckDB.
    let have_api = unsafe {
        duckdb_rs_extension_api_init(info, access, api_version)
            .map_err(|e| crate::error::ExtensionError::new(e.to_string()))?
    };

    if !have_api {
        return Ok(false);
    }

    // Step 1b: Verify the C API struct layout before touching anything past the
    // stable prefix. See `crate::abi` for why this matters.
    //
    // SAFETY: the dispatch table was initialised by the call above.
    unsafe { enforce_abi_policy(info, access, policy)? };

    // Step 2: Get the database handle. `None` means DuckDB failed and has
    // already recorded why; reporting again would overwrite its message.
    // SAFETY: info and access are the pointers DuckDB passed to the entry point.
    let Some(db) = (unsafe { database_from_access(info, access)? }) else {
        return Ok(false);
    };

    // Step 3: Open a connection for function registration.
    let mut raw_con: duckdb_connection = core::ptr::null_mut();
    // SAFETY: db is a valid duckdb_database returned by get_database.
    let rc = unsafe { duckdb_connect(db, &raw mut raw_con) };
    if rc != DuckDBSuccess {
        return Err(crate::error::ExtensionError::new(
            "duckdb_connect failed during extension initialization",
        ));
    }

    // Step 4: Build the Connection facade and call the user's registration closure.
    //
    // SAFETY: raw_con and db are both valid for the duration of this scope.
    let con = unsafe { Connection::from_raw(raw_con, db) };
    // PITFALL L3: see `init_extension_internal` — user code must not unwind out
    // of the C entry point.
    let result = catch_registration_panic(|| register(&con));

    // Step 5: Always disconnect, even if registration failed.
    // SAFETY: raw_con was successfully created by duckdb_connect above.
    unsafe { duckdb_disconnect(&raw mut raw_con) };

    result?;
    Ok(true)
}

/// Internal implementation using `?` for ergonomic error propagation.
///
/// # Safety
///
/// Same invariants as [`init_extension`].
unsafe fn init_extension_internal<F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    policy: AbiPolicy,
    register: F,
) -> Result<bool, ExtensionError>
where
    F: FnOnce(duckdb_connection) -> Result<(), ExtensionError>,
{
    // Step 1: Initialize the DuckDB C API. This must be called before any other
    // libduckdb_sys function in a loadable extension. The version string must be
    // the C API version (e.g. "v1.2.0"), NOT the DuckDB release version.
    //
    // PITFALL P2: Use the C API version, not the DuckDB release version.
    // DuckDB v1.4.x and v1.5.x use C API version v1.2.0.
    //
    // `duckdb_rs_extension_api_init` builds a CString from `api_version` and
    // unwraps; an interior NUL would panic across the C entry point.
    if api_version.contains('\0') {
        return Err(ExtensionError::new(
            "api_version must not contain an interior NUL byte",
        ));
    }

    // SAFETY: `access` is the pointer DuckDB passed; the helper only reads it.
    unsafe { check_access(access)? };

    // SAFETY: info and access are valid pointers provided by DuckDB.
    let have_api = unsafe {
        duckdb_rs_extension_api_init(info, access, api_version)
            .map_err(|e| ExtensionError::new(e.to_string()))?
    };

    if !have_api {
        // DuckDB indicated that the API version is not available. Return false
        // without an error — this can happen when the extension is loaded by
        // an older DuckDB version that predates the requested API version.
        return Ok(false);
    }

    // Step 1b: Verify the C API struct layout before touching anything past the
    // stable prefix. See `crate::abi` for why this matters.
    //
    // SAFETY: the dispatch table was initialised by the call above.
    unsafe { enforce_abi_policy(info, access, policy)? };

    // Step 2: Get the database handle. `None` means DuckDB failed and has
    // already recorded why; reporting again would overwrite its message.
    // SAFETY: info and access are the pointers DuckDB passed to the entry point.
    let Some(db) = (unsafe { database_from_access(info, access)? }) else {
        return Ok(false);
    };

    // Step 3: Open a connection for function registration.
    let mut raw_con: duckdb_connection = core::ptr::null_mut();
    // SAFETY: db is a valid duckdb_database returned by get_database.
    let rc = unsafe { duckdb_connect(db, &raw mut raw_con) };
    if rc != DuckDBSuccess {
        return Err(ExtensionError::new(
            "duckdb_connect failed during extension initialization",
        ));
    }

    // Step 4: Call the user's registration closure.
    //
    // PITFALL L3: a panic must never unwind across the C entry point. The
    // closure is arbitrary user code, so it is run inside `catch_unwind` and any
    // panic is converted into an `ExtensionError` that DuckDB reports as a LOAD
    // failure. (`catch_unwind` is inert under `panic = "abort"`; see the module
    // docs for why quack-rs recommends `panic = "unwind"` for extensions.)
    let result = catch_registration_panic(|| register(raw_con));

    // Step 5: Always disconnect, even if registration failed.
    // SAFETY: raw_con was successfully created by duckdb_connect above.
    unsafe { duckdb_disconnect(&raw mut raw_con) };

    result?;
    Ok(true)
}

/// Fetches the `duckdb_database` through `access.get_database`.
///
/// Returns `Ok(None)` when `get_database` returns a null pointer. `DuckDB`'s
/// implementation (`ExtensionAccess::GetDatabase` in
/// `src/main/extension/extension_load.cpp`) returns null only after catching an
/// exception, and it records that exception as the load error before returning —
/// so the caller must return `false` **without** calling `set_error`, which would
/// replace `DuckDB`'s own diagnostic with a vaguer one. The loader then throws
/// the recorded error.
///
/// # Errors
///
/// Returns an error if `access.get_database` itself is null; that is a broken
/// access struct, and nothing has been reported yet.
///
/// # Safety
///
/// `access` must be a valid, non-null `duckdb_extension_access` and `info` the
/// matching `duckdb_extension_info`.
unsafe fn database_from_access(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
) -> Result<Option<libduckdb_sys::duckdb_database>, ExtensionError> {
    // SAFETY: `access` is valid per this function's contract.
    let get_database = unsafe { (*access).get_database }
        .ok_or_else(|| ExtensionError::new("get_database function pointer is null"))?;
    // SAFETY: `info` is the handle DuckDB passed with `access`.
    let db_ptr = unsafe { get_database(info) };
    if db_ptr.is_null() {
        return Ok(None);
    }
    // SAFETY: non-null, and DuckDB keeps the wrapper it points to alive for the
    // whole load (`load_state.database_data`).
    Ok(Some(unsafe { *db_ptr }))
}

/// Runs the user's registration closure, converting any panic into an
/// [`ExtensionError`] instead of letting it unwind across the C boundary.
///
/// # Pitfall L3: no panic across FFI
///
/// The registration closure is arbitrary user code. Without this, a panic
/// unwinds to the `#[no_mangle] extern "C"` entry point, where Rust aborts the
/// whole process — taking the user's `DuckDB` session with it. Catching it here
/// turns a bug in registration into an ordinary `LOAD` error.
///
/// Note that `catch_unwind` cannot catch anything when the extension is built
/// with `panic = "abort"`. quack-rs's scaffold therefore generates
/// `panic = "unwind"` for extension crates.
/// Evaluates an entry-point macro's `$policy` and `$register` arguments, with
/// a panic in either turned into an error rather than unwinding out of the
/// generated `extern "C"` function.
fn evaluate_arguments<P, R, F>(policy: P, register: R) -> Result<(AbiPolicy, F), ExtensionError>
where
    P: FnOnce() -> AbiPolicy,
    R: FnOnce() -> F,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (policy(), register()))).map_err(
        |panic| {
            ExtensionError::new(format!(
                "evaluating the entry point's arguments panicked: {}",
                crate::callback::take_panic_message(panic)
            ))
        },
    )
}

/// What [`entry_point!`] expands to: [`init_extension_with_policy`], with the
/// macro's arguments evaluated under the panic guard. Not public API.
///
/// # Safety
///
/// Same invariants as [`init_extension`].
#[doc(hidden)]
pub unsafe fn __entry_point<P, R, F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    policy: P,
    register: R,
) -> bool
where
    P: FnOnce() -> AbiPolicy,
    R: FnOnce() -> F,
    F: FnOnce(duckdb_connection) -> Result<(), ExtensionError>,
{
    match evaluate_arguments(policy, register) {
        // SAFETY: forwarded from this function's own contract.
        Ok((policy, register)) => unsafe {
            init_extension_with_policy(info, access, api_version, policy, register)
        },
        Err(e) => {
            // SAFETY: `access` is null or valid per the caller's contract.
            unsafe { report_error(info, access, &e) };
            false
        }
    }
}

/// What [`entry_point_v2!`] expands to: [`init_extension_v2_with_policy`],
/// with the macro's arguments evaluated under the panic guard. Not public API.
///
/// # Safety
///
/// Same invariants as [`init_extension_v2`].
#[doc(hidden)]
pub unsafe fn __entry_point_v2<P, R, F>(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    api_version: &str,
    policy: P,
    register: R,
) -> bool
where
    P: FnOnce() -> AbiPolicy,
    R: FnOnce() -> F,
    F: FnOnce(&Connection) -> Result<(), ExtensionError>,
{
    match evaluate_arguments(policy, register) {
        // SAFETY: forwarded from this function's own contract.
        Ok((policy, register)) => unsafe {
            init_extension_v2_with_policy(info, access, api_version, policy, register)
        },
        Err(e) => {
            // SAFETY: `access` is null or valid per the caller's contract.
            unsafe { report_error(info, access, &e) };
            false
        }
    }
}

fn catch_registration_panic<F>(register: F) -> Result<(), ExtensionError>
where
    F: FnOnce() -> Result<(), ExtensionError>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(register)) {
        Ok(result) => result,
        // The payload is user data whose `Drop` may itself panic; dropping it
        // here, unguarded, would unwind out of the C entry point after all.
        Err(panic) => Err(ExtensionError::new(format!(
            "extension registration panicked: {}",
            crate::callback::take_panic_message(panic)
        ))),
    }
}

/// Refuses an `access` that `duckdb_rs_extension_api_init` cannot use: it
/// dereferences `access` and calls `get_api` through `Option::unwrap`, so a
/// null pointer would crash and a missing `get_api` would panic across the C
/// entry point. A released `DuckDB` always sets both for a loadable extension;
/// unreleased `DuckDB` (`main`) passes a null `get_api` when it links a C API
/// extension statically.
///
/// # Safety
///
/// `access` must be null or point at a live `duckdb_extension_access`.
unsafe fn check_access(access: *const duckdb_extension_access) -> Result<(), ExtensionError> {
    if access.is_null() {
        return Err(ExtensionError::new(
            "DuckDB passed a null extension access struct",
        ));
    }
    // SAFETY: `access` is non-null and live per this function's contract.
    if unsafe { (*access).get_api }.is_none() {
        return Err(ExtensionError::new(
            "DuckDB passed no get_api function, so the C API cannot be initialised",
        ));
    }
    Ok(())
}

/// Applies an [`AbiPolicy`] to the result of [`crate::abi::check`].
///
/// Returns `Err` when [`policy_verdict`] refuses the load. Under
/// [`AbiPolicy::Warn`] the diagnostic goes to stderr and loading continues
/// (not through `set_error`, which would fail the load: see below); under
/// [`AbiPolicy::Trust`] no check runs.
///
/// # Safety
///
/// The `DuckDB` C API dispatch table must already be initialised, and `info` /
/// `access` must be the pointers `DuckDB` passed to the entry point.
unsafe fn enforce_abi_policy(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    policy: AbiPolicy,
) -> Result<(), ExtensionError> {
    if policy == AbiPolicy::Trust {
        return Ok(());
    }
    // SAFETY: forwarded from this function's own contract.
    let check = unsafe { crate::abi::check() };
    let message = match policy_verdict(policy, &check) {
        AbiVerdict::Load => return Ok(()),
        AbiVerdict::Refuse(message) => return Err(ExtensionError::new(message)),
        AbiVerdict::Warn(message) => message,
    };
    // AbiPolicy::Warn: surface the diagnostic without failing the load.
    //
    // This deliberately does NOT go through `access.set_error`. DuckDB's loader
    // throws whenever an extension called `set_error`, regardless of what the
    // init function returned:
    //
    //     if (load_state.has_error) {
    //         load_state.error_data.Throw("An error was thrown during ...");
    //     }
    //
    // — so reporting a *warning* that way would abort the load and make `Warn`
    // indistinguishable from `Strict`. The C extension API has no non-fatal
    // diagnostic channel, so stderr is the honest one.
    //
    // `eprintln!` panics when the write fails (a closed pipe, a full disk),
    // and this runs under the C entry point, outside the registration
    // closure's `catch_unwind`: a panic here would abort the host process.
    // A lost warning is the right trade.
    let _ = (info, access);
    let _ = writeln!(std::io::stderr(), "quack-rs warning: {message}");
    Ok(())
}

/// What an [`AbiPolicy`] makes of one [`AbiCheck`][crate::abi::AbiCheck].
#[derive(Debug, PartialEq, Eq)]
enum AbiVerdict {
    Load,
    /// Load, and print the diagnostic.
    Warn(String),
    /// Fail the load with the diagnostic.
    Refuse(String),
}

/// The decision [`enforce_abi_policy`] applies, kept pure so every policy can
/// be tested against every check result without a `DuckDB`.
///
/// A passing check loads under every policy. A failing one is refused, except
/// that [`AbiPolicy::Warn`] only warns, [`AbiPolicy::AllowUnknownEngine`]
/// loads on an engine release the table has no entry for (it still refuses a
/// layout it can identify as different), and [`AbiPolicy::Trust`] loads.
fn policy_verdict(policy: AbiPolicy, check: &crate::abi::AbiCheck) -> AbiVerdict {
    let Some(message) = check.error_message() else {
        return AbiVerdict::Load;
    };
    match policy {
        AbiPolicy::Trust => AbiVerdict::Load,
        AbiPolicy::Warn => AbiVerdict::Warn(message),
        AbiPolicy::AllowUnknownEngine
            if matches!(check, crate::abi::AbiCheck::UnknownEngineVersion { .. }) =>
        {
            AbiVerdict::Load
        }
        AbiPolicy::Strict | AbiPolicy::AllowUnknownEngine => AbiVerdict::Refuse(message),
    }
}

/// Reports an `ExtensionError` back to `DuckDB` via `access.set_error`.
///
/// # Safety
///
/// `info` and `access` must be valid pointers provided by `DuckDB`.
unsafe fn report_error(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
    error: &ExtensionError,
) {
    // Defensive: if access is null, we cannot report the error to DuckDB.
    if access.is_null() {
        return;
    }
    // SAFETY: access is non-null per the check above and valid per caller's contract.
    if let Some(set_error) = unsafe { (*access).set_error } {
        // Replace, not truncate at, an interior NUL: the same rule as every
        // other error path (`ExtensionError::to_c_string` included).
        let c_msg = crate::callback::message_to_c_string(error.as_str());
        // SAFETY: c_msg is a valid CString; info is valid.
        unsafe { set_error(info, c_msg.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    // Integration tests for init_extension require a DuckDB instance and are in
    // tests/integration_test.rs. Unit tests here verify pure-Rust logic.

    use super::{
        catch_registration_panic, database_from_access, evaluate_arguments,
        init_extension_internal, policy_verdict, AbiPolicy, AbiVerdict,
    };
    use crate::error::ExtensionError;

    /// Every policy against every kind of check result. `Trust` skips the
    /// check entirely in `enforce_abi_policy`; its row here pins that it
    /// would load anyway.
    #[test]
    fn every_policy_against_every_check_result() {
        use crate::abi::AbiCheck;
        let checks = [
            AbiCheck::Compatible {
                engine_version: "v1.5.5".into(),
                slots: 546,
            },
            AbiCheck::StableOnly,
            AbiCheck::LayoutMismatch {
                engine_version: "v1.5.0".into(),
                engine_slots: 545,
                compiled_slots: 546,
            },
            AbiCheck::UnknownEngineVersion {
                engine_version: "v9.9.9".into(),
                compiled_slots: 546,
            },
            AbiCheck::EngineVersionUnavailable {
                compiled_slots: 546,
            },
            AbiCheck::DeclaredVersionMismatch {
                declared_version: "v1.2.0".into(),
                declared_slots: 0,
                compiled_slots: 546,
            },
        ];
        // Expected verdict per check, in the order above: L = load,
        // W = warn and load, R = refuse.
        let table: [(AbiPolicy, [char; 6]); 4] = [
            (AbiPolicy::Strict, ['L', 'L', 'R', 'R', 'R', 'R']),
            (AbiPolicy::Warn, ['L', 'L', 'W', 'W', 'W', 'W']),
            (
                AbiPolicy::AllowUnknownEngine,
                ['L', 'L', 'R', 'L', 'R', 'R'],
            ),
            (AbiPolicy::Trust, ['L', 'L', 'L', 'L', 'L', 'L']),
        ];
        for (policy, row) in table {
            for (check, want) in checks.iter().zip(row) {
                let got = policy_verdict(policy, check);
                let message = check.error_message();
                let expected = match want {
                    'L' => AbiVerdict::Load,
                    'W' => AbiVerdict::Warn(message.expect("a failing check has a message")),
                    _ => AbiVerdict::Refuse(message.expect("a failing check has a message")),
                };
                assert_eq!(got, expected, "{policy:?} on {check:?}");
            }
        }
    }

    #[test]
    fn extension_error_to_c_string() {
        let err = ExtensionError::new("test error message");
        let cstr = err.to_c_string();
        assert_eq!(cstr.to_str().unwrap(), "test error message");
    }

    #[test]
    fn registration_success_passes_through() {
        assert!(catch_registration_panic(|| Ok(())).is_ok());
    }

    #[test]
    fn registration_error_passes_through() {
        let err = catch_registration_panic(|| Err(ExtensionError::new("nope")))
            .expect_err("error must propagate");
        assert_eq!(err.as_str(), "nope");
    }

    #[test]
    fn str_panic_is_converted_to_an_error() {
        let err =
            catch_registration_panic(|| panic!("boom")).expect_err("panic must become an error");
        assert!(err.as_str().contains("registration panicked"), "{err}");
        assert!(err.as_str().contains("boom"), "{err}");
    }

    #[test]
    fn string_panic_is_converted_to_an_error() {
        let err = catch_registration_panic(|| panic!("boom {}", 42))
            .expect_err("panic must become an error");
        assert!(err.as_str().contains("boom 42"), "{err}");
    }

    #[test]
    fn non_string_panic_payload_still_yields_an_error() {
        let err = catch_registration_panic(|| std::panic::panic_any(7u8))
            .expect_err("panic must become an error");
        assert!(err.as_str().contains("registration panicked"), "{err}");
    }

    #[test]
    fn unwrap_inside_registration_does_not_escape() {
        // The single most common way real registration code panics.
        fn lookup(key: &str) -> Option<u8> {
            (key == "present").then_some(1)
        }
        let err = catch_registration_panic(|| {
            let _ = lookup("missing").unwrap();
            Ok(())
        })
        .expect_err("unwrap panic must become an error");
        assert!(err.as_str().contains("registration panicked"), "{err}");
    }

    #[test]
    fn allow_unknown_engine_only_forgives_the_unknown_case() {
        use crate::abi::AbiCheck;
        // The policy must not become a blanket opt-out: a layout DuckDB can be
        // shown to lay out differently is still refused.
        let unknown = AbiCheck::UnknownEngineVersion {
            engine_version: "v1.6.0".into(),
            compiled_slots: 546,
        };
        let mismatch = AbiCheck::LayoutMismatch {
            engine_version: "v1.5.0".into(),
            engine_slots: 545,
            compiled_slots: 546,
        };
        assert!(matches!(unknown, AbiCheck::UnknownEngineVersion { .. }));
        assert!(!matches!(mismatch, AbiCheck::UnknownEngineVersion { .. }));
        assert!(mismatch.error_message().is_some());
    }

    #[test]
    fn api_version_with_interior_nul_is_rejected_before_ffi() {
        // Must fail before touching DuckDB: libduckdb-sys would otherwise
        // `CString::new(..).unwrap()` and panic across the C entry point. Both
        // pointers stay null because nothing may dereference them on this path.
        let result = unsafe {
            init_extension_internal(
                core::ptr::null_mut(),
                core::ptr::null(),
                "v1.2\0.0",
                AbiPolicy::Trust,
                |_| Ok(()),
            )
        };
        let err = result.expect_err("interior NUL must be rejected");
        assert!(err.as_str().contains("NUL"), "{err}");
    }

    // ─── get_database returning null ────────────────────────────────────────
    //
    // A fake access struct stands in for DuckDB's here; nothing on this path
    // touches the C API dispatch table, so no database is needed.

    use libduckdb_sys::{duckdb_database, duckdb_extension_access, duckdb_extension_info};
    use std::os::raw::c_char;

    unsafe extern "C" fn no_error(_: duckdb_extension_info, _: *const c_char) {}
    unsafe extern "C" fn no_api(
        _: duckdb_extension_info,
        _: *const c_char,
    ) -> *const std::os::raw::c_void {
        core::ptr::null()
    }
    unsafe extern "C" fn null_database(_: duckdb_extension_info) -> *mut duckdb_database {
        core::ptr::null_mut()
    }

    /// A sentinel handle: only ever compared, never dereferenced.
    static mut FAKE_DB: duckdb_database = 0x5eed as duckdb_database;

    unsafe extern "C" fn fake_database(_: duckdb_extension_info) -> *mut duckdb_database {
        // Only the address is taken here; no reference to the `static mut` is
        // formed.
        &raw mut FAKE_DB
    }

    fn access_with(
        get_database: Option<unsafe extern "C" fn(duckdb_extension_info) -> *mut duckdb_database>,
    ) -> duckdb_extension_access {
        duckdb_extension_access {
            set_error: Some(no_error),
            get_database,
            get_api: Some(no_api),
        }
    }

    /// The message the capturing `set_error` below last received.
    static REPORTED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    unsafe extern "C" fn capture_error(_: duckdb_extension_info, msg: *const c_char) {
        // SAFETY: `report_error` passes a live NUL-terminated string.
        let text = unsafe { std::ffi::CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned();
        if let Ok(mut slot) = REPORTED.lock() {
            *slot = Some(text);
        }
    }

    /// A NUL inside an error message is replaced, not truncated at: the same
    /// rule as every other `set_error` path in the crate
    /// (`callback::message_to_c_string`), so the text after it survives.
    #[test]
    fn an_init_error_with_an_interior_nul_keeps_its_tail() {
        let access = duckdb_extension_access {
            set_error: Some(capture_error),
            get_database: Some(null_database),
            get_api: Some(no_api),
        };
        // SAFETY: `access` is a valid struct; `info` is never dereferenced.
        unsafe {
            super::report_error(
                core::ptr::null_mut(),
                &raw const access,
                &crate::error::ExtensionError::new("bad config\0: key `x` missing"),
            );
        }
        let reported = REPORTED.lock().ok().and_then(|mut s| s.take());
        assert_eq!(reported.as_deref(), Some("bad config?: key `x` missing"));
    }

    static REPORTED_NO_API: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    unsafe extern "C" fn capture_no_api(_: duckdb_extension_info, msg: *const c_char) {
        // SAFETY: `report_error` passes a live NUL-terminated string.
        let text = unsafe { std::ffi::CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned();
        if let Ok(mut slot) = REPORTED_NO_API.lock() {
            *slot = Some(text);
        }
    }

    /// `duckdb_rs_extension_api_init` unwraps `get_api`: a null one panicked
    /// across the C entry point (an abort in a real load), and a null
    /// `access` was dereferenced. Both now fail the load cleanly.
    #[test]
    fn a_missing_get_api_or_access_fails_the_load_without_panicking() {
        let access = duckdb_extension_access {
            set_error: Some(capture_no_api),
            get_database: Some(null_database),
            get_api: None,
        };
        // SAFETY: `access` is a valid struct; `info` is never dereferenced.
        let loaded = unsafe {
            super::init_extension(core::ptr::null_mut(), &raw const access, "v1.2.0", |_| {
                Ok(())
            })
        };
        assert!(!loaded);
        let reported = REPORTED_NO_API.lock().ok().and_then(|mut s| s.take());
        assert!(
            reported
                .as_deref()
                .is_some_and(|m| m.contains("no get_api")),
            "{reported:?}"
        );
        // SAFETY: a null `access` is what is being tested; nothing reads it.
        let loaded = unsafe {
            super::init_extension(core::ptr::null_mut(), core::ptr::null(), "v1.2.0", |_| {
                Ok(())
            })
        };
        assert!(!loaded);
    }

    #[test]
    fn a_null_database_is_reported_as_already_handled() {
        // Before the fix this dereferenced the null pointer (SIGSEGV).
        let access = access_with(Some(null_database));
        // SAFETY: `access` is a valid struct; `info` is never dereferenced.
        let db = unsafe { database_from_access(core::ptr::null_mut(), &raw const access) }
            .expect("a null database is not a new error");
        assert!(db.is_none(), "DuckDB already recorded the error");
    }

    #[test]
    fn a_non_null_database_is_read_through() {
        let access = access_with(Some(fake_database));
        // SAFETY: as above.
        let db = unsafe { database_from_access(core::ptr::null_mut(), &raw const access) }
            .expect("get_database succeeded");
        assert_eq!(db, Some(0x5eed as duckdb_database));
    }

    #[test]
    fn a_missing_get_database_pointer_is_an_error() {
        let access = access_with(None);
        // SAFETY: as above.
        let err = unsafe { database_from_access(core::ptr::null_mut(), &raw const access) }
            .expect_err("no function pointer is our error to report");
        assert!(err.as_str().contains("get_database"), "{err}");
    }

    #[test]
    fn a_payload_whose_drop_panics_does_not_escape_registration() {
        // Before the fix the payload was dropped outside any guard, so this
        // aborted the test binary instead of returning.
        struct PayloadBomb;
        impl Drop for PayloadBomb {
            fn drop(&mut self) {
                panic!("payload destructor deliberately exploded");
            }
        }
        let err = catch_registration_panic(|| std::panic::panic_any(PayloadBomb))
            .expect_err("panic must become an error");
        assert!(err.as_str().contains("registration panicked"), "{err}");
    }

    #[test]
    fn a_panicking_entry_point_argument_is_an_error_naming_the_panic() {
        let (policy, register) =
            evaluate_arguments(|| AbiPolicy::Trust, || 7).expect("no panic, no error");
        assert_eq!((policy, register), (AbiPolicy::Trust, 7));
        let err = evaluate_arguments(|| AbiPolicy::Strict, || -> u8 { panic!("bad register") })
            .expect_err("a panic is an error");
        assert_eq!(
            err.as_str(),
            "evaluating the entry point's arguments panicked: bad register"
        );
        let err = evaluate_arguments(|| -> AbiPolicy { panic!("bad policy") }, || 0)
            .expect_err("a panic is an error");
        assert!(err.as_str().ends_with("bad policy"), "{err}");
    }
}
