// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Panic-safe callback wrapper macros for `DuckDB` extension callbacks.
//!
//! Every `DuckDB` callback is an `unsafe extern "C" fn`, and a Rust panic that
//! reaches that boundary aborts the process — taking the user's `DuckDB` session,
//! and any application embedding it, with it. These macros wrap the body in
//! [`std::panic::catch_unwind`] and route the panic message to whichever
//! `set_error` `DuckDB` provides for that callback kind, so a bug in extension
//! code becomes an ordinary SQL error.
//!
//! # Macros
//!
//! | Macro | Wraps | Reports through |
//! |-------|-------|-----------------|
//! | `scalar_callback!` | scalar function execution | `duckdb_scalar_function_set_error` |
//! | `table_bind_callback!` | table function bind | `duckdb_bind_set_error` |
//! | `table_init_callback!` | table function init / local init | `duckdb_init_set_error` |
//! | `scalar_bind_callback!` | scalar function bind (`duckdb-1-5`) | `duckdb_scalar_function_bind_set_error` |
//! | `scalar_init_callback!` | scalar function init (`duckdb-1-5`) | `duckdb_scalar_function_init_set_error` |
//! | `table_scan_callback!` | table function scan | `duckdb_function_set_error`, then chunk size 0 |
//! | `aggregate_update_callback!` | aggregate update | `duckdb_aggregate_function_set_error` |
//! | `aggregate_combine_callback!` | aggregate combine | `duckdb_aggregate_function_set_error` |
//! | `aggregate_finalize_callback!` | aggregate finalize | `duckdb_aggregate_function_set_error` |
//! | `aggregate_destroy_callback!` | aggregate state destructor | *(no error channel — panic is swallowed)* |
//! | `cast_callback!` | cast function | `duckdb_cast_function_set_error`, returns `false` |
//! | `replacement_scan_callback!` | replacement scan | `duckdb_replacement_scan_set_error` |
//!
//! # `panic = "abort"`
//!
//! `catch_unwind` cannot catch anything when the crate is built with
//! `panic = "abort"`, which silently disables everything in this module. Keep
//! extension crates on `panic = "unwind"` — the quack-rs scaffold generates that.
//!
//! # The body returns `()`
//!
//! A body is the closure `catch_unwind` runs, and its value is discarded. So
//! every body except `cast_callback!`'s (which returns the cast's `bool`) must
//! have type `()`: a body that used `?` would otherwise compile, and the error
//! it returned would be dropped without being reported.
//!
//! ```rust,compile_fail,E0308
//! quack_rs::scalar_callback!(parse_or_drop, |info, input, output| {
//!     let _n: i64 = "not a number".parse()?; // dropped, not reported
//!     Ok::<(), std::num::ParseIntError>(())
//! });
//! ```
//!
//! Report such an error with the callback's `set_error` instead.
//!
//! # Example: Scalar callback
//!
//! ```rust,no_run
//! use quack_rs::data_chunk::DataChunk;
//! use quack_rs::vector::VectorWriter;
//!
//! quack_rs::scalar_callback!(my_func, |info, input, output| {
//!     let chunk = unsafe { DataChunk::from_raw(input) };
//!     let mut writer = unsafe { VectorWriter::from_vector(output) };
//!     for row in 0..chunk.size() {
//!         let val = unsafe { chunk.reader(0).read_i64(row) };
//!         unsafe { writer.write_i64(row, val * 2) };
//!     }
//! });
//! ```
//!
//! # Example: Table scan callback
//!
//! ```rust,no_run
//! use quack_rs::data_chunk::DataChunk;
//!
//! quack_rs::table_scan_callback!(my_scan, |info, output| {
//!     let chunk = unsafe { DataChunk::from_raw(output) };
//!     let mut writer = unsafe { chunk.writer(0) };
//!     unsafe { writer.write_i64(0, 42) };
//!     unsafe { chunk.set_size(1) };
//! });
//! ```

mod payload;

pub use payload::{drop_panic_payload, take_panic_message, MAX_NESTED_PAYLOAD_DROPS};

