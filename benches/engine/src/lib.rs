// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Benchmark extension: every `q_*` function is written with quack-rs, every
//! `r_*` function is the hand-written equivalent against `libduckdb-sys` alone.
//!
//! The raw twins are written the way a careful author would write them: pointers
//! hoisted out of the loop, validity tested as a bit, one loop for the no-NULL
//! case. They are the floor the SDK is compared with, not a recommendation to
//! avoid the SDK. Each pair returns the same values (`compare.py --check`
//! verifies it), so the comparison is of cost, not of behaviour.
#![allow(clippy::all, clippy::pedantic, clippy::nursery, missing_docs)]

use std::ffi::{c_char, CString};

use libduckdb_sys::*;
use quack_rs::connection::{Connection, Registrar};
use quack_rs::data_chunk::DataChunk;
use quack_rs::error::ExtensionError;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::ListBuilder;

unsafe fn raw_scalar(
    con: duckdb_connection,
    name: &str,
    params: &[duckdb_logical_type],
    ret: duckdb_logical_type,
    f: unsafe extern "C" fn(duckdb_function_info, duckdb_data_chunk, duckdb_vector),
) {
    let mut sf = duckdb_create_scalar_function();
    let cname = CString::new(name).unwrap();
    duckdb_scalar_function_set_name(sf, cname.as_ptr());
    for p in params {
        duckdb_scalar_function_add_parameter(sf, *p);
    }
    duckdb_scalar_function_set_return_type(sf, ret);
    duckdb_scalar_function_set_function(sf, Some(f));
    assert_eq!(duckdb_register_scalar_function(con, sf), duckdb_state_DuckDBSuccess, "register {name}");
    duckdb_destroy_scalar_function(&mut sf);
}

#[inline(always)]
unsafe fn bit_is_valid(validity: *const u64, i: usize) -> bool {
    (*validity.add(i >> 6) >> (i & 63)) & 1 != 0
}

#[inline(always)]
unsafe fn bit_set_invalid(validity: *mut u64, i: usize) {
    *validity.add(i >> 6) &= !(1u64 << (i & 63));
}

/// The bytes of row `i` of a VARCHAR vector's data.
#[inline(always)]
unsafe fn duck_str<'a>(data: *const u8, i: usize) -> &'a [u8] {
    let p = data.add(i * 16);
    let len = (p as *const u32).read_unaligned() as usize;
    if len <= 12 {
        std::slice::from_raw_parts(p.add(4), len)
    } else {
        let ptr = (p.add(8) as *const *const u8).read_unaligned();
        std::slice::from_raw_parts(ptr, len)
    }
}

// --- i64 -> i64, f(x) = 3x + 1 (nullable and not) ---------------------------

#[inline(always)]
fn f_inc(x: i64) -> i64 {
    x.wrapping_mul(3).wrapping_add(1)
}

unsafe extern "C" fn r_inc(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
    let n = duckdb_data_chunk_get_size(input) as usize;
    let v = duckdb_data_chunk_get_vector(input, 0);
    let src = duckdb_vector_get_data(v) as *const i64;
    let valid = duckdb_vector_get_validity(v);
    let dst = duckdb_vector_get_data(output) as *mut i64;
    if valid.is_null() {
        for i in 0..n {
            *dst.add(i) = f_inc(*src.add(i));
        }
    } else {
        duckdb_vector_ensure_validity_writable(output);
        let ov = duckdb_vector_get_validity(output);
        for i in 0..n {
            if bit_is_valid(valid, i) {
                *dst.add(i) = f_inc(*src.add(i));
            } else {
                bit_set_invalid(ov, i);
            }
        }
    }
}

// --- (i64, i64) -> i64, f(a, b) = 2a + b ------------------------------------

#[inline(always)]
fn f_add(a: i64, b: i64) -> i64 {
    a.wrapping_mul(2).wrapping_add(b)
}

