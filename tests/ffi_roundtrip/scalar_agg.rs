// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Regression tests for the scalar / aggregate builder fixes: bind-data copies,
//! typed-closure signatures, panic payloads whose `Drop` panics, and duplicate
//! overload signatures.

use super::Fixture;
use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_connection, duckdb_data_chunk, duckdb_function_info, idx_t,
};
use quack_rs::aggregate::AggregateFunctionSetBuilder;
use quack_rs::query::query;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
use quack_rs::types::{LogicalType, TypeId};

// ─── ScalarBindData survives DuckDB copying the bound expression ─────────────

#[cfg(feature = "duckdb-1-5")]
mod bind_copy {
    use quack_rs::data_chunk::DataChunk;
    use quack_rs::scalar::{ScalarBindData, ScalarBindInfo, ScalarFunctionInfo};
    use quack_rs::vector::VectorWriter;

    #[derive(Clone)]
    pub struct Times(pub i64);

    pub unsafe extern "C" fn times_bind(info: libduckdb_sys::duckdb_bind_info) {
        // SAFETY: DuckDB passes a valid bind info.
        let bind = unsafe { ScalarBindInfo::new(info) };
        ScalarBindData::set(&bind, Times(10));
    }

    quack_rs::scalar_callback!(times_exec, |info, input, output| {
        // SAFETY: DuckDB passes a valid function info.
        let fninfo = unsafe { ScalarFunctionInfo::new(info) };
        // SAFETY: `times_bind` stored a `Times`, and nothing else did.
        let Some(times) = (unsafe { ScalarBindData::<Times>::get(&fninfo) }) else {
            fninfo.set_error("times10: bind data missing at execution");
            return;
        };
        let chunk = unsafe { DataChunk::from_raw(input) };
        let reader = unsafe { chunk.reader(0) };
        let mut writer = unsafe { VectorWriter::from_vector(output) };
        for row in 0..chunk.size() {
            unsafe { writer.write_i64(row, reader.read_i64(row) * times.0) };
        }
    });
}

/// Filter pushdown through a projection copies the bound function expression.
/// Without a copy callback `DuckDB` leaves the copy's bind data NULL, so the
/// function silently ran with no bind data (or, here, reports that it has
/// none).
#[cfg(feature = "duckdb-1-5")]
#[test]
fn scalar_bind_data_survives_the_optimizer_copying_the_expression() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("times10")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .bind(bind_copy::times_bind)
            .function(bind_copy::times_exec)
            .register(fx.con())
            .expect("register times10");
    }

    // x = 0, 10, .., 90; the filter keeps 10..=90.
    assert_eq!(
        fx.scalar(
            "SELECT sum(x) FROM (SELECT times10(i) AS x FROM range(10) t(i)) WHERE x > 5",
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some(450)
    );
    assert_eq!(
        fx.scalar(
            "SELECT sum(times10(i)) FROM range(3) t(i) WHERE times10(i) >= 0",
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some(30)
    );
}

// ─── A panic payload whose own Drop panics ───────────────────────────────────

struct PayloadBomb;
impl Drop for PayloadBomb {
    fn drop(&mut self) {
        panic!("panic payload destructor deliberately exploded");
    }
}

/// Before the fix the caught payload was dropped outside any guard, inside the
/// `extern "C"` trampoline, so its panicking `Drop` aborted the process
/// (`panic_cannot_unwind`) instead of failing the query.
#[test]
fn a_panic_payload_whose_drop_panics_becomes_a_sql_error() {
    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::map1("t_payload_bomb", |x: i64| -> i64 {
            if x == 13 {
                std::panic::panic_any(PayloadBomb);
            }
            x
        })
        .expect("build")
        .register(fx.con())
        .expect("register");
    }

    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT t_payload_bomb(13::BIGINT)") }
        .expect_err("the panic must surface as an error");
    assert!(err.as_str().contains("panicked"), "{err}");

    // The connection survives.
    assert_eq!(
        fx.scalar("SELECT t_payload_bomb(2::BIGINT)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(2)
    );
}

/// Regression: a result longer than `DuckDB`'s 4 GiB − 1 string limit used to
/// be stored as its length modulo 2^32 — here, one byte — with no error.
/// `DuckDB` narrows the length with a plain cast in release builds
/// (`StringVector::AddStringOrBlob`). It must fail the query instead.
///
/// Linux only: the 4 GiB buffer is `calloc`ed and never written, so it costs
/// no physical memory there; other platforms may commit it eagerly.
#[cfg(target_os = "linux")]
#[test]
fn a_string_result_over_duckdbs_limit_is_an_error_not_a_truncation() {
    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::map1("t_huge_str", |n: i64| -> String {
            let len = usize::try_from(n).expect("non-negative");
            String::from_utf8(vec![0_u8; len]).expect("NUL bytes are UTF-8")
        })
        .expect("build")
        .register(fx.con())
        .expect("register");
    }

    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT strlen(t_huge_str(4294967297))") }
        .expect_err("an over-long result must fail the query");
    assert!(
        err.as_str().contains("exceeds DuckDB's maximum string length"),
        "{err}"
    );

    // A value within the limit still round-trips.
    assert_eq!(
        fx.scalar("SELECT strlen(t_huge_str(20))", |r, i| unsafe { r.read_i64(i) }),
        Some(20)
    );
}

