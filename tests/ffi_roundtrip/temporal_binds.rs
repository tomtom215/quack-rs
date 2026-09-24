// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Temporal and oversized values through `PreparedStatement` binds and the
//! `Appender`: refused before `DuckDB` stores what it cannot handle.

use quack_rs::appender::Appender;

use super::Fixture;

/// Binds `bind` into `SELECT CAST($1 AS VARCHAR)` and renders the result.
fn rendered(
    fx: &Fixture,
    bind: impl Fn(&quack_rs::query::PreparedStatement) -> Result<(), quack_rs::error::ExtensionError>,
) -> Result<String, String> {
    // SAFETY: `con` is open.
    let statement = unsafe { quack_rs::query::prepare(fx.con(), "SELECT CAST($1 AS VARCHAR)") }
        .map_err(|e| e.to_string())?;
    bind(&statement).map_err(|e| e.to_string())?;
    let mut result = statement.execute().map_err(|e| e.to_string())?;
    let chunk = result
        .next_chunk()
        .map_err(|e| e.to_string())?
        .ok_or("no row")?;
    // SAFETY: one VARCHAR row.
    Ok(unsafe { chunk.reader(0).read_str(0).to_owned() })
}

/// The fourth audit's values-V1 and V10. `bind_time`, `bind_timestamp` and
/// `bind_timestamp_tz` passed any `i64` to `DuckDB`, which stores it
/// unchecked: rendering `TIME i64::MIN` crashed the process (SIGSEGV in
/// `StringCast::Operation<dtime_t>`), and `TIME i64::MAX` invalidated the
/// database. They are now refused first; in-range values, including the
/// infinities, still bind.
#[test]
fn out_of_range_temporal_binds_are_refused() {
    let fx = Fixture::open();
    for (label, result) in [
        ("time i64::MIN", rendered(&fx, |s| s.bind_time(1, i64::MIN))),
        ("time i64::MAX", rendered(&fx, |s| s.bind_time(1, i64::MAX))),
        ("time -1", rendered(&fx, |s| s.bind_time(1, -1))),
        (
            "timestamp i64::MIN",
            rendered(&fx, |s| s.bind_timestamp(1, i64::MIN)),
        ),
        (
            "timestamptz i64::MIN",
            rendered(&fx, |s| s.bind_timestamp_tz(1, i64::MIN)),
        ),
    ] {
        let err = result.expect_err(label);
        assert!(err.contains("outside DuckDB's range"), "{label}: {err}");
    }
    assert_eq!(
        rendered(&fx, |s| s.bind_time(1, 3_600_000_000)).as_deref(),
        Ok("01:00:00")
    );
    assert_eq!(
        rendered(&fx, |s| s.bind_time(1, 86_400_000_000)).as_deref(),
        Ok("24:00:00")
    );
    assert_eq!(
        rendered(&fx, |s| s.bind_timestamp(1, -i64::MAX)).as_deref(),
        Ok("-infinity")
    );
    assert_eq!(
        rendered(&fx, |s| s.bind_timestamp(1, 0)).as_deref(),
        Ok("1970-01-01 00:00:00")
    );
}

/// `DATE` stays unchecked, as documented: every day count renders, but one
/// outside `DuckDB`'s SQL range comes back as text it cannot parse.
#[test]
fn an_out_of_range_date_binds_and_renders_unparseably() {
    let fx = Fixture::open();
    let text = rendered(&fx, |s| s.bind_date(1, i32::MIN)).expect("DATE binds");
    assert_eq!(text, "5877642-06-23 (BC)");
    let reparsed = fx.scalar(
        "SELECT TRY_CAST('5877642-06-23 (BC)' AS DATE) IS NULL",
        |r, i| unsafe { r.read_bool(i) },
    );
    assert_eq!(reparsed, Some(true));
}

/// The appender side of V1/V10: `append_time` / `append_timestamp` refuse an
/// out-of-range payload before `DuckDB` sees it. As the first value of a row
/// nothing is half-written, so the appender carries on.
#[test]
fn out_of_range_temporal_appends_are_refused() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE tb_t (t TIME, ts TIMESTAMP)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"tb_t") }.expect("create");
    let err = appender
        .row(|row| {
            row.append_time(i64::MIN)?;
            row.append_timestamp(0)
        })
        .expect_err("TIME i64::MIN");
    assert!(err.to_string().contains("outside DuckDB's range"), "{err}");
    appender
        .row(|row| {
            row.append_time(3_600_000_000)?;
            row.append_timestamp(0)
        })
        .expect("an in-range row still appends");
    appender.close().expect("close");
    drop(appender);
    assert_eq!(
        fx.scalar(
            "SELECT t::VARCHAR || ' ' || ts::VARCHAR FROM tb_t",
            |r, i| unsafe { r.read_str(i).to_owned() }
        )
        .as_deref(),
        Some("01:00:00 1970-01-01 00:00:00")
    );
}

/// The fourth audit's values-V3. `append_bytes` of more than `u32::MAX`
/// bytes stored the length modulo 2^32 — 4 GiB + 3 bytes came back as 3 —
/// while `append_str` already refused. Both appends and both binds now
/// refuse. The buffer is zero-filled by `calloc` and never touched, so the
/// test costs address space, not memory.
#[test]
fn values_over_four_gib_are_refused_by_append_and_bind() {
    let huge = vec![0_u8; (1_usize << 32) + 3];
    let fx = Fixture::open();
    fx.query("CREATE TABLE tb_big (b BLOB)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"tb_big") }.expect("create");
    let err = appender
        .row(|row| row.append_bytes(&huge))
        .expect_err("append_bytes");
    assert!(err.to_string().contains("appender limit"), "{err}");
    appender.close().expect("close");
    drop(appender);
    let err = rendered(&fx, |s| s.bind_blob(1, &huge)).expect_err("bind_blob");
    assert!(err.contains("maximum string length"), "{err}");
}