unsafe extern "C" fn r_add(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
    let n = duckdb_data_chunk_get_size(input) as usize;
    let va = duckdb_data_chunk_get_vector(input, 0);
    let vb = duckdb_data_chunk_get_vector(input, 1);
    let a = duckdb_vector_get_data(va) as *const i64;
    let b = duckdb_vector_get_data(vb) as *const i64;
    let validity_a = duckdb_vector_get_validity(va);
    let validity_b = duckdb_vector_get_validity(vb);
    let dst = duckdb_vector_get_data(output) as *mut i64;
    if validity_a.is_null() && validity_b.is_null() {
        for i in 0..n {
            *dst.add(i) = f_add(*a.add(i), *b.add(i));
        }
    } else {
        duckdb_vector_ensure_validity_writable(output);
        let ov = duckdb_vector_get_validity(output);
        for i in 0..n {
            let ok = (validity_a.is_null() || bit_is_valid(validity_a, i))
                && (validity_b.is_null() || bit_is_valid(validity_b, i));
            if ok {
                *dst.add(i) = f_add(*a.add(i), *b.add(i));
            } else {
                bit_set_invalid(ov, i);
            }
        }
    }
}

// --- VARCHAR -> BIGINT (count of b'a') and VARCHAR -> VARCHAR (trim) --------

#[inline(always)]
fn count_a(s: &[u8]) -> i64 {
    s.iter().filter(|&&b| b == b'a').count() as i64
}

unsafe extern "C" fn r_len(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
    let n = duckdb_data_chunk_get_size(input) as usize;
    let v = duckdb_data_chunk_get_vector(input, 0);
    let src = duckdb_vector_get_data(v) as *const u8;
    let valid = duckdb_vector_get_validity(v);
    let dst = duckdb_vector_get_data(output) as *mut i64;
    let mut ov: *mut u64 = std::ptr::null_mut();
    for i in 0..n {
        if valid.is_null() || bit_is_valid(valid, i) {
            *dst.add(i) = count_a(duck_str(src, i));
        } else {
            if ov.is_null() {
                duckdb_vector_ensure_validity_writable(output);
                ov = duckdb_vector_get_validity(output);
            }
            bit_set_invalid(ov, i);
        }
    }
}

/// Trims without allocating: the slice goes straight into `assign_string_element_len`.
/// The SDK's `map1_str` returns an owned `String`, so it allocates once per row; the
/// gap between `q_trim` and this function is that allocation plus the UTF-8 check.
unsafe extern "C" fn r_trim(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
    let n = duckdb_data_chunk_get_size(input) as usize;
    let v = duckdb_data_chunk_get_vector(input, 0);
    let src = duckdb_vector_get_data(v) as *const u8;
    let valid = duckdb_vector_get_validity(v);
    let mut ov: *mut u64 = std::ptr::null_mut();
    for i in 0..n {
        if valid.is_null() || bit_is_valid(valid, i) {
            let t = duck_str(src, i).trim_ascii();
            duckdb_vector_assign_string_element_len(output, i as idx_t, t.as_ptr() as *const c_char, t.len() as idx_t);
        } else {
            if ov.is_null() {
                duckdb_vector_ensure_validity_writable(output);
                ov = duckdb_vector_get_validity(output);
            }
            bit_set_invalid(ov, i);
        }
    }
}

// --- BIGINT -> BIGINT[]: [x, x+1, x+2], and with a NULL fourth element ------

quack_rs::scalar_callback!(q_list3, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let r = unsafe { chunk.reader(0) };
    let mut lb = unsafe { ListBuilder::new(output) };
    for row in 0..chunk.size() {
        let x = unsafe { r.read_i64(row) };
        unsafe {
            lb.push_row(row, 3, |w, base| {
                w.write_i64(base, x);
                w.write_i64(base + 1, x + 1);
                w.write_i64(base + 2, x + 2);
            })
        };
    }
    unsafe { lb.finish() };
});

quack_rs::scalar_callback!(q_list4n, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let r = unsafe { chunk.reader(0) };
    let mut lb = unsafe { ListBuilder::new(output) };
    for row in 0..chunk.size() {
        let x = unsafe { r.read_i64(row) };
        unsafe {
            lb.push_row(row, 4, |w, base| {
                w.write_i64(base, x);
                w.write_i64(base + 1, x + 1);
                w.write_i64(base + 2, x + 2);
                w.set_null(base + 3);
            })
        };
    }
    unsafe { lb.finish() };
});