// ─── Typed closures keep the signature they were compiled for ───────────────

/// A `Registrar` that edits the signature of every scalar it is handed — the
/// one route by which a typed closure's builder can still be altered.
struct TamperingRegistrar(duckdb_connection);

impl quack_rs::connection::Registrar for TamperingRegistrar {
    unsafe fn register_scalar(
        &self,
        builder: ScalarFunctionBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        // An i64 closure redeclared as returning a 4-byte INTEGER.
        unsafe { builder.returns(TypeId::Integer).register(self.0) }
    }
    unsafe fn register_scalar_set(
        &self,
        builder: ScalarFunctionSetBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { builder.register(self.0) }
    }
    unsafe fn register_aggregate(
        &self,
        builder: quack_rs::aggregate::AggregateFunctionBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { builder.register(self.0) }
    }
    unsafe fn register_aggregate_set(
        &self,
        builder: AggregateFunctionSetBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { builder.register(self.0) }
    }
    unsafe fn register_table(
        &self,
        builder: quack_rs::table::TableFunctionBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { builder.register(self.0) }
    }
    unsafe fn register_sql_macro(
        &self,
        sql_macro: quack_rs::sql_macro::SqlMacro,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { sql_macro.register(self.0) }
    }
    unsafe fn register_cast(
        &self,
        builder: quack_rs::cast::CastFunctionBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { builder.register(self.0) }
    }
    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_copy_function(
        &self,
        builder: quack_rs::copy_function::CopyFunctionBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { builder.register(self.0) }
    }
    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_config_option(
        &self,
        builder: quack_rs::config_option::ConfigOptionBuilder,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        unsafe { builder.register(self.0) }
    }
}

/// Before the fix `map1(..)?.returns(TypeId::Integer)` compiled, and the
/// trampoline wrote 8-byte `i64`s into `DuckDB`'s 4-byte INTEGER result vector:
/// truncated values, and a heap overflow past row 1024. The builder no longer
/// offers `returns` (see the `compile_fail` doctests on
/// `TypedScalarFunctionBuilder`); this covers the last route, a `Registrar`
/// that edits the builder it is handed, which the trampoline now refuses with
/// a SQL error before touching a row.
#[test]
fn an_edited_typed_signature_is_a_sql_error_not_memory_corruption() {
    use quack_rs::connection::Registrar;

    let fx = Fixture::open();
    let reg = TamperingRegistrar(fx.con());
    // SAFETY: `con` is open.
    unsafe {
        reg.register_typed_scalar(
            ScalarFunctionBuilder::map1("t_widen", |x: i64| x + (1_i64 << 32)).expect("build"),
        )
        .expect("register");
    }

    // More than one vector's worth of rows: the old overflow path.
    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT sum(t_widen(i)) FROM range(5000) t(i)") }
        .expect_err("a mismatched signature must be refused");
    assert!(
        err.as_str().contains("signature must not be changed"),
        "{err}"
    );
}

