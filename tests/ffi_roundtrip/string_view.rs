// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `DuckStringView`, `read_duck_string` and `read_duck_blob` against the
//! `duckdb_string_t` records a live `DuckDB` wrote: the inline form (12 bytes
//! or fewer), the pointer form (13 or more), the empty string, multi-byte
//! UTF-8, and a `BLOB` that is not UTF-8.
//!
//! Until now these were checked only against 16-byte arrays the tests built
//! by hand, which show the decoder agrees with the layout the author
//! remembered, not with the one `DuckDB` writes.

use libduckdb_sys::duckdb_vector_get_data;
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFn, ScalarFunctionBuilder};
use quack_rs::types::TypeId;
use quack_rs::vector::string::{
    read_duck_blob, read_duck_string, DuckStringView, DUCK_STRING_INLINE_MAX_LEN, DUCK_STRING_SIZE,
};
use quack_rs::vector::VectorWriter;

use super::vector_support::{edge_rows, text_rows};
use super::Fixture;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// VARCHAR -> "len:inline:text:hex" where every field comes from the view or
// the free functions, after checking that they all agree with each other.
quack_rs::scalar_callback!(varchar_probe, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let data = unsafe { duckdb_vector_get_data(chunk.vector(0)) }
        .cast::<u8>()
        .cast_const();
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        // SAFETY: row `row` of a VARCHAR vector is a 16-byte `duckdb_string_t`.
        let raw: &[u8; DUCK_STRING_SIZE] = unsafe {
            &*data
                .add(row * DUCK_STRING_SIZE)
                .cast::<[u8; DUCK_STRING_SIZE]>()
        };
        let view = unsafe { DuckStringView::from_raw(raw) };
        let text = view.as_str().expect("the engine wrote valid UTF-8");
        assert_eq!(view.len(), text.len());
        assert_eq!(view.is_empty(), text.is_empty());
        assert_eq!(unsafe { read_duck_string(data, row) }, text);
        assert_eq!(unsafe { reader.read_str(row) }, text);
        assert_eq!(unsafe { read_duck_blob(data, row) }, text.as_bytes());

        // The safe constructor decodes the inline form and refuses the other.
        // 12 bytes is what `duckdb_string_t` holds inline; not the crate's own constant.
        let inline = text.len() <= 12;
        let safe = DuckStringView::inline_from_bytes(raw);
        assert_eq!(safe.is_some(), inline, "row {row}: `{text}`");
        if let Some(safe) = safe {
            assert_eq!(safe.as_str(), Some(text));
            assert_eq!(safe.len(), text.len());
        }
        // The deprecated one cannot follow a pointer, and says so with `None`.
        #[allow(deprecated)]
        let legacy = DuckStringView::from_bytes(raw);
        assert_eq!(legacy.len(), text.len());
        assert_eq!(legacy.as_str(), inline.then_some(text));

        let summary = format!("{}:{inline}:{text}:{}", view.len(), hex(text.as_bytes()));
        unsafe { writer.write_varchar(row, &summary) };
    }
});

// BLOB -> "len:is-utf8:hex": the bytes need not be UTF-8.
quack_rs::scalar_callback!(blob_probe, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let data = unsafe { duckdb_vector_get_data(chunk.vector(0)) }
        .cast::<u8>()
        .cast_const();
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let raw: &[u8; DUCK_STRING_SIZE] = unsafe {
            &*data
                .add(row * DUCK_STRING_SIZE)
                .cast::<[u8; DUCK_STRING_SIZE]>()
        };
        let view = unsafe { DuckStringView::from_raw(raw) };
        let bytes = unsafe { read_duck_blob(data, row) };
        assert_eq!(view.len(), bytes.len());
        assert_eq!(bytes, unsafe { reader.read_blob(row) });
        let utf8 = view.as_str();
        assert_eq!(
            utf8.is_some(),
            std::str::from_utf8(bytes).is_ok(),
            "row {row}"
        );
        let summary = format!("{}:{}:{}", bytes.len(), utf8.is_some(), hex(bytes));
        unsafe { writer.write_varchar(row, &summary) };
    }
});

fn register(fx: &Fixture, name: &str, param: TypeId, callback: ScalarFn) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("valid function name")
            .param(param)
            .returns(TypeId::Varchar)
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

/// Empty, one byte, exactly 12 bytes (the inline maximum), 13 (the first
/// pointer form), multi-byte UTF-8 well past 12 bytes, and a number; NULL when
/// `x % 7 == 3`.
const TEXT: &str = "CASE WHEN x % 7 = 3 THEN NULL ELSE CASE x % 6 \
    WHEN 0 THEN '' WHEN 1 THEN 'a' WHEN 2 THEN repeat('b', 12) WHEN 3 THEN repeat('c', 13) \
    WHEN 4 THEN 'héllo wörld — ünïcode €' ELSE 'row ' || x END END";

#[test]
fn string_views_decode_the_records_duckdb_wrote() {
    let fx = Fixture::open();
    assert_eq!(DUCK_STRING_INLINE_MAX_LEN, 12);
    assert_eq!(DUCK_STRING_SIZE, 16);
    register(&fx, "varchar_probe", TypeId::Varchar, varchar_probe);
    for n in edge_rows() {
        let rows = text_rows(
            &fx,
            &format!(
                "SELECT varchar_probe(s), \
                        strlen(s) || ':' || (strlen(s) <= 12) || ':' || s || ':' || lower(hex(s)) \
                 FROM (SELECT {TEXT} AS s, x FROM range({n}) t(x)) ORDER BY x"
            ),
        );
        assert_eq!(rows.len(), n);
        for (x, row) in rows.iter().enumerate() {
            assert_eq!(row[0], row[1], "n={n} x={x}");
            assert_eq!(row[0].is_none(), x % 7 == 3, "n={n} x={x}");
        }
    }
}

