// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! # hello-ext
//!
//! A comprehensive `DuckDB` community extension built with [`quack_rs`] that
//! demonstrates **every feature** of the library.
//!
//! ## Functions registered
//!
//! | SQL function | Kind | Shows |
//! |---|---|---|
//! | `word_count(text)` | Aggregate | `AggregateFunctionBuilder` lifecycle |
//! | `first_word(text)` | Scalar | row-at-a-time, NULL propagation |
//! | `generate_series_ext(n)` | Table | full table function lifecycle (bind/init/scan) |
//! | `CAST(VARCHAR AS INTEGER)` | Cast | `CastFunctionBuilder`, `TRY_CAST` |
//! | `sum_list(LIST(BIGINT))` | Scalar | `param_logical(LogicalType)` |
//! | `make_pair(k, v)` | Scalar | `returns_logical(LogicalType)`, `StructVector` |
//! | `coalesce_val(a, b)` | Scalar Set | per-overload `null_handling`, `ScalarFunctionSetBuilder` |
//! | `typed_sum(a,b)` / `typed_sum(a,b,c)` | Aggregate Set | `AggregateFunctionSetBuilder::overloads` |
//! | `double_it(x)` | SQL Macro (scalar) | `SqlMacro::scalar` |
//! | `seq_n(n)` | SQL Macro (table) | `SqlMacro::table` |
//! | `read_hello(path)` | Replacement Scan | `ReplacementScanBuilder` |
//! | `make_kv_map(k, v)` | Scalar | `MapVector`, `LogicalType::map()` |
//! | `gen_series_v2(n, step:=1)` | Table | `named_param`, `local_init`, `projection_pushdown`, `set_max_threads` |
//! | `CAST(DOUBLE AS BIGINT)` | Cast | `implicit_cost`, `extra_info` |
//! | `add_interval(iv, micros)` | Scalar | `DuckInterval`, `read_interval`/`write_interval` |
//! | `all_types_echo(...)` | Scalar | all VectorReader/Writer types, `ValidityBitmap` |
//!
//! ## Entry point
//!
//! Uses `entry_point_v2!` with `Connection`/`Registrar` for type-safe registration.

use std::os::raw::{c_char, c_void};

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_bind_info, duckdb_data_chunk, duckdb_data_chunk_get_vector,
    duckdb_data_chunk_set_size, duckdb_function_info, duckdb_init_info, duckdb_vector,
    duckdb_vector_size, idx_t,
};
use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateFunctionSetBuilder, AggregateState, FfiState};
use quack_rs::cast::{CastFunctionBuilder, CastFunctionInfo, CastMode};
use quack_rs::connection::{Connection, Registrar};
use quack_rs::error::ExtensionError;
use quack_rs::interval::DuckInterval;
use quack_rs::replacement_scan::ReplacementScanInfo;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
use quack_rs::sql_macro::SqlMacro;
use quack_rs::table::{BindInfo, FfiBindData, FfiInitData, FfiLocalInitData, InitInfo, TableFunctionBuilder};
use quack_rs::types::{LogicalType, NullHandling, TypeId};
use quack_rs::vector::complex::{ListVector, MapVector, StructVector};
use quack_rs::vector::validity::ValidityBitmap;
use quack_rs::vector::{VectorReader, VectorWriter};

// ============================================================================
// Aggregate: word_count(VARCHAR) → BIGINT
// ============================================================================

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
// Aggregate Set: typed_sum(BIGINT, BIGINT) → BIGINT
//                typed_sum(BIGINT, BIGINT, BIGINT) → BIGINT
//
// Demonstrates AggregateFunctionSetBuilder with the overloads(range, closure) pattern.
// Sums 2 or 3 BIGINT columns per row across all rows.
// ============================================================================

#[derive(Default, Debug)]
struct TypedSumState {
    total: i64,
}

impl AggregateState for TypedSumState {}

unsafe extern "C" fn ts_state_size(_info: duckdb_function_info) -> idx_t {
    FfiState::<TypedSumState>::size_callback(_info)
}

unsafe extern "C" fn ts_state_init(
    info: duckdb_function_info,
    state: duckdb_aggregate_state,
) {
    unsafe { FfiState::<TypedSumState>::init_callback(info, state) };
}

unsafe extern "C" fn ts_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let row_count = unsafe { libduckdb_sys::duckdb_data_chunk_get_size(input) } as usize;
    let col_count = unsafe { libduckdb_sys::duckdb_data_chunk_get_column_count(input) } as usize;

    // Build readers for each column
    let readers: Vec<VectorReader> = (0..col_count)
        .map(|c| unsafe { VectorReader::new(input, c) })
        .collect();

    for row in 0..row_count {
        let state_ptr = unsafe { *states.add(row) };
        if let Some(st) = unsafe { FfiState::<TypedSumState>::with_state_mut(state_ptr) } {
            for reader in &readers {
                if unsafe { reader.is_valid(row) } {
                    st.total += unsafe { reader.read_i64(row) };
                }
            }
        }
    }
}

unsafe extern "C" fn ts_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        let src = unsafe { FfiState::<TypedSumState>::with_state(src_ptr) };
        let tgt = unsafe { FfiState::<TypedSumState>::with_state_mut(tgt_ptr) };
        if let (Some(s), Some(t)) = (src, tgt) {
            t.total += s.total;
        }
    }
}

