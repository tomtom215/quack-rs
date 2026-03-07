// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! # hello-ext
//!
//! A minimal `DuckDB` community extension built with [`quack_rs`].
//!
//! This example registers **two functions** that together demonstrate every
//! major pattern a real extension author needs:
//!
//! | SQL function | Kind | Input | Output | Shows |
//! |---|---|---|---|---|
//! | `word_count(text)` | Aggregate | `VARCHAR` | `BIGINT` | multi-row state, combine, finalize |
//! | `first_word(text)` | Scalar | `VARCHAR` | `VARCHAR` | row-at-a-time, NULL propagation |
//!
//! ## Quick start
//!
//! ```bash
//! cargo build --release
//! ```
//!
//! Then in DuckDB:
//!
//! ```sql
//! LOAD 'target/release/libhello_ext.so';   -- Linux
//! -- LOAD 'target/release/libhello_ext.dylib'; -- macOS
//!
//! SELECT word_count(sentence) FROM (
//!     VALUES ('hello world'), ('one two three'), (NULL)
//! ) t(sentence);
//! -- Returns 5  (2 + 3 + 0 for NULL)
//!
//! SELECT first_word(sentence) FROM (
//!     VALUES ('hello world'), ('  padded  '), (''), (NULL)
//! ) t(sentence);
//! -- Returns: 'hello', 'padded', '', NULL
//! ```
//!
//! ## Extension anatomy
//!
//! ```text
//! lib.rs
//! ├── WordCountState          — aggregate state struct
//! ├── update / combine / finalize — aggregate callbacks
//! ├── first_word_scalar       — scalar callback
//! ├── register()              — registers all functions on a connection
//! └── entry_point!()          — generates the C entry point DuckDB calls
//! ```
//!
//! See the [README](../README.md) and the
//! [quack-rs docs](https://docs.rs/quack-rs) for a full guide.

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};
use quack_rs::error::ExtensionError;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::TypeId;
use quack_rs::vector::{VectorReader, VectorWriter};

// ---------------------------------------------------------------------------
// Aggregate: word_count(VARCHAR) → BIGINT
//
// Pattern: multi-row accumulation with parallel combine.
// Pitfall L1 (combine): always copy *all* fields from source — not just
// the result field.  In a retention aggregate this might include window
// config; here WordCountState has only one field so it is trivial.
// ---------------------------------------------------------------------------

/// Accumulates the total word count across all input rows.
///
/// Words are whitespace-separated runs of non-whitespace characters.
/// `NULL` input rows contribute 0 words.
#[derive(Default, Debug)]
struct WordCountState {
    count: i64,
}

impl AggregateState for WordCountState {}

// --- Aggregate callbacks ---

/// Returns the number of bytes DuckDB must allocate per group.
///
/// # Safety
///
/// Called by DuckDB. `_info` may be null; we never dereference it here.
unsafe extern "C" fn wc_state_size(_info: duckdb_function_info) -> idx_t {
    FfiState::<WordCountState>::size_callback(_info)
}

/// Initialises a freshly allocated state.
///
/// # Safety
///
/// `state` points to exactly `wc_state_size()` bytes of writable memory
/// provided by DuckDB.
unsafe extern "C" fn wc_state_init(
    info: duckdb_function_info,
    state: duckdb_aggregate_state,
) {
    unsafe { FfiState::<WordCountState>::init_callback(info, state) };
}

/// Processes one data chunk, accumulating word counts.
///
/// `NULL` rows are silently skipped (they contribute 0 words).
///
/// # Safety
///
/// - `input` is a valid data chunk with one `VARCHAR` column.
/// - `states` is an array of `chunk_size` state pointers.
unsafe extern "C" fn wc_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    // SAFETY: input is a valid chunk provided by DuckDB; column 0 is VARCHAR.
    let reader = unsafe { VectorReader::new(input, 0) };
    let row_count = reader.row_count();

    for row in 0..row_count {
        // NULL input → skip (contributes 0 words to the aggregate)
        // SAFETY: row < row_count.
        if !unsafe { reader.is_valid(row) } {
            continue;
        }
        // SAFETY: row < row_count, column is VARCHAR.
        let s = unsafe { reader.read_str(row) };
        let words = count_words(s);

        // SAFETY: states is a valid array of row_count pointers.
        let state_ptr = unsafe { *states.add(row) };
        // SAFETY: state_ptr was initialised by wc_state_init.
        if let Some(st) = unsafe { FfiState::<WordCountState>::with_state_mut(state_ptr) } {
            st.count += words;
        }
    }
}