/// `volatile` is stable C API (v1.2.0) and no longer needs `duckdb-1-5`; on a
/// typed closure it changes nothing about the signature.
#[test]
fn a_typed_closure_can_be_volatile() {
    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::map1("t_volatile_inc", |x: i64| x + 1)
            .expect("build")
            .volatile()
            .register(fx.con())
            .expect("register");
    }
    assert_eq!(
        fx.scalar(
            "SELECT sum(t_volatile_inc(i)) FROM range(3000) t(i)",
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some((1..=3000_i128).sum::<i128>())
    );
}

// ─── Duplicate overload signatures ───────────────────────────────────────────

quack_rs::scalar_callback!(first_arg_as_i64, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { quack_rs::vector::VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row)) };
    }
});

fn bigint_overload() -> ScalarOverloadBuilder {
    ScalarOverloadBuilder::new()
        .param(TypeId::BigInt)
        .returns(TypeId::BigInt)
        .function(first_arg_as_i64)
}

/// Before the fix this set registered successfully and every call to it then
/// failed with "Could not choose a best candidate function".
#[test]
fn a_scalar_set_with_duplicate_overloads_is_rejected_at_registration() {
    let fx = Fixture::open();
    let set = ScalarFunctionSetBuilder::try_new("dup_scalar")
        .expect("name")
        .overload(bigint_overload())
        .overload(
            ScalarOverloadBuilder::new()
                .param(TypeId::Double)
                .returns(TypeId::BigInt)
                .function(first_arg_as_i64),
        )
        .overload(bigint_overload());
    // SAFETY: `con` is open.
    let err = unsafe { set.register(fx.con()) }.expect_err("duplicate overloads");
    let msg = err.as_str();
    assert!(msg.contains("overload 0 and overload 2"), "{msg}");
    assert!(msg.contains("(BIGINT)"), "{msg}");

    // Nothing was registered.
    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT dup_scalar(1::BIGINT)") }
        .expect_err("the rejected set must not exist");
    assert!(err.as_str().contains("dup_scalar"), "{err}");
}

/// A `TypeId` parameter and the equivalent `LogicalType` one are the same
/// argument type to `DuckDB`.
#[test]
fn a_type_id_and_an_equal_logical_type_are_duplicates() {
    let fx = Fixture::open();
    let set = ScalarFunctionSetBuilder::try_new("dup_mixed")
        .expect("name")
        .overload(bigint_overload())
        .overload(
            ScalarOverloadBuilder::new()
                .param_logical(LogicalType::new(TypeId::BigInt))
                .returns(TypeId::BigInt)
                .function(first_arg_as_i64),
        );
    // SAFETY: `con` is open.
    let err = unsafe { set.register(fx.con()) }.expect_err("duplicate overloads");
    assert!(err.as_str().contains("overload 0 and overload 1"), "{err}");
}

quack_rs::scalar_callback!(writes_one, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let mut writer = unsafe { quack_rs::vector::VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, 1) };
    }
});

quack_rs::scalar_callback!(writes_two, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let mut writer = unsafe { quack_rs::vector::VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, 2) };
    }
});

fn logical_overload(ty: LogicalType, f: quack_rs::scalar::ScalarFn) -> ScalarOverloadBuilder {
    ScalarOverloadBuilder::new()
        .param_logical(ty)
        .returns(TypeId::BigInt)
        .function(f)
}

