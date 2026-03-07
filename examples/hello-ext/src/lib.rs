// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! # hello-ext
//!
//! A minimal DuckDB community extension built with [`quack_rs`].
//!
//! This example registers one aggregate function:
//!
//! ```sql
//! SELECT word_count(text_column) FROM my_table;
//! -- Returns: BIGINT — total number of whitespace-separated words
//! ```
//!
//! ## How to build
//!
//! ```bash
//! cargo build --release
//! # output: target/release/libhello_ext.so (Linux)
//! #         target/release/libhello_ext.dylib (macOS)
//! ```
//!
//! ## How to load in DuckDB
//!
//! ```sql
//! LOAD 'path/to/libhello_ext.so';
//! SELECT word_count(name) FROM range(5) t(n), (SELECT 'hello world' AS name);
//! -- Returns 10 (5 rows × 2 words each)
//! ```
//!
//! ## Architecture
//!
//! The extension follows the standard `quack_rs` pattern:
//!
//! 1. **Entry point**: `hello_ext_init_c_api` initializes the DuckDB C API and
//!    registers all functions.
//! 2. **State**: `WordCountState` accumulates the word count.
//! 3. **Callbacks**: `update`, `combine`, `finalize` implement the aggregate logic.
//! 4. **Registration**: `AggregateFunctionBuilder` handles all FFI boilerplate.

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_extension_access,
    duckdb_extension_info, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{AggregateState, FfiState};
use quack_rs::aggregate::AggregateFunctionBuilder;
use quack_rs::error::ExtensionError;
use quack_rs::types::TypeId;
use quack_rs::vector::{VectorReader, VectorWriter};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Accumulates the total word count across all input rows.
///
/// Words are defined as runs of non-whitespace characters separated by
/// ASCII whitespace. Empty strings contribute 0 words.
#[derive(Default, Debug)]
struct WordCountState {
    /// Total words seen so far
    count: i64,
}

impl AggregateState for WordCountState {}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

/// `state_size` callback: returns the number of bytes DuckDB must allocate per group.
///
/// # Safety
///
/// Called by DuckDB. `_info` may be null-ish; we never dereference it.
unsafe extern "C" fn state_size(_info: duckdb_function_info) -> idx_t {
    FfiState::<WordCountState>::size_callback(_info)
}

/// `state_init` callback: initializes a freshly allocated state.
///
/// # Safety
///
/// `state` points to `state_size()` bytes of writable memory allocated by DuckDB.
unsafe extern "C" fn state_init(info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { FfiState::<WordCountState>::init_callback(info, state) };
}

/// `update` callback: processes one data chunk.
///
/// Each row is a VARCHAR. We count words in each string (whitespace-split).
///
/// # Safety
///
/// - `input` is a valid data chunk with at least one column of VARCHAR type.
/// - `states` is an array of `chunk_size` state pointers.
unsafe extern "C" fn update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    // SAFETY: input is a valid data chunk provided by DuckDB.
    let reader = unsafe { VectorReader::new(input, 0) };
    let row_count = reader.row_count();

    for row in 0..row_count {
        // SAFETY: row < row_count per loop bounds.
        if !unsafe { reader.is_valid(row) } {
            // NULL input: contributes 0 words (skip)
            continue;
        }

        // SAFETY: row < row_count, column is VARCHAR.
        let s = unsafe { reader.read_str(row) };
        let words = count_words(s);

        // SAFETY: states is a valid array of row_count pointers.
        let state_ptr = unsafe { *states.add(row) };
        // SAFETY: state_ptr was initialized by state_init.
        if let Some(st) = unsafe { FfiState::<WordCountState>::with_state_mut(state_ptr) } {
            st.count += words;
        }
    }
}

/// `combine` callback: merges source states into target states.
///
/// # Pitfall L1
///
/// DuckDB creates fresh zero-initialized target states. This combine function
/// adds the source count to the target count. For `WordCountState` there is
/// only one field, so there are no config fields to propagate — but in a real
/// aggregate (e.g., retention), you must also copy all configuration fields.
///
/// # Safety
///
/// - `source` and `target` are arrays of `count` state pointers.
unsafe extern "C" fn combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        // SAFETY: source and target are arrays of count valid state pointers.
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };

        let src = unsafe { FfiState::<WordCountState>::with_state(src_ptr) };
        let tgt = unsafe { FfiState::<WordCountState>::with_state_mut(tgt_ptr) };

        if let (Some(s), Some(t)) = (src, tgt) {
            t.count += s.count;
        }
    }
}