/// Merges source states into target states (used in parallel query plans).
///
/// # Pitfall L1 — copy *all* fields from source
///
/// DuckDB allocates fresh, zero-initialised target states before calling
/// combine.  You must copy every field — not just the result field.
/// For a more complex aggregate (e.g., a histogram with a width config),
/// failing to propagate config fields silently corrupts results.
///
/// # Safety
///
/// `source` and `target` are arrays of `count` valid state pointers.
unsafe extern "C" fn wc_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        // SAFETY: source and target hold count valid pointers.
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };

        let src = unsafe { FfiState::<WordCountState>::with_state(src_ptr) };
        let tgt = unsafe { FfiState::<WordCountState>::with_state_mut(tgt_ptr) };

        if let (Some(s), Some(t)) = (src, tgt) {
            t.count += s.count;
        }
    }
}

/// Writes final aggregate results into the output vector.
///
/// # Safety
///
/// - `source` is an array of `count` initialised state pointers.
/// - `result` is a valid `BIGINT` output vector.
unsafe extern "C" fn wc_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    // SAFETY: result is a valid output vector provided by DuckDB.
    let mut writer = unsafe { VectorWriter::new(result) };

    for i in 0..count as usize {
        // SAFETY: source holds count valid pointers.
        let state_ptr = unsafe { *source.add(i) };

        match unsafe { FfiState::<WordCountState>::with_state(state_ptr) } {
            Some(st) => {
                // SAFETY: offset + i is within the vector's capacity (DuckDB contract).
                unsafe { writer.write_i64(offset as usize + i, st.count) };
            }
            None => {
                // Null or uninitialised state — write NULL to output
                // SAFETY: offset + i is within the vector's capacity.
                unsafe { writer.set_null(offset as usize + i) };
            }
        }
    }
}

/// Frees heap-allocated state memory for each group.
///
/// # Safety
///
/// `states` is an array of `count` state pointers initialised by `wc_state_init`.
unsafe extern "C" fn wc_state_destroy(
    states: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    unsafe { FfiState::<WordCountState>::destroy_callback(states, count) };
}

// ---------------------------------------------------------------------------
// Scalar: first_word(VARCHAR) → VARCHAR
//
// Pattern: row-at-a-time processing with NULL propagation.
// The key rule: if the input row is NULL, write NULL to output and continue.
// ---------------------------------------------------------------------------

/// Returns the first whitespace-separated word of the input string.
///
/// `NULL` input → `NULL` output.  Empty or all-whitespace input → `''`.
///
/// # Safety
///
/// - `input` is a valid data chunk with one `VARCHAR` column.
/// - `output` is a valid `VARCHAR` output vector.
unsafe extern "C" fn first_word_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    // SAFETY: input is valid; column 0 is VARCHAR.
    let reader = unsafe { VectorReader::new(input, 0) };
    // SAFETY: output is a valid vector provided by DuckDB.
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = reader.row_count();

    for row in 0..row_count {
        // NULL input → NULL output.  Never read from an invalid row.
        // SAFETY: row < row_count.
        if !unsafe { reader.is_valid(row) } {
            // SAFETY: row < row_count, within the output vector's capacity.
            unsafe { writer.set_null(row) };
            continue;
        }

        // SAFETY: row is valid and column is VARCHAR.
        let s = unsafe { reader.read_str(row) };
        // SAFETY: row < row_count, within the output vector's capacity.
        unsafe { writer.write_varchar(row, first_word(s)) };
    }
}

// ---------------------------------------------------------------------------
// Pure logic (no unsafe — easy to unit-test without DuckDB)
// ---------------------------------------------------------------------------

/// Counts whitespace-separated words in a string.
///
/// Empty string or all-whitespace returns 0.
/// Leading and trailing whitespace does not produce empty words.
///
/// # Examples
///
/// ```
/// # use hello_ext::count_words;
/// assert_eq!(count_words("hello world"), 2);
/// assert_eq!(count_words("  spaces  "), 1);
/// assert_eq!(count_words(""), 0);
/// assert_eq!(count_words("   "), 0);
/// ```
pub fn count_words(s: &str) -> i64 {
    s.split_whitespace().count() as i64
}

/// Returns the first whitespace-separated word, or `""` if none exists.
///
/// # Examples
///
/// ```
/// # use hello_ext::first_word;
/// assert_eq!(first_word("hello world"), "hello");
/// assert_eq!(first_word("  padded  "), "padded");
/// assert_eq!(first_word(""), "");
/// assert_eq!(first_word("   "), "");
/// assert_eq!(first_word("one"), "one");
/// ```
pub fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Registers all extension functions on `con`.
///
/// Called by the entry point after the connection is established.
///
/// # Errors
///
/// Returns `ExtensionError` if DuckDB rejects any registration.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
unsafe fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        // Aggregate: word_count(VARCHAR) → BIGINT
        AggregateFunctionBuilder::new("word_count")
            .param(TypeId::Varchar)
            .returns(TypeId::BigInt)
            .state_size(wc_state_size)
            .init(wc_state_init)
            .update(wc_update)
            .combine(wc_combine)
            .finalize(wc_finalize)
            .destructor(wc_state_destroy)
            .register(con)?;

        // Scalar: first_word(VARCHAR) → VARCHAR
        ScalarFunctionBuilder::new("first_word")
            .param(TypeId::Varchar)
            .returns(TypeId::Varchar)
            .function(first_word_scalar)
            .register(con)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