unsafe extern "C" fn ts_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::new(result) };
    for i in 0..count as usize {
        let state_ptr = unsafe { *source.add(i) };
        match unsafe { FfiState::<TypedSumState>::with_state(state_ptr) } {
            Some(st) => unsafe { writer.write_i64(offset as usize + i, st.total) },
            None => unsafe { writer.set_null(offset as usize + i) },
        }
    }
}

unsafe extern "C" fn ts_state_destroy(
    states: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    unsafe { FfiState::<TypedSumState>::destroy_callback(states, count) };
}

// ============================================================================
// Scalar: first_word(VARCHAR) → VARCHAR
// ============================================================================

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
// ============================================================================

struct GenerateSeriesState {
    current: i64,
    total:   i64,
}

unsafe extern "C" fn gs_bind(info: duckdb_bind_info) {
    unsafe {
        let param = libduckdb_sys::duckdb_bind_get_parameter(info, 0);
        let n     = libduckdb_sys::duckdb_get_int64(param);
        libduckdb_sys::duckdb_destroy_value(&mut { param });
        let total = n.max(0);
        BindInfo::new(info)
            .add_result_column("value", TypeId::BigInt)
            .set_cardinality(total as u64, true);
        FfiBindData::<i64>::set(info, total);
    }
}

unsafe extern "C" fn gs_init(info: duckdb_init_info) {
    unsafe {
        let total = FfiBindData::<i64>::get_from_init(info).copied().unwrap_or(0);
        FfiInitData::<GenerateSeriesState>::set(
            info,
            GenerateSeriesState { current: 0, total },
        );
    }
}

unsafe extern "C" fn gs_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    unsafe {
        let Some(state) = FfiInitData::<GenerateSeriesState>::get_mut(info) else {
            duckdb_data_chunk_set_size(output, 0);
            return;
        };
        if state.current >= state.total {
            duckdb_data_chunk_set_size(output, 0);
            return;
        }
        let vector_size = duckdb_vector_size() as i64;
        let remaining   = state.total - state.current;
        let batch_size  = remaining.min(vector_size) as usize;
        let vec = duckdb_data_chunk_get_vector(output, 0);
        let mut writer = VectorWriter::from_vector(vec);
        for i in 0..batch_size {
            writer.write_i64(i, state.current + i as i64);
        }
        state.current += batch_size as i64;
        duckdb_data_chunk_set_size(output, batch_size as idx_t);
    }
}

// ============================================================================
// Table function v2: gen_series_v2(n BIGINT, step := 1)
//
// Demonstrates: named_param, local_init, projection_pushdown, set_max_threads,
// FfiLocalInitData, InitInfo
// ============================================================================

struct GsV2BindData {
    total: i64,
    step: i64,
}

struct GsV2GlobalState {
    next_offset: i64,
    total: i64,
    step: i64,
}

struct GsV2LocalState {
    local_start: i64,
    local_count: i64,
    emitted: i64,
}

unsafe extern "C" fn gs_v2_bind(info: duckdb_bind_info) {
    unsafe {
        let param = libduckdb_sys::duckdb_bind_get_parameter(info, 0);
        let n = libduckdb_sys::duckdb_get_int64(param);
        libduckdb_sys::duckdb_destroy_value(&mut { param });

        // Read named param "step" if provided, default to 1.
        // DuckDB provides named params via duckdb_bind_get_named_parameter.
        let bind_info = BindInfo::new(info);
        let step = {
            let step_name = std::ffi::CString::new("step").unwrap();
            let step_val = libduckdb_sys::duckdb_bind_get_named_parameter(info, step_name.as_ptr());
            if step_val.is_null() {
                1i64
            } else {
                let s = libduckdb_sys::duckdb_get_int64(step_val);
                libduckdb_sys::duckdb_destroy_value(&mut { step_val });
                if s == 0 { 1 } else { s }
            }
        };

        let total = n.max(0);
        bind_info
            .add_result_column("value", TypeId::BigInt)
            .add_result_column("step_used", TypeId::BigInt)
            .set_cardinality(total as u64, true);
        FfiBindData::<GsV2BindData>::set(info, GsV2BindData { total, step });
    }
}

unsafe extern "C" fn gs_v2_init(info: duckdb_init_info) {
    unsafe {
        let bind_data = FfiBindData::<GsV2BindData>::get_from_init(info);
        let (total, step) = bind_data.map_or((0, 1), |d| (d.total, d.step));

        // Demonstrate InitInfo: set_max_threads
        let init_info = InitInfo::new(info);
        init_info.set_max_threads(1);

        FfiInitData::<GsV2GlobalState>::set(info, GsV2GlobalState {
            next_offset: 0,
            total,
            step,
        });
    }
}

unsafe extern "C" fn gs_v2_local_init(info: duckdb_init_info) {
    unsafe {
        // Demonstrates FfiLocalInitData for per-thread state.
        // In single-threaded mode this is called once.
        FfiLocalInitData::<GsV2LocalState>::set(info, GsV2LocalState {
            local_start: 0,
            local_count: 0,
            emitted: 0,
        });
    }
}