/// `finalize` callback: writes aggregate results into the output vector.
///
/// # Safety
///
/// - `source` is an array of `count` initialized state pointers.
/// - `result` is a valid BIGINT output vector.
unsafe extern "C" fn finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    // SAFETY: result is a valid output vector provided by DuckDB.
    let mut writer = unsafe { VectorWriter::new(result) };

    for i in 0..count as usize {
        // SAFETY: source is an array of count valid state pointers.
        let state_ptr = unsafe { *source.add(i) };

        match unsafe { FfiState::<WordCountState>::with_state(state_ptr) } {
            Some(st) => {
                // SAFETY: offset + i is within the vector's capacity (DuckDB contract).
                unsafe { writer.write_i64(offset as usize + i, st.count) };
            }
            None => {
                // Null state — mark output as NULL
                // SAFETY: offset + i is within the vector's capacity.
                unsafe { writer.set_null(offset as usize + i) };
            }
        }
    }
}

/// `state_destroy` callback: frees heap-allocated state for each group.
///
/// # Safety
///
/// `states` is an array of `count` state pointers previously initialized by `state_init`.
unsafe extern "C" fn state_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<WordCountState>::destroy_callback(states, count) };
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Registers `word_count(VARCHAR) → BIGINT` on the given connection.
///
/// # Errors
///
/// Returns `ExtensionError` if DuckDB reports a registration failure.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
unsafe fn register_word_count(
    con: libduckdb_sys::duckdb_connection,
) -> Result<(), ExtensionError> {
    unsafe {
        AggregateFunctionBuilder::new("word_count")
            .param(TypeId::Varchar)
            .returns(TypeId::BigInt)
            .state_size(state_size)
            .init(state_init)
            .update(update)
            .combine(combine)
            .finalize(finalize)
            .destructor(state_destroy)
            .register(con)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// DuckDB extension entry point.
///
/// DuckDB calls this function when the extension is loaded. The symbol name
/// must be `{extension_name}_init_c_api` (all lowercase, underscores).
///
/// # Safety
///
/// Called by DuckDB. `info` and `access` are provided by the DuckDB runtime
/// and are valid for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn hello_ext_init_c_api(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
) -> bool {
    unsafe {
        quack_rs::entry_point::init_extension(
            info,
            access,
            quack_rs::DUCKDB_API_VERSION,
            |con| register_word_count(con),
        )
    }
}

// ---------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------

/// Counts whitespace-separated words in a string.
///
/// An empty string returns 0. Runs of whitespace are treated as a single
/// delimiter. Leading and trailing whitespace do not produce empty words.
///
/// # Examples
///
/// ```
/// # use hello_ext::count_words;
/// assert_eq!(count_words("hello world"), 2);
/// assert_eq!(count_words("  spaces  "), 1);
/// assert_eq!(count_words(""), 0);
/// assert_eq!(count_words("one"), 1);
/// ```
pub fn count_words(s: &str) -> i64 {
    s.split_whitespace().count() as i64
}

// ---------------------------------------------------------------------------
// Unit tests for pure logic
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use quack_rs::testing::AggregateTestHarness;

    #[test]
    fn count_words_basic() {
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words("one"), 1);
        assert_eq!(count_words(""), 0);
    }

    #[test]
    fn count_words_extra_whitespace() {
        assert_eq!(count_words("  hello  world  "), 2);
        assert_eq!(count_words("\t\nhello\tworld\n"), 2);
    }

    #[test]
    fn count_words_unicode() {
        assert_eq!(count_words("héllo wörld"), 2);
        assert_eq!(count_words("日本語 テスト"), 2);
    }

    #[test]
    fn word_count_state_accumulates() {
        let mut harness = AggregateTestHarness::<WordCountState>::new();
        harness.update(|s| s.count += count_words("hello world"));
        harness.update(|s| s.count += count_words("one two three"));
        harness.update(|s| s.count += count_words(""));
        let result = harness.finalize();
        assert_eq!(result.count, 5); // 2 + 3 + 0
    }

    #[test]
    fn word_count_combine() {
        let mut h1 = AggregateTestHarness::<WordCountState>::new();
        h1.update(|s| s.count += count_words("hello world"));

        let mut h2 = AggregateTestHarness::<WordCountState>::new();
        h2.update(|s| s.count += count_words("one two three four"));

        h2.combine(&h1, |src, tgt| tgt.count += src.count);

        let result = h2.finalize();
        assert_eq!(result.count, 6); // 2 + 4
    }

    #[test]
    fn word_count_aggregate_helper() {
        let inputs = ["hello world", "one", "two three four", ""];
        let result = AggregateTestHarness::<WordCountState>::aggregate(
            inputs,
            |s, text| s.count += count_words(text),
        );
        // 2 + 1 + 3 + 0 = 6
        assert_eq!(result.count, 6);
    }

    #[test]
    fn word_count_state_default_is_zero() {
        let h = AggregateTestHarness::<WordCountState>::new();
        assert_eq!(h.state().count, 0);
    }
}