/// The comparison is structural, not by type id: parameterised types that
/// differ only in a parameter are not duplicates, and a set of them registers.
#[test]
fn overloads_differing_only_in_type_parameters_are_not_duplicates() {
    let fx = Fixture::open();

    // Distinct nested types: registers, and DuckDB resolves each call.
    let lists = ScalarFunctionSetBuilder::try_new("list_pick")
        .expect("name")
        .overload(logical_overload(
            LogicalType::list(TypeId::BigInt),
            writes_one,
        ))
        .overload(logical_overload(
            LogicalType::list(TypeId::Varchar),
            writes_two,
        ));
    // SAFETY: `con` is open.
    unsafe { lists.register(fx.con()) }.expect("distinct overloads register");
    assert_eq!(
        fx.scalar("SELECT list_pick([1, 2]::BIGINT[])", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(1)
    );
    assert_eq!(
        fx.scalar("SELECT list_pick(['a']::VARCHAR[])", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(2)
    );

    // Same type id, different parameters: not an exact duplicate, so quack-rs
    // does not reject it. (DuckDB 1.5.5's binder cannot choose between two
    // DECIMAL overloads that differ only in width/scale, so such a set is of
    // little use — but that is DuckDB's overload resolution, not a duplicate
    // declaration.)
    let decimals = ScalarFunctionSetBuilder::try_new("dec_scale")
        .expect("name")
        .overload(logical_overload(LogicalType::decimal(18, 2), writes_one))
        .overload(logical_overload(LogicalType::decimal(18, 3), writes_two));
    // SAFETY: `con` is open.
    unsafe { decimals.register(fx.con()) }.expect("DECIMAL(18,2) and DECIMAL(18,3) differ");

    let structs = ScalarFunctionSetBuilder::try_new("struct_pick")
        .expect("name")
        .overload(logical_overload(
            LogicalType::struct_type(&[("a", TypeId::BigInt)]),
            writes_one,
        ))
        .overload(logical_overload(
            LogicalType::struct_type(&[("b", TypeId::BigInt)]),
            writes_two,
        ));
    // SAFETY: `con` is open.
    unsafe { structs.register(fx.con()) }.expect("field names are part of the type");

    // ...while a structurally identical nested type is a duplicate.
    let dup = ScalarFunctionSetBuilder::try_new("dup_list")
        .expect("name")
        .overload(logical_overload(
            LogicalType::list(TypeId::BigInt),
            writes_one,
        ))
        .overload(logical_overload(
            LogicalType::list(TypeId::BigInt),
            writes_two,
        ));
    // SAFETY: `con` is open.
    let err = unsafe { dup.register(fx.con()) }.expect_err("identical LIST(BIGINT)");
    assert!(err.as_str().contains("(LIST(BIGINT))"), "{err}");
}

mod dummy_aggregate {
    use super::{duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, idx_t};
    use libduckdb_sys::duckdb_vector;

    pub const unsafe extern "C" fn size(_: duckdb_function_info) -> idx_t {
        8
    }
    pub const unsafe extern "C" fn init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
    pub const unsafe extern "C" fn update(
        _: duckdb_function_info,
        _: duckdb_data_chunk,
        _: *mut duckdb_aggregate_state,
    ) {
    }
    pub const unsafe extern "C" fn combine(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: *mut duckdb_aggregate_state,
        _: idx_t,
    ) {
    }
    pub const unsafe extern "C" fn finalize(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: duckdb_vector,
        _: idx_t,
        _: idx_t,
    ) {
    }
}

/// Before the fix this set registered successfully and every call to it then
/// failed with "Could not choose a best candidate function".
#[test]
fn an_aggregate_set_with_duplicate_overloads_is_rejected_at_registration() {
    let fx = Fixture::open();
    let arity = |n: usize, b: quack_rs::aggregate::AggregateOverloadBuilder| {
        (0..n)
            .fold(b, |b, _| b.param(TypeId::Boolean))
            .state_size(dummy_aggregate::size)
            .init(dummy_aggregate::init)
            .update(dummy_aggregate::update)
            .combine(dummy_aggregate::combine)
            .finalize(dummy_aggregate::finalize)
    };
    // 2..=3 and then 3..=4: arity 3 is declared twice.
    let set = AggregateFunctionSetBuilder::try_new("dup_agg")
        .expect("name")
        .returns(TypeId::BigInt)
        .overloads(2..=3, arity)
        .overloads(3..=4, arity);
    // SAFETY: `con` is open.
    let err = unsafe { set.register(fx.con()) }.expect_err("duplicate overloads");
    let msg = err.as_str();
    assert!(msg.contains("overload 1 and overload 2"), "{msg}");
    assert!(msg.contains("(BOOLEAN, BOOLEAN, BOOLEAN)"), "{msg}");
}