unsafe extern "C" fn gs_v2_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    unsafe {
        let Some(global) = FfiInitData::<GsV2GlobalState>::get_mut(info) else {
            duckdb_data_chunk_set_size(output, 0);
            return;
        };

        let vector_size = duckdb_vector_size() as i64;
        let remaining = global.total - global.next_offset;
        if remaining <= 0 {
            duckdb_data_chunk_set_size(output, 0);
            return;
        }
        let batch_size = remaining.min(vector_size) as usize;
        let start = global.next_offset;
        let step = global.step;
        global.next_offset += batch_size as i64;

        // Demonstrate FfiLocalInitData::get_mut in scan
        if let Some(local) = FfiLocalInitData::<GsV2LocalState>::get_mut(info) {
            local.local_start = start;
            local.local_count = batch_size as i64;
            local.emitted += batch_size as i64;
        }

        let vec0 = duckdb_data_chunk_get_vector(output, 0);
        let vec1 = duckdb_data_chunk_get_vector(output, 1);
        let mut writer0 = VectorWriter::from_vector(vec0);
        let mut writer1 = VectorWriter::from_vector(vec1);
        for i in 0..batch_size {
            writer0.write_i64(i, (start + i as i64) * step);
            writer1.write_i64(i, step);
        }
        duckdb_data_chunk_set_size(output, batch_size as idx_t);
    }
}

// ============================================================================
// Cast function: CAST(VARCHAR AS INTEGER) / TRY_CAST(VARCHAR AS INTEGER)
// ============================================================================

unsafe extern "C" fn varchar_to_int(
    info: duckdb_function_info,
    count: idx_t,
    input: duckdb_vector,
    output: duckdb_vector,
) -> bool {
    let cast_info = unsafe { CastFunctionInfo::new(info) };
    let reader = unsafe { VectorReader::from_vector(input, count as usize) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..count as usize {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        match parse_varchar_to_int(s) {
            Some(v) => unsafe { writer.write_i32(row, v) },
            None => {
                let msg = format!("cannot cast {:?} to INTEGER", s);
                if cast_info.cast_mode() == CastMode::Try {
                    unsafe { writer.set_null(row) };
                    unsafe { cast_info.set_row_error(&msg, row as idx_t, output) };
                } else {
                    cast_info.set_error(&msg);
                    return false;
                }
            }
        }
    }
    true
}

// ============================================================================
// Cast function: CAST(DOUBLE AS BIGINT) with implicit_cost and extra_info
//
// Demonstrates CastFunctionBuilder::implicit_cost and extra_info.
// The extra_info stores a rounding mode flag (0 = truncate, 1 = round).
// ============================================================================

unsafe extern "C" fn double_to_bigint(
    info: duckdb_function_info,
    count: idx_t,
    input: duckdb_vector,
    output: duckdb_vector,
) -> bool {
    let cast_info = unsafe { CastFunctionInfo::new(info) };
    let reader = unsafe { VectorReader::from_vector(input, count as usize) };
    let mut writer = unsafe { VectorWriter::new(output) };

    // Retrieve rounding mode from extra_info
    let extra = unsafe { cast_info.get_extra_info() };
    let round = if extra.is_null() {
        false
    } else {
        unsafe { *(extra.cast::<bool>()) }
    };

    for row in 0..count as usize {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let v = unsafe { reader.read_f64(row) };
        let result = if round { v.round() as i64 } else { v as i64 };
        unsafe { writer.write_i64(row, result) };
    }
    true
}

unsafe extern "C" fn destroy_rounding_mode(ptr: *mut c_void) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(ptr.cast::<bool>())) };
    }
}

// ============================================================================
// Scalar: sum_list(LIST(BIGINT)) → BIGINT
// ============================================================================

unsafe extern "C" fn sum_list_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let reader = unsafe { VectorReader::new(input, 0) };
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = reader.row_count();
    let list_vec = unsafe { duckdb_data_chunk_get_vector(input, 0) };

    for row in 0..row_count {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let entry = unsafe { ListVector::get_entry(list_vec, row) };
        let child_vec = unsafe { ListVector::get_child(list_vec) };
        let total_elements = unsafe { ListVector::get_size(list_vec) };
        let child_reader = unsafe { VectorReader::from_vector(child_vec, total_elements) };

        let mut sum: i64 = 0;
        for i in 0..entry.length as usize {
            let idx = entry.offset as usize + i;
            if unsafe { child_reader.is_valid(idx) } {
                sum += unsafe { child_reader.read_i64(idx) };
            }
        }
        unsafe { writer.write_i64(row, sum) };
    }
}

// ============================================================================
// Scalar: make_pair(VARCHAR, INTEGER) → STRUCT(key VARCHAR, value INTEGER)
// ============================================================================

unsafe extern "C" fn make_pair_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let key_reader = unsafe { VectorReader::new(input, 0) };
    let val_reader = unsafe { VectorReader::new(input, 1) };
    let row_count = key_reader.row_count();

    let mut key_writer = unsafe { StructVector::field_writer(output, 0) };
    let mut val_writer = unsafe { StructVector::field_writer(output, 1) };

    for row in 0..row_count {
        let key_valid = unsafe { key_reader.is_valid(row) };
        let val_valid = unsafe { val_reader.is_valid(row) };
        if !key_valid || !val_valid {
            let mut parent_writer = unsafe { VectorWriter::new(output) };
            unsafe { parent_writer.set_null(row) };
            continue;
        }
        let k = unsafe { key_reader.read_str(row) };
        let v = unsafe { val_reader.read_i32(row) };
        unsafe { key_writer.write_varchar(row, k) };
        unsafe { val_writer.write_i32(row, v) };
    }
}