/// Grows the child geometrically like `ListBuilder` does, re-fetching the
/// pointers only after a reserve.
unsafe extern "C" fn r_list3_grow(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
    let n = duckdb_data_chunk_get_size(input) as usize;
    let v = duckdb_data_chunk_get_vector(input, 0);
    let src = duckdb_vector_get_data(v) as *const i64;
    let entries = duckdb_vector_get_data(output) as *mut duckdb_list_entry;
    let mut reserved = 0usize;
    let mut child = duckdb_list_vector_get_child(output);
    let mut cd = duckdb_vector_get_data(child) as *mut i64;
    let mut written = 0usize;
    for i in 0..n {
        let need = written + 3;
        if need > reserved {
            reserved = need.next_power_of_two();
            duckdb_list_vector_reserve(output, reserved as idx_t);
            child = duckdb_list_vector_get_child(output);
            cd = duckdb_vector_get_data(child) as *mut i64;
        }
        let x = *src.add(i);
        *cd.add(written) = x;
        *cd.add(written + 1) = x + 1;
        *cd.add(written + 2) = x + 2;
        *entries.add(i) = duckdb_list_entry { offset: written as u64, length: 3 };
        written = need;
    }
    duckdb_list_vector_set_size(output, written as idx_t);
}

/// Four elements per row, the last NULL; the total is known, so one reserve.
unsafe extern "C" fn r_list4n(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
    let n = duckdb_data_chunk_get_size(input) as usize;
    let v = duckdb_data_chunk_get_vector(input, 0);
    let src = duckdb_vector_get_data(v) as *const i64;
    duckdb_list_vector_reserve(output, (n * 4) as idx_t);
    let child = duckdb_list_vector_get_child(output);
    let cd = duckdb_vector_get_data(child) as *mut i64;
    duckdb_vector_ensure_validity_writable(child);
    let cv = duckdb_vector_get_validity(child);
    let entries = duckdb_vector_get_data(output) as *mut duckdb_list_entry;
    for i in 0..n {
        let x = *src.add(i);
        *cd.add(i * 4) = x;
        *cd.add(i * 4 + 1) = x + 1;
        *cd.add(i * 4 + 2) = x + 2;
        bit_set_invalid(cv, i * 4 + 3);
        *entries.add(i) = duckdb_list_entry { offset: (i * 4) as u64, length: 4 };
    }
    duckdb_list_vector_set_size(output, (n * 4) as idx_t);
}

// --- registration ----------------------------------------------------------

unsafe fn register_all(con: &Connection) -> Result<(), ExtensionError> {
    let c = con.as_raw_connection();

    con.register_typed_scalar(ScalarFunctionBuilder::map1("q_inc", |x: i64| f_inc(x))?)?;
    con.register_typed_scalar(ScalarFunctionBuilder::map2("q_add", |a: i64, b: i64| f_add(a, b))?)?;
    con.register_typed_scalar(ScalarFunctionBuilder::map1_str("q_len", |s: &str| count_a(s.as_bytes()))?)?;
    con.register_typed_scalar(ScalarFunctionBuilder::map1_str("q_trim", |s: &str| s.trim_ascii().to_owned())?)?;
    for (name, callback) in [("q_list3", q_list3 as _), ("q_list4n", q_list4n as _)] {
        con.register_scalar(
            ScalarFunctionBuilder::new(name)
                .param(TypeId::BigInt)
                .returns_logical(LogicalType::list(TypeId::BigInt))
                .function(callback),
        )?;
    }

    let mut b = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_BIGINT);
    let mut v = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR);
    raw_scalar(c, "r_inc", &[b], b, r_inc);
    raw_scalar(c, "r_add", &[b, b], b, r_add);
    raw_scalar(c, "r_len", &[v], b, r_len);
    raw_scalar(c, "r_trim", &[v], v, r_trim);
    let mut list = duckdb_create_list_type(b);
    raw_scalar(c, "r_list3_grow", &[b], list, r_list3_grow);
    raw_scalar(c, "r_list4n", &[b], list, r_list4n);
    duckdb_destroy_logical_type(&mut list);
    duckdb_destroy_logical_type(&mut b);
    duckdb_destroy_logical_type(&mut v);
    Ok(())
}

quack_rs::entry_point_v2!(benchext_init_c_api, |con| register_all(con));
