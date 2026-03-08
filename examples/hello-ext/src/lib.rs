// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! # hello-ext
//!
//! A minimal `DuckDB` community extension built with [`quack_rs`].
//!
//! This example registers **three functions** that together demonstrate every
//! major pattern a real extension author needs:
//!
//! | SQL function | Kind | Input | Output | Shows |
//! |---|---|---|---|---|
//! | `word_count(text)` | Aggregate | `VARCHAR` | `BIGINT` | multi-row state, combine, finalize |
//! | `first_word(text)` | Scalar | `VARCHAR` | `VARCHAR` | row-at-a-time, NULL propagation |
//! | `generate_series_ext(n)` | Table | `BIGINT` | `BIGINT` | full table function lifecycle (bind/init/scan) |
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
//!
//! SELECT * FROM generate_series_ext(5);
//! -- Returns rows: 0, 1, 2, 3, 4
//!
//! SELECT * FROM generate_series_ext(0);
//! -- Returns 0 rows
//! ```
//!
//! ## Extension anatomy
//!
//! ```text
//! lib.rs
//! ├── WordCountState              — aggregate state struct
//! ├── wc_update/combine/finalize  — aggregate callbacks
//! ├── first_word_scalar           — scalar callback
//! ├── GenerateSeriesState         — table function scan state struct
//! ├── gs_bind / gs_init / gs_scan — table function callbacks
//! ├── register()                  — registers all functions on a connection
//! └── entry_point!()              — generates the C entry point DuckDB calls
//! ```
//!
//! See the [README](../README.md) and the
//! [quack-rs docs](https://docs.rs/quack-rs) for a full guide.

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_bind_info, duckdb_data_chunk, duckdb_data_chunk_get_vector,
    duckdb_data_chunk_set_size, duckdb_function_info, duckdb_init_info, duckdb_vector,
    duckdb_vector_size, idx_t,
};
use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};
use quack_rs::error::ExtensionError;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::table::{BindInfo, FfiBindData, FfiInitData, TableFunctionBuilder};
use quack_rs::types::TypeId;
use quack_rs::vector::{VectorReader, VectorWriter};

// ============================================================================
// Aggregate: word_count(VARCHAR) → BIGINT
// ============================================================================

/// Accumulates the total word count across all input rows.
///
/// Words are whitespace-separated runs of non-whitespace characters.
/// `NULL` input rows contribute 0 words.
#[derive(Default, Debug)]
struct WordCountState {
    count: i64,
}

impl AggregateState for WordCountState {}

unsafe extern "C" fn wc_state_size(_info: duckdb_function_info) -> idx_t {
    FfiState::<WordCountState>::size_callback(_info)
}

unsafe extern "C" fn wc_state_init(
    info: duckdb_function_info,
    state: duckdb_aggregate_state,
) {
    unsafe { FfiState::<WordCountState>::init_callback(info, state) };
}

unsafe extern "C" fn wc_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    // SAFETY: input is a valid chunk; column 0 is VARCHAR.
    let reader = unsafe { VectorReader::new(input, 0) };
    let row_count = reader.row_count();
    for row in 0..row_count {
        if !unsafe { reader.is_valid(row) } {
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        let words = count_words(s);
        let state_ptr = unsafe { *states.add(row) };
        if let Some(st) = unsafe { FfiState::<WordCountState>::with_state_mut(state_ptr) } {
            st.count += words;
        }
    }
}

/// Merges source states into target states (for parallel query plans).
///
/// # Pitfall L1: copy *all* fields from source.
///
/// DuckDB allocates fresh, zero-initialised targets before calling combine.
/// Every field must be merged — not just the result field.
unsafe extern "C" fn wc_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        let src = unsafe { FfiState::<WordCountState>::with_state(src_ptr) };
        let tgt = unsafe { FfiState::<WordCountState>::with_state_mut(tgt_ptr) };
        if let (Some(s), Some(t)) = (src, tgt) {
            t.count += s.count;
        }
    }
}

unsafe extern "C" fn wc_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::new(result) };
    for i in 0..count as usize {
        let state_ptr = unsafe { *source.add(i) };
        match unsafe { FfiState::<WordCountState>::with_state(state_ptr) } {
            Some(st) => unsafe { writer.write_i64(offset as usize + i, st.count) },
            None => unsafe { writer.set_null(offset as usize + i) },
        }
    }
}