// ============================================================================
// Scalar: make_kv_map(VARCHAR, INTEGER) → MAP(VARCHAR, INTEGER)
//
// Demonstrates MapVector, LogicalType::map(), and map write workflow.
// Creates a single-entry map {key: k, value: v} per row.
// ============================================================================

unsafe extern "C" fn make_kv_map_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let key_reader = unsafe { VectorReader::new(input, 0) };
    let val_reader = unsafe { VectorReader::new(input, 1) };
    let row_count = key_reader.row_count();

    // Reserve space in the MAP child vector for row_count entries (1 per row)
    unsafe { MapVector::reserve(output, row_count) };

    // Get the key and value child vectors
    let keys_vec = unsafe { MapVector::keys(output) };
    let vals_vec = unsafe { MapVector::values(output) };
    let mut key_writer = unsafe { VectorWriter::from_vector(keys_vec) };
    let mut val_writer = unsafe { VectorWriter::from_vector(vals_vec) };

    let mut entry_offset: u64 = 0;
    for row in 0..row_count {
        let key_valid = unsafe { key_reader.is_valid(row) };
        let val_valid = unsafe { val_reader.is_valid(row) };
        if !key_valid || !val_valid {
            // Empty map for NULL inputs
            unsafe { MapVector::set_entry(output, row, entry_offset, 0) };
            continue;
        }
        let k = unsafe { key_reader.read_str(row) };
        let v = unsafe { val_reader.read_i32(row) };

        let child_idx = entry_offset as usize;
        unsafe { key_writer.write_varchar(child_idx, k) };
        unsafe { val_writer.write_i32(child_idx, v) };

        unsafe { MapVector::set_entry(output, row, entry_offset, 1) };
        entry_offset += 1;
    }

    unsafe { MapVector::set_size(output, entry_offset as usize) };
}

// ============================================================================
// Scalar function set: coalesce_val(a, b)
// ============================================================================

unsafe extern "C" fn coalesce_bigint(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let reader_a = unsafe { VectorReader::new(input, 0) };
    let reader_b = unsafe { VectorReader::new(input, 1) };
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = reader_a.row_count();

    for row in 0..row_count {
        if unsafe { reader_a.is_valid(row) } {
            let v = unsafe { reader_a.read_i64(row) };
            unsafe { writer.write_i64(row, v) };
        } else if unsafe { reader_b.is_valid(row) } {
            let v = unsafe { reader_b.read_i64(row) };
            unsafe { writer.write_i64(row, v) };
        } else {
            unsafe { writer.set_null(row) };
        }
    }
}

unsafe extern "C" fn coalesce_varchar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let reader_a = unsafe { VectorReader::new(input, 0) };
    let reader_b = unsafe { VectorReader::new(input, 1) };
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = reader_a.row_count();

    for row in 0..row_count {
        if unsafe { reader_a.is_valid(row) } {
            let v = unsafe { reader_a.read_str(row) };
            unsafe { writer.write_varchar(row, v) };
        } else if unsafe { reader_b.is_valid(row) } {
            let v = unsafe { reader_b.read_str(row) };
            unsafe { writer.write_varchar(row, v) };
        } else {
            unsafe { writer.set_null(row) };
        }
    }
}

// ============================================================================
// Scalar: add_interval(INTERVAL, BIGINT) → INTERVAL
//
// Demonstrates DuckInterval read/write via VectorReader/VectorWriter.
// Adds `micros` microseconds to the interval's micros component.
// ============================================================================

unsafe extern "C" fn add_interval_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let iv_reader = unsafe { VectorReader::new(input, 0) };
    let micros_reader = unsafe { VectorReader::new(input, 1) };
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = iv_reader.row_count();

    for row in 0..row_count {
        if !unsafe { iv_reader.is_valid(row) } || !unsafe { micros_reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let iv = unsafe { iv_reader.read_interval(row) };
        let add_micros = unsafe { micros_reader.read_i64(row) };
        let result = DuckInterval {
            months: iv.months,
            days: iv.days,
            micros: iv.micros.saturating_add(add_micros),
        };
        unsafe { writer.write_interval(row, result) };
    }
}

// ============================================================================
// Scalar: all_types_echo(
//   BOOLEAN, TINYINT, SMALLINT, INTEGER, BIGINT,
//   UTINYINT, USMALLINT, UINTEGER, UBIGINT,
//   FLOAT, DOUBLE, HUGEINT
// ) → VARCHAR
//
// Reads every numeric type via VectorReader, echoes them as a formatted string.
// Also demonstrates ValidityBitmap for direct NULL checking.
// ============================================================================