/// Generates a panic-safe `unsafe extern "C"` scalar function callback.
///
/// The macro emits a function with signature:
/// ```text
/// unsafe extern "C" fn $name(
///     info: duckdb_function_info,
///     input: duckdb_data_chunk,
///     output: duckdb_vector,
/// )
/// ```
///
/// The body is wrapped in `std::panic::catch_unwind`. If the closure panics,
/// the error is reported via `duckdb_scalar_function_set_error` and the
/// function returns without unwinding across the FFI boundary.
///
/// # Parameters
///
/// - `$name` — the name of the generated function
/// - `$body` — a closure with signature `|info: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector|`
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::scalar_callback!(double_it, |info, input, output| {
///     let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
///     let mut writer = unsafe { quack_rs::vector::VectorWriter::from_vector(output) };
///     for row in 0..chunk.size() {
///         let val = unsafe { chunk.reader(0).read_i64(row) };
///         unsafe { writer.write_i64(row, val * 2) };
///     }
/// });
/// ```
#[macro_export]
macro_rules! scalar_callback {
    ($name:ident, |$info:ident, $input:ident, $output:ident| $body:block) => {
        /// Scalar function callback (generated by `scalar_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        /// Panics are caught and reported via `duckdb_scalar_function_set_error`.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_function_info,
            $input: ::libduckdb_sys::duckdb_data_chunk,
            $output: ::libduckdb_sys::duckdb_vector,
        ) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in; `c_msg` outlives the call.
                unsafe {
                    ::libduckdb_sys::duckdb_scalar_function_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` table function scan callback.
///
/// The macro emits a function with signature:
/// ```text
/// unsafe extern "C" fn $name(
///     info: duckdb_function_info,
///     output: duckdb_data_chunk,
/// )
/// ```
///
/// The body is wrapped in `std::panic::catch_unwind`. If the closure panics,
/// the output chunk size is set to 0 (signaling end of stream) to prevent
/// undefined behaviour from unwinding across the FFI boundary.
///
/// # Parameters
///
/// - `$name` — the name of the generated function
/// - `$body` — a closure with signature `|info: duckdb_function_info, output: duckdb_data_chunk|`
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::table_scan_callback!(my_scan, |info, output| {
///     let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(output) };
///     let mut writer = unsafe { chunk.writer(0) };
///     unsafe { writer.write_i64(0, 42) };
///     unsafe { chunk.set_size(1) };
/// });
/// ```
#[macro_export]
macro_rules! table_scan_callback {
    ($name:ident, |$info:ident, $output:ident| $body:block) => {
        /// Table function scan callback (generated by `table_scan_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        /// Panics are caught; on panic the output chunk size is set to 0.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_function_info,
            $output: ::libduckdb_sys::duckdb_data_chunk,
        ) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` and `output` are the pointers DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_function_set_error($info, c_msg.as_ptr());
                    // Signal end-of-stream so the scan terminates.
                    ::libduckdb_sys::duckdb_data_chunk_set_size($output, 0);
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` table function **bind** callback.
///
/// Emits `unsafe extern "C" fn $name(info: duckdb_bind_info)`. A panic is
/// reported through `duckdb_bind_set_error`, which fails the query during
/// planning rather than aborting the process.
///
/// **Table functions only.** `duckdb_bind_set_error` casts its argument to the
/// table function's info struct, which is much larger than a scalar
/// function's: used on a scalar function, it wrote past the scalar bind info
/// (SIGSEGV in testing). A scalar callback takes a
/// [`RawScalarBindInfo`](crate::scalar::RawScalarBindInfo), so the compiler
/// refuses this macro's output there. Use `scalar_bind_callback!` (feature
/// `duckdb-1-5`) for scalar functions.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::table_bind_callback!(my_bind, |info| {
///     let bind = unsafe { quack_rs::table::BindInfo::new(info) };
///     bind.add_result_column("n", quack_rs::types::TypeId::BigInt);
/// });
/// ```
#[macro_export]
macro_rules! table_bind_callback {
    ($name:ident, |$info:ident| $body:block) => {
        /// Table function bind callback (generated by `table_bind_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. `info` is provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name($info: ::libduckdb_sys::duckdb_bind_info) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_bind_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` table function **init** callback.
///
/// Emits `unsafe extern "C" fn $name(info: duckdb_init_info)`. Use it for both
/// the global `init` and the per-thread `local_init` callback — they share a
/// signature. A panic is reported through `duckdb_init_set_error`.
///
/// **Table functions only.** `duckdb_init_set_error` casts its argument to the
/// table function's info struct, which is much larger than a scalar
/// function's: used on a scalar function, it wrote past the scalar init info
/// (SIGSEGV in testing). A scalar callback takes a
/// [`RawScalarInitInfo`](crate::scalar::RawScalarInitInfo), so the compiler
/// refuses this macro's output there. Use `scalar_init_callback!` (feature
/// `duckdb-1-5`) for scalar functions.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::table_init_callback!(my_init, |info| {
///     let init = unsafe { quack_rs::table::InitInfo::new(info) };
///     init.set_max_threads(1);
/// });
/// ```
#[macro_export]
macro_rules! table_init_callback {
    ($name:ident, |$info:ident| $body:block) => {
        /// Table function init callback (generated by `table_init_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. `info` is provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name($info: ::libduckdb_sys::duckdb_init_info) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_init_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` **scalar function bind** callback
/// (`duckdb-1-5`).
///
/// Emits `unsafe extern "C" fn $name(info: RawScalarBindInfo)`, for
/// [`ScalarFunctionBuilder::bind`](crate::scalar::ScalarFunctionBuilder::bind).
/// A panic is reported through `duckdb_scalar_function_bind_set_error`, which
/// fails the query during planning. The argument type keeps the callback out
/// of a table function's `bind`, and [`table_bind_callback!`](crate::table_bind_callback)'s
/// output out of this one.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::scalar_bind_callback!(my_bind, |info| {
///     let bind = unsafe { quack_rs::scalar::ScalarBindInfo::new(info) };
///     assert!(bind.argument_count() > 0, "needs an argument");
/// });
/// ```
#[cfg(feature = "duckdb-1-5")]
#[macro_export]
macro_rules! scalar_bind_callback {
    ($name:ident, |$info:ident| $body:block) => {
        /// Scalar function bind callback (generated by `scalar_bind_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. `info` is provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name($info: $crate::scalar::RawScalarBindInfo) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the scalar bind info DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_scalar_function_bind_set_error($info.0, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` **scalar function init** callback
/// (`duckdb-1-5`).
///
/// Emits `unsafe extern "C" fn $name(info: RawScalarInitInfo)`, for
/// [`ScalarFunctionBuilder::init`](crate::scalar::ScalarFunctionBuilder::init).
/// A panic is reported through `duckdb_scalar_function_init_set_error`. The
/// argument type keeps the callback out of a table function's `init`, and
/// [`table_init_callback!`](crate::table_init_callback)'s output out of this
/// one.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::scalar_init_callback!(my_init, |info| {
///     let _ = info;
/// });
/// ```
#[cfg(feature = "duckdb-1-5")]
#[macro_export]
macro_rules! scalar_init_callback {
    ($name:ident, |$info:ident| $body:block) => {
        /// Scalar function init callback (generated by `scalar_init_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. `info` is provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name($info: $crate::scalar::RawScalarInitInfo) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the scalar init info DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_scalar_function_init_set_error($info.0, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` aggregate **update** callback.
///
/// Emits
/// `unsafe extern "C" fn $name(info: duckdb_function_info, input: duckdb_data_chunk, states: *mut duckdb_aggregate_state)`.
/// A panic is reported through `duckdb_aggregate_function_set_error`.
///
/// Aggregate callbacks run on `DuckDB`'s worker threads, so an unguarded panic
/// here aborts the process from a thread the user never sees.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::aggregate_update_callback!(my_update, |info, input, states| {
///     let _ = (input, states);
/// });
/// ```
#[macro_export]
macro_rules! aggregate_update_callback {
    ($name:ident, |$info:ident, $input:ident, $states:ident| $body:block) => {
        /// Aggregate update callback (generated by `aggregate_update_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_function_info,
            $input: ::libduckdb_sys::duckdb_data_chunk,
            $states: *mut ::libduckdb_sys::duckdb_aggregate_state,
        ) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_aggregate_function_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` aggregate **combine** callback.
///
/// Emits
/// `unsafe extern "C" fn $name(info: duckdb_function_info, source: *mut duckdb_aggregate_state, target: *mut duckdb_aggregate_state, count: idx_t)`.
///
/// # Pitfall L1
///
/// `target` states were set up by the aggregate's `init` callback, not copied
/// from anything, so the body must propagate **every** field of the state, not
/// only the accumulated data. See
/// [`CombineFn`][crate::aggregate::callbacks::CombineFn].
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::aggregate_combine_callback!(my_combine, |info, source, target, count| {
///     let _ = (source, target, count);
/// });
/// ```
#[macro_export]
macro_rules! aggregate_combine_callback {
    ($name:ident, |$info:ident, $source:ident, $target:ident, $count:ident| $body:block) => {
        /// Aggregate combine callback (generated by `aggregate_combine_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_function_info,
            $source: *mut ::libduckdb_sys::duckdb_aggregate_state,
            $target: *mut ::libduckdb_sys::duckdb_aggregate_state,
            $count: ::libduckdb_sys::idx_t,
        ) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_aggregate_function_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` aggregate **finalize** callback.
///
/// Emits
/// `unsafe extern "C" fn $name(info: duckdb_function_info, source: *mut duckdb_aggregate_state, result: duckdb_vector, count: idx_t, offset: idx_t)`.
///
/// Results are written starting at `offset` in the output vector, not at 0.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::aggregate_finalize_callback!(my_finalize, |info, source, result, count, offset| {
///     let _ = (source, result, count, offset);
/// });
/// ```
#[macro_export]
macro_rules! aggregate_finalize_callback {
    ($name:ident, |$info:ident, $source:ident, $result:ident, $count:ident, $offset:ident| $body:block) => {
        /// Aggregate finalize callback (generated by `aggregate_finalize_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_function_info,
            $source: *mut ::libduckdb_sys::duckdb_aggregate_state,
            $result: ::libduckdb_sys::duckdb_vector,
            $count: ::libduckdb_sys::idx_t,
            $offset: ::libduckdb_sys::idx_t,
        ) {
            let outcome =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = outcome {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_aggregate_function_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` aggregate **state destructor**.
///
/// Emits `unsafe extern "C" fn $name(states: *mut duckdb_aggregate_state, count: idx_t)`.
///
/// This callback receives no `info`, so `DuckDB` offers no way to report an
/// error from it. A panic is therefore caught and **swallowed**: that leaks
/// whatever the destructor had not yet freed, which is strictly better than
/// aborting the database process during query teardown.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::aggregate_destroy_callback!(my_destroy, |states, count| {
///     let _ = (states, count);
/// });
/// ```
#[macro_export]
macro_rules! aggregate_destroy_callback {
    ($name:ident, |$states:ident, $count:ident| $body:block) => {
        /// Aggregate state destructor (generated by `aggregate_destroy_callback!`).
        ///
        /// # Safety
        ///
        /// Called by `DuckDB`. All parameters are provided by the `DuckDB` runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $states: *mut ::libduckdb_sys::duckdb_aggregate_state,
            $count: ::libduckdb_sys::idx_t,
        ) {
            // DuckDB provides no error channel for the destructor, so the panic
            // is dropped rather than reported. Leaking beats aborting — and the
            // payload's own `Drop` is user code too, so it is dropped under a
            // second guard rather than here.
            // `::<_, ()>`: a body that always panics has type `!`, and under
            // the never-type fallback that would make this `if let`
            // irrefutable (`irrefutable_let_patterns`, an error under
            // `-D warnings` on nightly).
            if let ::std::result::Result::Err(panic) =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body))
            {
                $crate::callback::drop_panic_payload(panic);
            }
        }
    };
}

/// The error a [`cast_callback!`][crate::cast_callback] function reports in a
/// regular `CAST` when its body returns `false` without setting a message.
pub const CAST_FAILED_WITHOUT_MESSAGE: &std::ffi::CStr =
    c"cast function failed without reporting an error message";

/// Generates a panic-safe `unsafe extern "C"` **cast** callback.
///
/// Emits
/// `unsafe extern "C" fn $name(info: duckdb_function_info, count: idx_t, input: duckdb_vector, output: duckdb_vector) -> bool`.
/// The body must evaluate to `bool` — `true` when every row converted.
///
/// A panic is reported through `duckdb_cast_function_set_error` and the
/// generated function returns `false`. In a regular `CAST` that fails the
/// query with the panic message. In a `TRY_CAST`, `DuckDB` ignores the return
/// value (see [`CastFn`][crate::cast::CastFn]), so the generated function also
/// sets **every** row of the chunk to `NULL` through
/// `duckdb_cast_function_set_row_error`: whatever the body wrote before it
/// panicked cannot be trusted, and a row it never reached would otherwise keep
/// a stale value.
///
/// A body that returns `false` without panicking is passed through untouched;
/// in `TRY_CAST` mode it must null its own failed rows. In a regular `CAST`, a
/// body that returns `false` without setting a message fails the query with
/// [`CAST_FAILED_WITHOUT_MESSAGE`] rather than an empty `Conversion Error: `.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::cast_callback!(my_cast, |info, count, input, output| {
///     let _ = (count, input, output);
///     true
/// });
/// ```
#[macro_export]
macro_rules! cast_callback {
    ($name:ident, |$info:ident, $count:ident, $input:ident, $output:ident| $body:block) => {
        /// Cast function callback (generated by `cast_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_function_info,
            $count: ::libduckdb_sys::idx_t,
            $input: ::libduckdb_sys::duckdb_vector,
            $output: ::libduckdb_sys::duckdb_vector,
        ) -> bool {
            // SAFETY: `info` is the pointer DuckDB passed in.
            let try_mode = unsafe {
                ::libduckdb_sys::duckdb_cast_function_get_cast_mode($info)
                    == ::libduckdb_sys::duckdb_cast_mode_DUCKDB_CAST_TRY
            };
            if !try_mode {
                // A default for a body that returns `false` without a message:
                // `set_error` only assigns the message (cast_function-c.cpp),
                // so the body's own call replaces it, and DuckDB reads it only
                // when the callback returns `false`.
                // SAFETY: `info` is the pointer DuckDB passed in; the message
                // is a static NUL-terminated string DuckDB copies.
                unsafe {
                    ::libduckdb_sys::duckdb_cast_function_set_error(
                        $info,
                        $crate::callback::CAST_FAILED_WITHOUT_MESSAGE.as_ptr(),
                    );
                }
            }
            let outcome: ::std::result::Result<bool, _> =
                ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body));
            match outcome {
                ::std::result::Result::Ok(ok) => ok,
                ::std::result::Result::Err(panic) => {
                    let c_msg = $crate::callback::panic_c_message(panic);
                    if try_mode {
                        // DuckDB ignores the return value of a TRY cast
                        // (execute_cast.cpp), so the rows must be nulled here.
                        for row in 0..$count {
                            // SAFETY: `info` and `output` are the handles
                            // DuckDB passed in, and `row < count`.
                            unsafe {
                                ::libduckdb_sys::duckdb_cast_function_set_row_error(
                                    $info,
                                    c_msg.as_ptr(),
                                    row,
                                    $output,
                                );
                            }
                        }
                    } else {
                        // SAFETY: `info` is the pointer DuckDB passed in.
                        unsafe {
                            ::libduckdb_sys::duckdb_cast_function_set_error($info, c_msg.as_ptr());
                        }
                    }
                    false
                }
            }
        }
    };
}

/// Generates a panic-safe `unsafe extern "C"` **replacement scan** callback.
///
/// Emits
/// `unsafe extern "C" fn $name(info: duckdb_replacement_scan_info, table_name: *const c_char, data: *mut c_void)`.
/// A panic is reported through `duckdb_replacement_scan_set_error`.
///
/// # Example
///
/// ```rust,no_run
/// quack_rs::replacement_scan_callback!(my_scan, |info, table_name, data| {
///     let _ = (info, table_name, data);
/// });
/// ```
#[macro_export]
macro_rules! replacement_scan_callback {
    ($name:ident, |$info:ident, $table_name:ident, $data:ident| $body:block) => {
        /// Replacement scan callback (generated by `replacement_scan_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_replacement_scan_info,
            $table_name: *const ::std::os::raw::c_char,
            $data: *mut ::std::os::raw::c_void,
        ) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_replacement_scan_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Wraps a `COPY ... TO` **bind** callback so a panic becomes a `DuckDB` error.
///
/// Requires the `duckdb-1-5` feature: `duckdb_copy_function_bind_set_error`
/// lives past the stable prefix of `duckdb_ext_api_v1`.
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "duckdb-1-5")]
/// quack_rs::copy_bind_callback!(my_bind, |info| {
///     let _ = info;
/// });
/// ```
#[cfg(feature = "duckdb-1-5")]
#[macro_export]
macro_rules! copy_bind_callback {
    ($name:ident, |$info:ident| $body:block) => {
        $crate::__copy_callback_impl!(
            $name,
            $info,
            duckdb_copy_function_bind_info,
            duckdb_copy_function_bind_set_error,
            $body
        );
    };
}

/// Wraps a `COPY ... TO` **global init** callback so a panic becomes a
/// `DuckDB` error.
///
/// Requires the `duckdb-1-5` feature.
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "duckdb-1-5")]
/// quack_rs::copy_global_init_callback!(my_init, |info| {
///     let _ = info;
/// });
/// ```
#[cfg(feature = "duckdb-1-5")]
#[macro_export]
macro_rules! copy_global_init_callback {
    ($name:ident, |$info:ident| $body:block) => {
        $crate::__copy_callback_impl!(
            $name,
            $info,
            duckdb_copy_function_global_init_info,
            duckdb_copy_function_global_init_set_error,
            $body
        );
    };
}

/// Wraps a `COPY ... TO` **finalize** callback so a panic becomes a `DuckDB`
/// error.
///
/// Requires the `duckdb-1-5` feature.
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "duckdb-1-5")]
/// quack_rs::copy_finalize_callback!(my_finalize, |info| {
///     let _ = info;
/// });
/// ```
#[cfg(feature = "duckdb-1-5")]
#[macro_export]
macro_rules! copy_finalize_callback {
    ($name:ident, |$info:ident| $body:block) => {
        $crate::__copy_callback_impl!(
            $name,
            $info,
            duckdb_copy_function_finalize_info,
            duckdb_copy_function_finalize_set_error,
            $body
        );
    };
}

/// Shared body of the single-argument copy-function callback macros.
///
/// Not part of the public API: it exists because bind, global init and finalize
/// differ only in their info type and their `set_error` function.
#[cfg(feature = "duckdb-1-5")]
#[doc(hidden)]
#[macro_export]
macro_rules! __copy_callback_impl {
    ($name:ident, $info:ident, $info_ty:ident, $set_error:ident, $body:block) => {
        /// Copy function callback (generated by a `copy_*_callback!` macro).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name($info: ::libduckdb_sys::$info_ty) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let ::std::result::Result::Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::$set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Wraps a `COPY ... TO` **sink** callback so a panic becomes a `DuckDB` error.
///
/// The sink takes a data chunk as well as its info handle, so it has its own
/// macro rather than sharing the single-argument implementation.
///
/// Requires the `duckdb-1-5` feature.
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "duckdb-1-5")]
/// quack_rs::copy_sink_callback!(my_sink, |info, chunk| {
///     let _ = (info, chunk);
/// });
/// ```
#[cfg(feature = "duckdb-1-5")]
#[macro_export]
macro_rules! copy_sink_callback {
    ($name:ident, |$info:ident, $chunk:ident| $body:block) => {
        /// Copy function sink callback (generated by `copy_sink_callback!`).
        ///
        /// # Safety
        ///
        /// Called by DuckDB. All parameters are provided by the DuckDB runtime.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name(
            $info: ::libduckdb_sys::duckdb_copy_function_sink_info,
            $chunk: ::libduckdb_sys::duckdb_data_chunk,
        ) {
            let result =
                ::std::panic::catch_unwind::<_, ()>(::std::panic::AssertUnwindSafe(|| $body));
            if let ::std::result::Result::Err(panic) = result {
                let c_msg = $crate::callback::panic_c_message(panic);
                // SAFETY: `info` is the pointer DuckDB passed in.
                unsafe {
                    ::libduckdb_sys::duckdb_copy_function_sink_set_error($info, c_msg.as_ptr());
                }
            }
        }
    };
}

/// Extracts a human-readable message from a `catch_unwind` payload.
///
/// This only borrows the payload; when you are done with it, dispose of it with
/// [`drop_panic_payload`] (or use [`take_panic_message`], which does both) — a
/// payload's own `Drop` may panic, and at an FFI boundary that aborts.
///
/// # Example
///
/// ```rust
/// let payload = std::panic::catch_unwind(|| panic!("boom")).unwrap_err();
/// assert_eq!(quack_rs::callback::panic_message(&payload), "boom");
/// ```
#[must_use]
pub fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .or_else(|| payload.downcast_ref::<Box<str>>().map(ToString::to_string))
        .unwrap_or_else(|| String::from("<non-string panic payload>"))
}

/// What the callback macros report for a panic whose message is empty
/// (`panic!("")`).
///
/// `DuckDB` would otherwise show a bare error prefix such as
/// `Invalid Input Error: ` — and a replacement scan ignores an empty error
/// altogether and carries on with whatever the callback set up before it
/// panicked (`replacement_scan-c.cpp` only raises a non-empty message).
pub const EMPTY_PANIC_PLACEHOLDER: &str = "an extension callback panicked without a message";

/// Turns a caught panic into the C string a callback macro reports.
///
/// That is the panic message, with any interior NUL replaced by `?`, or
/// [`EMPTY_PANIC_PLACEHOLDER`] if the message is empty. The payload is
/// disposed of as [`take_panic_message`] does.
///
/// # Example
///
/// ```rust
/// let payload = std::panic::catch_unwind(|| panic!("")).unwrap_err();
/// assert_eq!(
///     quack_rs::callback::panic_c_message(payload).to_str().unwrap(),
///     quack_rs::callback::EMPTY_PANIC_PLACEHOLDER,
/// );
/// ```
#[must_use]
pub fn panic_c_message(payload: Box<dyn std::any::Any + Send>) -> std::ffi::CString {
    crate::table::cstr::error_cstring(&take_panic_message(payload), EMPTY_PANIC_PLACEHOLDER)
}

/// Converts a panic message into a `CString`, replacing any interior NUL.
///
/// `CString::new` rejects interior NULs, and a panic message is arbitrary user
/// text — dropping the diagnostic because it happened to contain a NUL would be
/// the wrong trade.
///
/// # Example
///
/// ```rust
/// let c = quack_rs::callback::message_to_c_string("a\0b");
/// assert_eq!(c.to_str().unwrap(), "a?b");
/// ```
#[must_use]
pub fn message_to_c_string(message: &str) -> std::ffi::CString {
    std::ffi::CString::new(message.replace('\0', "?"))
        .unwrap_or_else(|_| std::ffi::CString::default())
}

/// Runs `body` with unwinding contained, returning the panic message on unwind.
///
/// # Why this exists
///
/// Every function quack-rs hands to `DuckDB` is an `extern "C" fn`. Since Rust
/// 1.81 a panic that reaches such a boundary is a guaranteed **process abort**
/// (`panic_cannot_unwind`), not merely undefined behaviour — so an aggregate
/// state whose `Drop` calls `.unwrap()` takes the whole `DuckDB` session down.
/// The macros in this module contain that for the callbacks *you* write; this
/// function is the same containment for the ones quack-rs generates on your
/// behalf, and it is public so extensions writing their own `extern "C"`
/// destructors can use it too.
///
/// `AssertUnwindSafe` is applied internally: at an FFI boundary there is no
/// caller left to observe a broken invariant, and aborting instead is strictly
/// worse.
///
/// # Not a substitute for an error channel
///
/// Where the `DuckDB` callback has a `set_error` counterpart, report the message
/// through it — see the macro table in the [module docs][self]. Some callbacks
/// (notably the aggregate state destructor, which `DuckDB` invokes as
/// `info.destroy(states, count)` with no info handle and no return value) have
/// no channel at all; there, discarding the message is the only option left, and
/// it still beats aborting.
///
/// # `panic = "abort"`
///
/// Like every `catch_unwind` in this crate, this is inert under
/// `panic = "abort"`. Keep extension crates on `panic = "unwind"`.
///
/// # Example
///
/// ```rust
/// use quack_rs::callback::catch_ffi_panic;
///
/// assert_eq!(catch_ffi_panic(|| 1 + 1), Ok(2));
/// assert_eq!(catch_ffi_panic(|| panic!("boom")), Err("boom".to_string()));
/// ```
pub fn catch_ffi_panic<T, F>(body: F) -> Result<T, String>
where
    F: FnOnce() -> T,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).map_err(take_panic_message)
}