//
// `entry_point!` generates the `#[no_mangle] pub unsafe extern "C"` symbol
// that DuckDB calls when the extension is loaded.  The symbol name MUST be
// `{extension_name}_init_c_api` — DuckDB looks it up by name.
// ---------------------------------------------------------------------------

quack_rs::entry_point!(hello_ext_init_c_api, |con| register(con));

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use quack_rs::testing::AggregateTestHarness;

    // ── count_words ────────────────────────────────────────────────────────

    #[test]
    fn count_words_basic() {
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words("one"), 1);
        assert_eq!(count_words(""), 0);
    }

    #[test]
    fn count_words_whitespace_variants() {
        assert_eq!(count_words("  hello  world  "), 2);
        assert_eq!(count_words("\t\nhello\tworld\n"), 2);
        assert_eq!(count_words("   "), 0); // all whitespace → 0
    }

    #[test]
    fn count_words_unicode() {
        assert_eq!(count_words("héllo wörld"), 2);
        assert_eq!(count_words("日本語 テスト"), 2);
    }

    // ── first_word ─────────────────────────────────────────────────────────

    #[test]
    fn first_word_basic() {
        assert_eq!(first_word("hello world"), "hello");
        assert_eq!(first_word("one"), "one");
    }

    #[test]
    fn first_word_empty_and_whitespace() {
        assert_eq!(first_word(""), "");
        assert_eq!(first_word("   "), "");
        assert_eq!(first_word("\t\n"), "");
    }

    #[test]
    fn first_word_leading_whitespace() {
        assert_eq!(first_word("  padded  "), "padded");
        assert_eq!(first_word("\t\nhello\tworld"), "hello");
    }

    #[test]
    fn first_word_single_word() {
        assert_eq!(first_word("only"), "only");
    }

    #[test]
    fn first_word_unicode() {
        assert_eq!(first_word("héllo wörld"), "héllo");
        assert_eq!(first_word("日本語 テスト"), "日本語");
    }

    // ── word_count aggregate state via AggregateTestHarness ───────────────

    #[test]
    fn word_count_state_default_is_zero() {
        let h = AggregateTestHarness::<WordCountState>::new();
        assert_eq!(h.state().count, 0);
    }

    #[test]
    fn word_count_accumulates() {
        let mut h = AggregateTestHarness::<WordCountState>::new();
        h.update(|s| s.count += count_words("hello world")); // 2
        h.update(|s| s.count += count_words("one two three")); // 3
        h.update(|s| s.count += count_words("")); // 0 (empty string)
        // Simulates NULL: skip the update (callback skips invalid rows)
        let result = h.finalize();
        assert_eq!(result.count, 5);
    }

    #[test]
    fn word_count_null_rows_are_skipped() {
        // In the real callback, NULL rows are detected via reader.is_valid()
        // and the update is skipped entirely.  Here we verify that skipping
        // has no effect on the accumulator.
        let mut h = AggregateTestHarness::<WordCountState>::new();
        h.update(|s| s.count += count_words("hello"));
        // Do not call h.update() for the NULL row — models callback skip
        h.update(|s| s.count += count_words("world"));
        assert_eq!(h.finalize().count, 2);
    }

    #[test]
    fn word_count_all_null_rows_yield_zero() {
        // If every row is NULL, the accumulator is never touched → 0.
        let h = AggregateTestHarness::<WordCountState>::new();
        assert_eq!(h.finalize().count, 0);
    }

    #[test]
    fn word_count_combine() {
        let mut h1 = AggregateTestHarness::<WordCountState>::new();
        h1.update(|s| s.count += count_words("hello world")); // 2

        let mut h2 = AggregateTestHarness::<WordCountState>::new();
        h2.update(|s| s.count += count_words("one two three four")); // 4

        h2.combine(&h1, |src, tgt| tgt.count += src.count);
        assert_eq!(h2.finalize().count, 6);
    }

    #[test]
    fn word_count_aggregate_helper() {
        let inputs = ["hello world", "one", "two three four", ""];
        let result = AggregateTestHarness::<WordCountState>::aggregate(
            inputs,
            |s, text| s.count += count_words(text),
        );
        assert_eq!(result.count, 6); // 2 + 1 + 3 + 0
    }

    #[test]
    fn word_count_combine_propagates_all_fields() {
        // Guard against Pitfall L1: combine must copy *all* state fields.
        // For WordCountState there is only one field, but the test makes
        // the requirement explicit so anyone adding a second field notices.
        let mut src = AggregateTestHarness::<WordCountState>::new();
        src.update(|s| s.count = 42);

        let mut tgt = AggregateTestHarness::<WordCountState>::new();
        tgt.combine(&src, |s, t| {
            t.count += s.count;
            // If you add fields to WordCountState, combine them here too.
        });

        assert_eq!(tgt.finalize().count, 42);
    }
}