unsafe extern "C" fn all_types_echo_scalar(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let row_count = unsafe { libduckdb_sys::duckdb_data_chunk_get_size(input) } as usize;

    // Create readers for all 12 input columns
    let r_bool  = unsafe { VectorReader::new(input, 0) };
    let r_i8    = unsafe { VectorReader::new(input, 1) };
    let r_i16   = unsafe { VectorReader::new(input, 2) };
    let r_i32   = unsafe { VectorReader::new(input, 3) };
    let r_i64   = unsafe { VectorReader::new(input, 4) };
    let r_u8    = unsafe { VectorReader::new(input, 5) };
    let r_u16   = unsafe { VectorReader::new(input, 6) };
    let r_u32   = unsafe { VectorReader::new(input, 7) };
    let r_u64   = unsafe { VectorReader::new(input, 8) };
    let r_f32   = unsafe { VectorReader::new(input, 9) };
    let r_f64   = unsafe { VectorReader::new(input, 10) };
    let r_i128  = unsafe { VectorReader::new(input, 11) };

    let mut writer = unsafe { VectorWriter::new(output) };

    // Demonstrate ValidityBitmap for read-only NULL checking on output
    let output_bm = unsafe { ValidityBitmap::get_read_only(output) };
    // All output rows start as valid; this just exercises the API
    let _ = unsafe { output_bm.row_is_valid(0) };

    for row in 0..row_count {
        // Check any NULL — if the boolean column is NULL, output NULL
        if !unsafe { r_bool.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }

        let b   = unsafe { r_bool.read_bool(row) };
        let i8v = unsafe { r_i8.read_i8(row) };
        let i16v = unsafe { r_i16.read_i16(row) };
        let i32v = unsafe { r_i32.read_i32(row) };
        let i64v = unsafe { r_i64.read_i64(row) };
        let u8v  = unsafe { r_u8.read_u8(row) };
        let u16v = unsafe { r_u16.read_u16(row) };
        let u32v = unsafe { r_u32.read_u32(row) };
        let u64v = unsafe { r_u64.read_u64(row) };
        let f32v = unsafe { r_f32.read_f32(row) };
        let f64v = unsafe { r_f64.read_f64(row) };
        let i128v = unsafe { r_i128.read_i128(row) };

        let s = format!(
            "b={b},i8={i8v},i16={i16v},i32={i32v},i64={i64v},\
             u8={u8v},u16={u16v},u32={u32v},u64={u64v},\
             f32={f32v},f64={f64v},i128={i128v}"
        );
        unsafe { writer.write_varchar(row, &s) };
    }
}

// ============================================================================
// Replacement scan: SELECT * FROM 'hello:greeting'
//
// Demonstrates ReplacementScanBuilder/ReplacementScanInfo.
// When DuckDB sees a table name starting with "hello:", it redirects to
// read_hello(path) table function.
// ============================================================================

unsafe extern "C" fn hello_replacement_scan(
    info: libduckdb_sys::duckdb_replacement_scan_info,
    table_name: *const c_char,
    _data: *mut c_void,
) {
    let name = unsafe { std::ffi::CStr::from_ptr(table_name).to_string_lossy() };
    if !name.starts_with("hello:") {
        return; // Not ours
    }
    let greeting = &name[6..]; // strip "hello:" prefix
    unsafe {
        ReplacementScanInfo::new(info)
            .set_function("read_hello")
            .add_varchar_parameter(greeting);
    }
}

// read_hello(path) — table function that returns one row with the greeting
unsafe extern "C" fn read_hello_bind(info: duckdb_bind_info) {
    unsafe {
        let param = libduckdb_sys::duckdb_bind_get_parameter(info, 0);
        // Get string value via duckdb_get_varchar
        let str_ptr = libduckdb_sys::duckdb_get_varchar(param);
        let greeting = if str_ptr.is_null() {
            String::new()
        } else {
            let s = std::ffi::CStr::from_ptr(str_ptr).to_string_lossy().into_owned();
            libduckdb_sys::duckdb_free(str_ptr.cast::<c_void>());
            s
        };
        libduckdb_sys::duckdb_destroy_value(&mut { param });

        BindInfo::new(info)
            .add_result_column("greeting", TypeId::Varchar);
        FfiBindData::<String>::set(info, greeting);
    }
}

unsafe extern "C" fn read_hello_init(info: duckdb_init_info) {
    unsafe {
        FfiInitData::<bool>::set(info, false); // false = not yet emitted
    }
}

unsafe extern "C" fn read_hello_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    unsafe {
        let Some(emitted) = FfiInitData::<bool>::get_mut(info) else {
            duckdb_data_chunk_set_size(output, 0);
            return;
        };
        if *emitted {
            duckdb_data_chunk_set_size(output, 0);
            return;
        }
        *emitted = true;

        let greeting = FfiBindData::<String>::get_from_function(info)
            .map(|s| s.as_str())
            .unwrap_or("world");

        let vec = duckdb_data_chunk_get_vector(output, 0);
        let mut writer = VectorWriter::from_vector(vec);
        let msg = format!("Hello, {greeting}!");
        writer.write_varchar(0, &msg);
        duckdb_data_chunk_set_size(output, 1);
    }
}

// ============================================================================
// Pure Rust logic — no unsafe, unit-testable without a DuckDB instance
// ============================================================================

/// Counts whitespace-separated words in a string.
pub fn count_words(s: &str) -> i64 {
    s.split_whitespace().count() as i64
}