unsafe extern "C" fn wc_state_destroy(
    states: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    unsafe { FfiState::<WordCountState>::destroy_callback(states, count) };
}

// ============================================================================
// Scalar: first_word(VARCHAR) → VARCHAR
// ============================================================================

/// Returns the first whitespace-separated word of the input, or `""`.
/// `NULL` input → `NULL` output.
unsafe extern "C" fn first_word_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let reader = unsafe { VectorReader::new(input, 0) };
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = reader.row_count();
    for row in 0..row_count {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        unsafe { writer.write_varchar(row, first_word(s)) };
    }
}

// ============================================================================
// Table function: generate_series_ext(n BIGINT) → TABLE(value BIGINT)
//
// DuckDB table function lifecycle (see quack_rs::table for full docs):
//
//   bind  — read parameters, declare output columns, store bind data
//   init  — allocate global scan state (one per query)
//   scan  — fill one output chunk per call; set size=0 to signal end
//
// This function is intentionally single-threaded (no local_init) to keep
// the example focused. Add local_init + set_max_threads for parallel scans.
// ============================================================================

/// Scan state for `generate_series_ext`.
struct GenerateSeriesState {
    current: i64,
    total:   i64,
}

/// Bind callback: reads `n`, declares output schema, stores bind data.
///
/// # Safety
///
/// `info` must be a valid `duckdb_bind_info` provided by DuckDB.
unsafe extern "C" fn gs_bind(info: duckdb_bind_info) {
    unsafe {
        // ── Step 1: Read the first positional parameter (n BIGINT). ──────
        // duckdb_bind_get_parameter returns a duckdb_value that we own;
        // we must call duckdb_destroy_value when done.
        let param = libduckdb_sys::duckdb_bind_get_parameter(info, 0);
        let n     = libduckdb_sys::duckdb_get_int64(param);
        libduckdb_sys::duckdb_destroy_value(&mut { param });

        // Clamp: we cannot produce a negative number of rows.
        let total = n.max(0);

        // ── Step 2: Declare output schema and hint cardinality. ──────────
        BindInfo::new(info)
            .add_result_column("value", TypeId::BigInt)
            .set_cardinality(total as u64, /* is_exact */ true);

        // ── Step 3: Store bind data so init can read it. ─────────────────
        FfiBindData::<i64>::set(info, total);
    }
}

/// Init callback: allocates the scan state.
///
/// # Safety
///
/// `info` must be a valid `duckdb_init_info`.
unsafe extern "C" fn gs_init(info: duckdb_init_info) {
    unsafe {
        // Read the total row count that bind stored.
        let total = FfiBindData::<i64>::get_from_init(info).copied().unwrap_or(0);
        FfiInitData::<GenerateSeriesState>::set(
            info,
            GenerateSeriesState { current: 0, total },
        );
    }
}

/// Scan callback: fills one output chunk per call.
///
/// DuckDB calls this repeatedly until chunk size is set to 0.
///
/// # Safety
///
/// - `info` must be a valid `duckdb_function_info`.
/// - `output` must be a valid output `duckdb_data_chunk`.
unsafe extern "C" fn gs_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    unsafe {
        let Some(state) = FfiInitData::<GenerateSeriesState>::get_mut(info) else {
            // Should never happen: defensive end-of-stream.
            duckdb_data_chunk_set_size(output, 0);
            return;
        };

        if state.current >= state.total {
            // All rows emitted — signal end of stream.
            duckdb_data_chunk_set_size(output, 0);
            return;
        }

        // Emit min(remaining, STANDARD_VECTOR_SIZE) rows.
        let vector_size = duckdb_vector_size() as i64;
        let remaining   = state.total - state.current;
        let batch_size  = remaining.min(vector_size) as usize;

        // Get the output vector for column 0 ("value", BIGINT).
        let vec = duckdb_data_chunk_get_vector(output, 0);

        // VectorWriter::from_vector lets us write into an already-obtained child vector.
        let mut writer = VectorWriter::from_vector(vec);
        for i in 0..batch_size {
            // SAFETY: i < batch_size ≤ STANDARD_VECTOR_SIZE.
            writer.write_i64(i, state.current + i as i64);
        }

        state.current += batch_size as i64;
        duckdb_data_chunk_set_size(output, batch_size as idx_t);
    }
}

// ============================================================================
// Pure Rust logic — no unsafe, unit-testable without a DuckDB instance
// ============================================================================