/// Bytes that are not UTF-8 although they start with an ASCII byte, and one
/// valid row: inline, a truncated sequence as the last inline byte, and the
/// pointer form. `DuckDB` validates every VARCHAR SQL builds, but stores
/// these as given through `duckdb_unsafe_vector_assign_string_element_len`
/// (as the CLI's `getenv` stores environment bytes).
#[cfg(feature = "duckdb-1-5")]
const NOT_UTF8: [&[u8]; 4] = [
    b"x\xff\xfey",
    b"abcdefghijk\xc3",
    b"long-prefix-0123456789\xc3(",
    b"abcd",
];

// BIGINT i -> NOT_UTF8[i % 4], assigned unchecked.
#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_callback!(not_utf8_source, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        let bytes = NOT_UTF8[(unsafe { reader.read_i64(row) } % 4) as usize];
        // SAFETY: `output` is this call's VARCHAR result vector and `row` is
        // within the chunk; the bytes are copied (or inlined) by DuckDB.
        unsafe {
            libduckdb_sys::duckdb_unsafe_vector_assign_string_element_len(
                output,
                row as u64,
                bytes.as_ptr().cast(),
                bytes.len() as u64,
            );
        }
    }
});

// VARCHAR -> "as_str is Some:bytes are UTF-8:read_str length".
#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_callback!(utf8_probe, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let data = unsafe { duckdb_vector_get_data(chunk.vector(0)) }
        .cast::<u8>()
        .cast_const();
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        // SAFETY: row `row` of a VARCHAR vector is a 16-byte `duckdb_string_t`.
        let raw: &[u8; DUCK_STRING_SIZE] = unsafe {
            &*data
                .add(row * DUCK_STRING_SIZE)
                .cast::<[u8; DUCK_STRING_SIZE]>()
        };
        let view = unsafe { DuckStringView::from_raw(raw) };
        let bytes = unsafe { read_duck_blob(data, row) };
        let summary = format!(
            "{}:{}:{}",
            view.as_str().is_some(),
            std::str::from_utf8(bytes).is_ok(),
            unsafe { reader.read_str(row) }.len()
        );
        unsafe { writer.write_varchar(row, &summary) };
    }
});

/// `as_str` returns `None` for bytes that are not UTF-8, also when they
/// start with ASCII ones: its fast path must look at every byte before it
/// skips validation (a mutant that checked only the first byte handed out a
/// `&str` of invalid UTF-8, which is undefined behaviour, and passed every
/// other live test here). `read_str` reads such a row as `""`.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn string_views_refuse_invalid_utf8_behind_an_ascii_byte() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; each callback matches its declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("not_utf8_source")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::Varchar)
            .function(not_utf8_source)
            .register(fx.con())
            .expect("register not_utf8_source");
    }
    register(&fx, "utf8_probe", TypeId::Varchar, utf8_probe);
    let rows = text_rows(
        &fx,
        "SELECT utf8_probe(not_utf8_source(i)) FROM range(8) t(i) ORDER BY i",
    );
    let got: Vec<Option<&str>> = rows.iter().map(|r| r[0].as_deref()).collect();
    let bad = Some("false:false:0");
    let good = Some("true:true:4");
    assert_eq!(got, [bad, bad, bad, good, bad, bad, bad, good]);
}

#[test]
fn blob_views_read_bytes_that_are_not_utf8() {
    let fx = Fixture::open();
    register(&fx, "blob_probe", TypeId::Blob, blob_probe);
    // Inline and pointer form; valid and invalid UTF-8; embedded NULs.
    let long_valid: Vec<u8> = "hello, wörld and a longer tail".bytes().collect();
    let mut long_invalid: Vec<u8> = (0..40).map(|b| b'a' + (b % 26)).collect();
    long_invalid[20] = 0xFF;
    let blobs: Vec<Vec<u8>> = vec![
        vec![],
        vec![0x00],
        vec![0x80, 0xFE],
        vec![0xC3, 0x28],
        b"abcdefghijkl".to_vec(),
        // 13 bytes: the first that does not fit inline, and not UTF-8.
        [b"abcdefghijkl".as_slice(), &[0xFF]].concat(),
        long_valid,
        long_invalid,
        (0..64).collect(),
        vec![0x00; 100],
    ];
    let values = blobs
        .iter()
        .map(|b| format!("('{}')", hex(b)))
        .chain(std::iter::once("(NULL)".to_owned()))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = text_rows(
        &fx,
        &format!("SELECT blob_probe(from_hex(h)) FROM (VALUES {values}) t(h)"),
    );
    assert_eq!(rows.len(), blobs.len() + 1);
    for (i, bytes) in blobs.iter().enumerate() {
        let want = format!(
            "{}:{}:{}",
            bytes.len(),
            std::str::from_utf8(bytes).is_ok(),
            hex(bytes)
        );
        assert_eq!(rows[i][0].as_deref(), Some(want.as_str()), "blob {i}");
    }
    assert_eq!(rows[blobs.len()][0], None, "the NULL blob stays NULL");
}