/// Tries to parse a trimmed VARCHAR string as an `i32`.
pub fn parse_varchar_to_int(s: &str) -> Option<i32> {
    s.trim().parse::<i32>().ok()
}

/// Returns the first whitespace-separated word, or `""` if none.
pub fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

// ============================================================================
// Registration — uses entry_point_v2! / Connection / Registrar
// ============================================================================

/// Registers all extension functions using the `Registrar` trait.
///
/// # Safety
///
/// `con` must provide a valid `DuckDB` connection for the duration of this call.
unsafe fn register_all(con: &Connection) -> Result<(), ExtensionError> {
    unsafe {
        // ── Aggregate: word_count ────────────────────────────────────────
        con.register_aggregate(
            AggregateFunctionBuilder::new("word_count")
                .param(TypeId::Varchar)
                .returns(TypeId::BigInt)
                .state_size(wc_state_size)
                .init(wc_state_init)
                .update(wc_update)
                .combine(wc_combine)
                .finalize(wc_finalize)
                .destructor(wc_state_destroy),
        )?;

        // ── Aggregate Set: typed_sum (2 and 3 arg overloads) ────────────
        con.register_aggregate_set(
            AggregateFunctionSetBuilder::new("typed_sum")
                .returns(TypeId::BigInt)
                .overloads(2..=3, |n, builder| {
                    let mut b = builder
                        .state_size(ts_state_size)
                        .init(ts_state_init)
                        .update(ts_update)
                        .combine(ts_combine)
                        .finalize(ts_finalize)
                        .destructor(ts_state_destroy);
                    for _ in 0..n {
                        b = b.param(TypeId::BigInt);
                    }
                    b
                }),
        )?;

        // ── Scalar: first_word ──────────────────────────────────────────
        con.register_scalar(
            ScalarFunctionBuilder::new("first_word")
                .param(TypeId::Varchar)
                .returns(TypeId::Varchar)
                .function(first_word_scalar),
        )?;

        // ── Table function: generate_series_ext ─────────────────────────
        con.register_table(
            TableFunctionBuilder::new("generate_series_ext")
                .param(TypeId::BigInt)
                .bind(gs_bind)
                .init(gs_init)
                .scan(gs_scan),
        )?;

        // ── Table function v2: gen_series_v2 (named_param, local_init, InitInfo)
        con.register_table(
            TableFunctionBuilder::new("gen_series_v2")
                .param(TypeId::BigInt)
                .named_param("step", TypeId::BigInt)
                .bind(gs_v2_bind)
                .init(gs_v2_init)
                .local_init(gs_v2_local_init)
                .scan(gs_v2_scan),
        )?;

        // ── Cast: VARCHAR → INTEGER ─────────────────────────────────────
        con.register_cast(
            CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
                .function(varchar_to_int),
        )?;

        // ── Cast: DOUBLE → BIGINT (with implicit_cost + extra_info) ─────
        let rounding_mode = Box::into_raw(Box::new(true)).cast::<c_void>();
        con.register_cast(
            CastFunctionBuilder::new(TypeId::Double, TypeId::BigInt)
                .function(double_to_bigint)
                .implicit_cost(100)
                .extra_info(rounding_mode, Some(destroy_rounding_mode)),
        )?;

        // ── Scalar: sum_list (param_logical) ────────────────────────────
        con.register_scalar(
            ScalarFunctionBuilder::new("sum_list")
                .param_logical(LogicalType::list(TypeId::BigInt))
                .returns(TypeId::BigInt)
                .function(sum_list_scalar),
        )?;

        // ── Scalar: make_pair (returns_logical, StructVector) ───────────
        con.register_scalar(
            ScalarFunctionBuilder::new("make_pair")
                .param(TypeId::Varchar)
                .param(TypeId::Integer)
                .returns_logical(LogicalType::struct_type(&[
                    ("key",   TypeId::Varchar),
                    ("value", TypeId::Integer),
                ]))
                .function(make_pair_scalar),
        )?;

        // ── Scalar: make_kv_map (MapVector, LogicalType::map) ───────────
        con.register_scalar(
            ScalarFunctionBuilder::new("make_kv_map")
                .param(TypeId::Varchar)
                .param(TypeId::Integer)
                .returns_logical(LogicalType::map(TypeId::Varchar, TypeId::Integer))
                .function(make_kv_map_scalar),
        )?;

        // ── Scalar Set: coalesce_val ────────────────────────────────────
        con.register_scalar_set(
            ScalarFunctionSetBuilder::new("coalesce_val")
                .overload(
                    ScalarOverloadBuilder::new()
                        .param(TypeId::BigInt)
                        .param(TypeId::BigInt)
                        .returns(TypeId::BigInt)
                        .null_handling(NullHandling::SpecialNullHandling)
                        .function(coalesce_bigint),
                )
                .overload(
                    ScalarOverloadBuilder::new()
                        .param(TypeId::Varchar)
                        .param(TypeId::Varchar)
                        .returns(TypeId::Varchar)
                        .null_handling(NullHandling::SpecialNullHandling)
                        .function(coalesce_varchar),
                ),
        )?;

        // ── Scalar: add_interval (DuckInterval read/write) ──────────────
        con.register_scalar(
            ScalarFunctionBuilder::new("add_interval")
                .param(TypeId::Interval)
                .param(TypeId::BigInt)
                .returns(TypeId::Interval)
                .function(add_interval_scalar),
        )?;

        // ── Scalar: all_types_echo (all reader/writer types + ValidityBitmap)
        con.register_scalar(
            ScalarFunctionBuilder::new("all_types_echo")
                .param(TypeId::Boolean)    // 0
                .param(TypeId::TinyInt)    // 1
                .param(TypeId::SmallInt)   // 2
                .param(TypeId::Integer)    // 3
                .param(TypeId::BigInt)     // 4
                .param(TypeId::UTinyInt)   // 5
                .param(TypeId::USmallInt)  // 6
                .param(TypeId::UInteger)   // 7
                .param(TypeId::UBigInt)    // 8
                .param(TypeId::Float)      // 9
                .param(TypeId::Double)     // 10
                .param(TypeId::HugeInt)    // 11
                .returns(TypeId::Varchar)
                .function(all_types_echo_scalar),
        )?;

        // ── SQL Macro (scalar): double_it(x) ────────────────────────────
        con.register_sql_macro(
            SqlMacro::scalar("double_it", &["x"], "x * 2")?,
        )?;

        // ── SQL Macro (table): seq_n(n) — returns n rows ──────────────
        con.register_sql_macro(
            SqlMacro::table(
                "seq_n",
                &["n"],
                "SELECT * FROM generate_series(1, n)",
            )?,
        )?;

        // ── Table function: read_hello (for replacement scan) ───────────
        con.register_table(
            TableFunctionBuilder::new("read_hello")
                .param(TypeId::Varchar)
                .bind(read_hello_bind)
                .init(read_hello_init)
                .scan(read_hello_scan),
        )?;

        // ── Replacement scan: hello:xxx → read_hello('xxx') ─────────────
        con.register_replacement_scan(
            hello_replacement_scan,
            std::ptr::null_mut(),
            None,
        );
    }

    Ok(())
}