/// Counts whitespace-separated words in a string.
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

/// Returns the first whitespace-separated word, or `""` if none.
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

// ============================================================================
// Registration
// ============================================================================

/// Registers all extension functions on `con`.
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
        // Aggregate
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

        // Scalar
        ScalarFunctionBuilder::new("first_word")
            .param(TypeId::Varchar)
            .returns(TypeId::Varchar)
            .function(first_word_scalar)
            .register(con)?;

        // Table function
        TableFunctionBuilder::new("generate_series_ext")
            .param(TypeId::BigInt)
            .bind(gs_bind)
            .init(gs_init)
            .scan(gs_scan)
            .register(con)?;
    }

    Ok(())
}

// ============================================================================
// Entry point
//
// `entry_point!` generates the `#[no_mangle] pub unsafe extern "C"` symbol
// DuckDB calls when loading the extension. The symbol must be
// `{extension_name}_init_c_api` — DuckDB locates it by name.
// ============================================================================

quack_rs::entry_point!(hello_ext_init_c_api, |con| register(con));

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use quack_rs::testing::AggregateTestHarness;

    // ── count_words ─────────────────────────────────────────────────────────

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
        assert_eq!(count_words("   "), 0);
    }

    #[test]
    fn count_words_unicode() {
        assert_eq!(count_words("héllo wörld"), 2);
        assert_eq!(count_words("日本語 テスト"), 2);
    }

    // ── first_word ──────────────────────────────────────────────────────────

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
    fn first_word_unicode() {
        assert_eq!(first_word("héllo wörld"), "héllo");
        assert_eq!(first_word("日本語 テスト"), "日本語");
    }

    // ── WordCountState via AggregateTestHarness ─────────────────────────────

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
        h.update(|s| s.count += count_words("")); // 0
        assert_eq!(h.finalize().count, 5);
    }

    #[test]
    fn word_count_null_rows_skipped() {
        let mut h = AggregateTestHarness::<WordCountState>::new();
        h.update(|s| s.count += count_words("hello"));
        // NULL row: skipped (no update call)
        h.update(|s| s.count += count_words("world"));
        assert_eq!(h.finalize().count, 2);
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
    fn word_count_combine_propagates_all_fields() {
        // Guards against Pitfall L1: combine must copy *all* state fields.
        let mut src = AggregateTestHarness::<WordCountState>::new();
        src.update(|s| s.count = 42);

        let mut tgt = AggregateTestHarness::<WordCountState>::new();
        tgt.combine(&src, |s, t| {
            t.count += s.count;
        });

        assert_eq!(tgt.finalize().count, 42);
    }

    // ── GenerateSeriesState logic ────────────────────────────────────────────
    // These tests exercise the pure scan logic without calling DuckDB FFI.

    #[test]
    fn generate_series_emits_correct_values() {
        let mut state = GenerateSeriesState { current: 0, total: 5 };
        let vector_size: i64 = 2048;
        let mut emitted: Vec<i64> = Vec::new();

        loop {
            if state.current >= state.total {
                break;
            }
            let remaining  = state.total - state.current;
            let batch_size = remaining.min(vector_size) as usize;
            for i in 0..batch_size {
                emitted.push(state.current + i as i64);
            }
            state.current += batch_size as i64;
        }

        assert_eq!(emitted, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn generate_series_zero_total_emits_nothing() {
        let state = GenerateSeriesState { current: 0, total: 0 };
        // current >= total immediately — no rows emitted.
        assert!(state.current >= state.total);
    }

    #[test]
    fn generate_series_negative_n_clamped_to_zero() {
        // gs_bind calls n.max(0) before storing bind data.
        let raw_n: i64 = -99;
        let total = raw_n.max(0);
        assert_eq!(total, 0);
    }

    #[test]
    fn generate_series_large_total_multiple_batches() {
        let vector_size: i64 = 10; // small batch for test
        let mut state = GenerateSeriesState { current: 0, total: 25 };
        let mut batch_count = 0usize;
        let mut last_value  = -1i64;

        loop {
            if state.current >= state.total {
                break;
            }
            let remaining  = state.total - state.current;
            let batch_size = remaining.min(vector_size) as usize;
            last_value = state.current + batch_size as i64 - 1;
            state.current += batch_size as i64;
            batch_count += 1;
        }

        assert_eq!(batch_count, 3); // 10 + 10 + 5
        assert_eq!(last_value,  24);
    }
}