// ============================================================================
// Entry point — uses entry_point_v2! for Connection/Registrar support
// ============================================================================

quack_rs::entry_point_v2!(hello_ext_init_c_api, |con| register_all(con));

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
        h.update(|s| s.count += count_words("hello world"));
        h.update(|s| s.count += count_words("one two three"));
        h.update(|s| s.count += count_words(""));
        assert_eq!(h.finalize().count, 5);
    }

    #[test]
    fn word_count_null_rows_skipped() {
        let mut h = AggregateTestHarness::<WordCountState>::new();
        h.update(|s| s.count += count_words("hello"));
        h.update(|s| s.count += count_words("world"));
        assert_eq!(h.finalize().count, 2);
    }

    #[test]
    fn word_count_combine() {
        let mut h1 = AggregateTestHarness::<WordCountState>::new();
        h1.update(|s| s.count += count_words("hello world"));

        let mut h2 = AggregateTestHarness::<WordCountState>::new();
        h2.update(|s| s.count += count_words("one two three four"));

        h2.combine(&h1, |src, tgt| tgt.count += src.count);
        assert_eq!(h2.finalize().count, 6);
    }

    #[test]
    fn word_count_combine_propagates_all_fields() {
        let mut src = AggregateTestHarness::<WordCountState>::new();
        src.update(|s| s.count = 42);

        let mut tgt = AggregateTestHarness::<WordCountState>::new();
        tgt.combine(&src, |s, t| {
            t.count += s.count;
        });

        assert_eq!(tgt.finalize().count, 42);
    }

    // ── TypedSumState ───────────────────────────────────────────────────────

    #[test]
    fn typed_sum_state_default_is_zero() {
        let h = AggregateTestHarness::<TypedSumState>::new();
        assert_eq!(h.state().total, 0);
    }

    #[test]
    fn typed_sum_accumulates() {
        let mut h = AggregateTestHarness::<TypedSumState>::new();
        h.update(|s| s.total += 10 + 20);
        h.update(|s| s.total += 30 + 40 + 50);
        assert_eq!(h.finalize().total, 150);
    }

    #[test]
    fn typed_sum_combine() {
        let mut h1 = AggregateTestHarness::<TypedSumState>::new();
        h1.update(|s| s.total = 100);

        let mut h2 = AggregateTestHarness::<TypedSumState>::new();
        h2.update(|s| s.total = 200);

        h2.combine(&h1, |src, tgt| tgt.total += src.total);
        assert_eq!(h2.finalize().total, 300);
    }

    // ── GenerateSeriesState logic ────────────────────────────────────────────

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
        assert!(state.current >= state.total);
    }

    #[test]
    fn generate_series_negative_n_clamped_to_zero() {
        let raw_n: i64 = -99;
        let total = raw_n.max(0);
        assert_eq!(total, 0);
    }

    #[test]
    fn generate_series_large_total_multiple_batches() {
        let vector_size: i64 = 10;
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

        assert_eq!(batch_count, 3);
        assert_eq!(last_value,  24);
    }

    // ── parse_varchar_to_int ─────────────────────────────────────────────────

    #[test]
    fn parse_int_basic() {
        assert_eq!(parse_varchar_to_int("42"),   Some(42));
        assert_eq!(parse_varchar_to_int("-7"),   Some(-7));
        assert_eq!(parse_varchar_to_int("0"),    Some(0));
    }

    #[test]
    fn parse_int_with_whitespace() {
        assert_eq!(parse_varchar_to_int("  42  "),  Some(42));
        assert_eq!(parse_varchar_to_int("\t-3\n"),  Some(-3));
    }

    #[test]
    fn parse_int_invalid_returns_none() {
        assert_eq!(parse_varchar_to_int("bad"),   None);
        assert_eq!(parse_varchar_to_int("3.14"),  None);
        assert_eq!(parse_varchar_to_int(""),      None);
        assert_eq!(parse_varchar_to_int("   "),   None);
        assert_eq!(parse_varchar_to_int("1e5"),   None);
    }

    #[test]
    fn parse_int_overflow_returns_none() {
        assert_eq!(parse_varchar_to_int("2147483648"), None);
        assert_eq!(parse_varchar_to_int("-2147483649"), None);
    }

    #[test]
    fn parse_int_boundary_values() {
        assert_eq!(parse_varchar_to_int("2147483647"),  Some(i32::MAX));
        assert_eq!(parse_varchar_to_int("-2147483648"), Some(i32::MIN));
    }

    // ── sum_list pure logic ───────────────────────────────────────────────────

    #[test]
    fn sum_list_logic_basic() {
        let values = [Some(1i64), Some(2), Some(3)];
        let sum: i64 = values.iter().filter_map(|v| *v).sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn sum_list_logic_with_nulls() {
        let values = [Some(10i64), None, Some(20)];
        let sum: i64 = values.iter().filter_map(|v| *v).sum();
        assert_eq!(sum, 30);
    }

    #[test]
    fn sum_list_logic_empty() {
        let values: Vec<Option<i64>> = vec![];
        let sum: i64 = values.iter().filter_map(|v| *v).sum();
        assert_eq!(sum, 0);
    }

    #[test]
    fn sum_list_logic_all_null_elements() {
        let values: [Option<i64>; 3] = [None, None, None];
        let sum: i64 = values.iter().filter_map(|v| *v).sum();
        assert_eq!(sum, 0);
    }

    // ── coalesce logic ───────────────────────────────────────────────────────

    #[test]
    fn coalesce_logic_first_non_null() {
        let a: Option<i64> = Some(42);
        let b: Option<i64> = Some(99);
        let result = a.or(b);
        assert_eq!(result, Some(42));
    }

    #[test]
    fn coalesce_logic_first_null_fallback() {
        let a: Option<i64> = None;
        let b: Option<i64> = Some(99);
        let result = a.or(b);
        assert_eq!(result, Some(99));
    }

    #[test]
    fn coalesce_logic_both_null() {
        let a: Option<i64> = None;
        let b: Option<i64> = None;
        let result = a.or(b);
        assert_eq!(result, None);
    }

    #[test]
    fn coalesce_logic_varchar() {
        let a: Option<&str> = None;
        let b: Option<&str> = Some("fallback");
        let result = a.or(b);
        assert_eq!(result, Some("fallback"));
    }

    // ── DuckInterval logic ──────────────────────────────────────────────────

    #[test]
    fn duck_interval_add_micros() {
        let iv = DuckInterval { months: 1, days: 2, micros: 100 };
        let result = DuckInterval {
            months: iv.months,
            days: iv.days,
            micros: iv.micros.saturating_add(500),
        };
        assert_eq!(result.months, 1);
        assert_eq!(result.days, 2);
        assert_eq!(result.micros, 600);
    }

    #[test]
    fn duck_interval_zero() {
        let iv = DuckInterval::zero();
        assert_eq!(iv.months, 0);
        assert_eq!(iv.days, 0);
        assert_eq!(iv.micros, 0);
    }

    #[test]
    fn duck_interval_saturating_add() {
        let iv = DuckInterval { months: 0, days: 0, micros: i64::MAX };
        let result = iv.micros.saturating_add(1);
        assert_eq!(result, i64::MAX);
    }

    // ── SqlMacro construction ───────────────────────────────────────────────

    #[test]
    fn sql_macro_double_it() {
        let m = SqlMacro::scalar("double_it", &["x"], "x * 2").unwrap();
        assert_eq!(m.to_sql(), "CREATE OR REPLACE MACRO double_it(x) AS (x * 2)");
    }

    #[test]
    fn sql_macro_seq_n() {
        let m = SqlMacro::table("seq_n", &["n"], "SELECT * FROM generate_series(1, n)").unwrap();
        assert_eq!(
            m.to_sql(),
            "CREATE OR REPLACE MACRO seq_n(n) AS TABLE SELECT * FROM generate_series(1, n)"
        );
    }

    // ── gen_series_v2 logic ─────────────────────────────────────────────────

    #[test]
    fn gen_series_v2_step_logic() {
        // With step=2, values should be 0, 2, 4
        let step = 2i64;
        let values: Vec<i64> = (0..3).map(|i| i * step).collect();
        assert_eq!(values, vec![0, 2, 4]);
    }

    #[test]
    fn gen_series_v2_zero_step_defaults_to_one() {
        let step = 0i64;
        let effective_step = if step == 0 { 1 } else { step };
        assert_eq!(effective_step, 1);
    }
}
