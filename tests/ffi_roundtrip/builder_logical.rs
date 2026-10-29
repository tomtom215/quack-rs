// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The `*_logical` builder methods: a parameter, varargs or return type that
//! a bare `TypeId` cannot express.
//!
//! Each is held to what `duckdb_functions()` says the registered function
//! looks like, and to a call only the declared types accept. The position
//! arithmetic of `param_logical` (which interleaves with `param`) is the part
//! a single call cannot show, so the aggregate and table tests mix the two in
//! several orders.

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionSetBuilder, AggregateOverloadBuilder,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::{LogicalType, NullHandling, TypeId};
use quack_rs::vector::VectorWriter;

use super::Fixture;

/// Every registered signature of `name`: `parameter_types` and `varargs`, as
/// `DuckDB` spells them, sorted.
fn signatures(fx: &Fixture, name: &str) -> Vec<(String, Option<String>)> {
    let mut result = fx.query(&format!(
        "SELECT '[' || coalesce(array_to_string(parameter_types, ', '), '') || ']', varargs \
         FROM duckdb_functions() WHERE function_name = '{name}' ORDER BY 1, 2"
    ));
    let mut out = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("fetch") {
        // SAFETY: two VARCHAR columns.
        let (types, varargs) = unsafe { (chunk.reader(0), chunk.reader(1)) };
        for row in 0..chunk.size() {
            // SAFETY: `row` is in range.
            unsafe {
                out.push((
                    types.read_str(row).to_owned(),
                    varargs
                        .is_valid(row)
                        .then(|| varargs.read_str(row).to_owned()),
                ));
            }
        }
    }
    out
}

fn error_of(fx: &Fixture, sql: &str) -> String {
    // SAFETY: the fixture's connection is open.
    let err = unsafe { quack_rs::query::query(fx.con(), sql) }
        .map(drop)
        .expect_err(sql);
    format!("{err}")
}

fn text(fx: &Fixture, sql: &str) -> String {
    fx.scalar(sql, |r, i| unsafe { r.read_str(i).to_owned() })
        .unwrap_or_else(|| panic!("{sql} returned NULL"))
}

fn int(fx: &Fixture, sql: &str) -> i64 {
    fx.scalar(sql, |r, i| unsafe { r.read_i64(i) })
        .unwrap_or_else(|| panic!("{sql} returned NULL"))
}

/// One parameter, declared either by id or as a logical type.
#[derive(Clone, Copy)]
enum Slot {
    Id(TypeId),
    /// How `DuckDB` spells it, and how to build it.
    Logical(&'static str, fn() -> LogicalType),
}

impl Slot {
    fn sql(self) -> String {
        match self {
            Self::Id(id) => id.sql_name().to_owned(),
            Self::Logical(sql, _) => sql.to_owned(),
        }
    }
}

const DECIMAL: Slot = Slot::Logical("DECIMAL(18,2)", || LogicalType::decimal(18, 2));
const INT_LIST: Slot = Slot::Logical("INTEGER[]", || LogicalType::list(TypeId::Integer));
const MAP: Slot = Slot::Logical("MAP(VARCHAR, DOUBLE)", || {
    LogicalType::map(TypeId::Varchar, TypeId::Double)
});
const FLOAT_ARRAY: Slot = Slot::Logical("FLOAT[2]", || LogicalType::array(TypeId::Float, 2));
const POINT: Slot = Slot::Logical("STRUCT(x BIGINT, y BIGINT)", || {
    LogicalType::struct_type(&[("x", TypeId::BigInt), ("y", TypeId::BigInt)])
});
const SMALL_DECIMAL: Slot = Slot::Logical("DECIMAL(9,2)", || LogicalType::decimal(9, 2));
const VARCHAR: Slot = Slot::Id(TypeId::Varchar);
const BIGINT: Slot = Slot::Id(TypeId::BigInt);

/// Parameter lists that put `param` and `param_logical` in every relative
/// order that matters: logical first, last, alone, adjacent, and between.
fn arrangements() -> Vec<Vec<Slot>> {
    vec![
        vec![DECIMAL],
        vec![VARCHAR, DECIMAL],
        vec![DECIMAL, VARCHAR],
        vec![
            VARCHAR,
            INT_LIST,
            BIGINT,
            MAP,
            FLOAT_ARRAY,
            Slot::Id(TypeId::Blob),
        ],
        vec![
            INT_LIST,
            POINT,
            Slot::Id(TypeId::Date),
            Slot::Id(TypeId::Integer),
            SMALL_DECIMAL,
        ],
    ]
}

fn spelled(slots: &[Slot]) -> String {
    format!(
        "[{}]",
        slots.iter().map(|s| s.sql()).collect::<Vec<_>>().join(", ")
    )
}

/// `NULL::T, NULL::U, ...` for a call that must bind to exactly `slots`.
fn null_args(slots: &[Slot]) -> String {
    slots
        .iter()
        .map(|s| format!("NULL::{}", s.sql()))
        .collect::<Vec<_>>()
        .join(", ")
}

// ─── Aggregates ──────────────────────────────────────────────────────────────

// A plain-old-data `i64` state that counts rows, and a second that sums a
// weighted function of the first three columns.

const unsafe extern "C" fn state_size(_info: duckdb_function_info) -> idx_t {
    8
}

const unsafe extern "C" fn state_init(_info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { state.cast::<i64>().write(0) };
}

unsafe extern "C" fn count_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    for row in 0..chunk.size() {
        unsafe { *(*states.add(row)).cast::<i64>() += 1 };
    }
}

/// `len(a) * 1_000_000 + b * 1000 + c` over columns (VARCHAR, DECIMAL(18,2),
/// BIGINT): each column lands in its own digits, so reading one as another
/// type, or in another position, gives a different total.
unsafe extern "C" fn weighted_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let (a, b, c) = unsafe { (chunk.reader(0), chunk.reader(1), chunk.reader(2)) };
    for row in 0..chunk.size() {
        let weight = unsafe {
            a.read_str(row).len() as i64 * 1_000_000 + b.read_i64(row) * 1000 + c.read_i64(row)
        };
        unsafe { *(*states.add(row)).cast::<i64>() += weight };
    }
}

unsafe extern "C" fn sum_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        unsafe { *(*target.add(i)).cast::<i64>() += *(*source.add(i)).cast::<i64>() };
    }
}

unsafe extern "C" fn sum_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        unsafe { writer.write_i64(offset as usize + i, *(*source.add(i)).cast::<i64>()) };
    }
}

fn counting_overload(slots: &[Slot]) -> AggregateOverloadBuilder {
    let mut builder = AggregateOverloadBuilder::new();
    for slot in slots {
        builder = match *slot {
            Slot::Id(id) => builder.param(id),
            Slot::Logical(_, make) => builder.param_logical(make()),
        };
    }
    builder
        .state_size(state_size)
        .init(state_init)
        .update(count_update)
        .combine(sum_combine)
        .finalize(sum_finalize)
}

#[test]
fn aggregate_param_logical_keeps_declaration_order_with_param() {
    let fx = Fixture::open();
    for (i, slots) in arrangements().iter().enumerate() {
        let name = format!("agg_single_{i}");
        let mut builder = AggregateFunctionBuilder::try_new(&name)
            .expect("name")
            .returns(TypeId::BigInt)
            .state_size(state_size)
            .init(state_init)
            .update(count_update)
            .combine(sum_combine)
            .finalize(sum_finalize);
        for slot in slots {
            builder = match *slot {
                Slot::Id(id) => builder.param(id),
                Slot::Logical(_, make) => builder.param_logical(make()),
            };
        }
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe { builder.register(fx.con()) }
            .unwrap_or_else(|e| panic!("register {name} {}: {e}", spelled(slots)));
        assert_eq!(
            signatures(&fx, &name),
            [(spelled(slots), None)],
            "{name}: registered parameter types"
        );
        // A call with exactly those types binds and counts its rows; the
        // function sees three rows of NULLs.
        let call = format!("SELECT {name}({}) FROM range(3)", null_args(slots));
        assert_eq!(int(&fx, &call), 3, "{call}");
    }
}

#[test]
fn aggregate_overload_param_logical_keeps_declaration_order_with_param() {
    let fx = Fixture::open();
    let all = arrangements();
    let mut set = AggregateFunctionSetBuilder::try_new("agg_overloads")
        .expect("name")
        .returns(TypeId::BigInt);
    for slots in &all {
        set = set.overload(counting_overload(slots));
    }
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe { set.register(fx.con()) }.expect("register agg_overloads");

    let mut expected: Vec<(String, Option<String>)> =
        all.iter().map(|s| (spelled(s), None)).collect();
    expected.sort();
    assert_eq!(signatures(&fx, "agg_overloads"), expected);
    // Each overload is reachable by exactly its own types.
    for slots in &all {
        let call = format!("SELECT agg_overloads({}) FROM range(3)", null_args(slots));
        assert_eq!(int(&fx, &call), 3, "{call}");
    }
}

/// Registers the weighted aggregate over (VARCHAR, DECIMAL(18,2), BIGINT,
/// INTEGER[]) as a single function or as the only overload of a set. The
/// update reads the first three columns; the list is only there to be
/// declared.
fn register_weighted(fx: &Fixture, name: &str, as_overload: bool) {
    if as_overload {
        let overload = AggregateOverloadBuilder::new()
            .param(TypeId::Varchar)
            .param_logical(LogicalType::decimal(18, 2))
            .param(TypeId::BigInt)
            .param_logical(LogicalType::list(TypeId::Integer))
            .state_size(state_size)
            .init(state_init)
            .update(weighted_update)
            .combine(sum_combine)
            .finalize(sum_finalize);
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe {
            AggregateFunctionSetBuilder::try_new(name)
                .expect("name")
                .returns(TypeId::BigInt)
                .overload(overload)
                .register(fx.con())
        }
        .expect("register the set");
    } else {
        // SAFETY: `con` is open; the callbacks match their signatures.
        unsafe {
            AggregateFunctionBuilder::try_new(name)
                .expect("name")
                .param(TypeId::Varchar)
                .param_logical(LogicalType::decimal(18, 2))
                .param(TypeId::BigInt)
                .param_logical(LogicalType::list(TypeId::Integer))
                .returns(TypeId::BigInt)
                .state_size(state_size)
                .init(state_init)
                .update(weighted_update)
                .combine(sum_combine)
                .finalize(sum_finalize)
                .register(fx.con())
        }
        .expect("register the single");
    }
}

#[test]
fn aggregate_logical_parameters_arrive_in_the_columns_the_update_expects() {
    let fx = Fixture::open();
    register_weighted(&fx, "weighted", false);
    register_weighted(&fx, "weighted_set", true);
    for name in ["weighted", "weighted_set"] {
        // Each of the two rows adds len('ab') * 1_000_000 + 125 * 1000 + 3,
        // where 125 is the DECIMAL(18,2) 1.25 as DuckDB stores it.
        let call = format!("SELECT {name}('ab', 1.25::DECIMAL(18,2), 3, [1, 2]) FROM range(2)");
        assert_eq!(
            int(&fx, &call),
            2 * (2 * 1_000_000 + 125 * 1000 + 3),
            "{call}"
        );
        // The declared types are enforced: a scalar where the LIST goes, and
        // a LIST where the DECIMAL goes, bind to nothing.
        // The engine lists the signature that was registered; the DECIMAL(18,2)
        // can only come from it, as neither call passes one.
        for wrong in ["'ab', 1.25, 3, 7", "'ab', [1], 3, [1, 2]"] {
            let err = error_of(&fx, &format!("SELECT {name}({wrong})"));
            assert!(
                err.contains(name) && err.contains("DECIMAL(18,2)") && err.contains("INTEGER[]"),
                "{wrong}: {err}"
            );
        }
    }
}

#[test]
fn aggregate_set_level_return_type_applies_to_overloads_without_their_own() {
    let fx = Fixture::open();
    let overload = |param: TypeId| {
        AggregateOverloadBuilder::new()
            .param(param)
            .state_size(state_size)
            .init(state_init)
            .update(count_update)
            .combine(sum_combine)
            .finalize(sum_finalize)
    };
    // The count is written as an i64, which is DECIMAL(18,0)'s storage.
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        AggregateFunctionSetBuilder::try_new("set_default_ret")
            .expect("name")
            .returns_logical(LogicalType::decimal(18, 0))
            .overload(overload(TypeId::Varchar))
            .overload(overload(TypeId::Blob).returns(TypeId::BigInt))
            .register(fx.con())
    }
    .expect("register set_default_ret");
    assert_eq!(
        text(
            &fx,
            "SELECT typeof(set_default_ret('a'::VARCHAR)) FROM range(1)"
        ),
        "DECIMAL(18,0)",
        "an overload with no return type of its own takes the set's"
    );
    assert_eq!(
        text(
            &fx,
            "SELECT typeof(set_default_ret('a'::BLOB)) FROM range(1)"
        ),
        "BIGINT",
        "an overload's own return type wins over the set's"
    );
}

// ─── Scalars ─────────────────────────────────────────────────────────────────

quack_rs::scalar_callback!(count_arguments, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, chunk.column_count() as i64) };
    }
});

// The sum of every column from the second on, read as the raw `i64` of a
// DECIMAL(18, 2) and written back as one.
quack_rs::scalar_callback!(sum_decimal_varargs, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        let mut total = 0_i64;
        for col in 1..chunk.column_count() {
            total += unsafe { chunk.reader(col).read_i64(row) };
        }
        unsafe { writer.write_i64(row, total) };
    }
});

#[test]
fn scalar_varargs_logical_declares_the_variadic_type() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        ScalarFunctionBuilder::try_new("va_lists")
            .expect("name")
            .varargs_logical(LogicalType::list(TypeId::BigInt))
            .returns(TypeId::BigInt)
            .function(count_arguments)
            .register(fx.con())
            .expect("register va_lists");
        ScalarFunctionBuilder::try_new("va_decimal")
            .expect("name")
            .param(TypeId::Varchar)
            .varargs_logical(LogicalType::decimal(18, 2))
            .returns_logical(LogicalType::decimal(18, 2))
            .function(sum_decimal_varargs)
            .register(fx.con())
            .expect("register va_decimal");
    }
    assert_eq!(
        signatures(&fx, "va_lists"),
        [("[]".to_owned(), Some("BIGINT[]".to_owned()))]
    );
    assert_eq!(
        signatures(&fx, "va_decimal"),
        [("[VARCHAR]".to_owned(), Some("DECIMAL(18,2)".to_owned()))]
    );

    assert_eq!(int(&fx, "SELECT va_lists()"), 0);
    assert_eq!(int(&fx, "SELECT va_lists([1], [2, 3], [])"), 3);
    // A bare integer is not a list, and says which function refused it.
    let err = error_of(&fx, "SELECT va_lists(1)");
    assert!(
        err.contains("va_lists") && err.contains("BIGINT[]"),
        "the binder names the function and its variadic type: {err}"
    );

    // Integer arguments are cast to the declared DECIMAL(18,2), so the
    // callback's raw i64s are scaled by 100: 1 + 2.5 + 0.25 is 375.
    assert_eq!(
        text(&fx, "SELECT typeof(va_decimal('x', 1, 2.5, 0.25))"),
        "DECIMAL(18,2)"
    );
    assert_eq!(
        text(&fx, "SELECT va_decimal('x', 1, 2.5, 0.25)::VARCHAR"),
        "3.75"
    );
    assert_eq!(text(&fx, "SELECT va_decimal('x')::VARCHAR"), "0.00");
}

quack_rs::scalar_callback!(write_scaled_3, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row) * 1000) };
    }
});

quack_rs::scalar_callback!(write_length_9_1, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        let len = unsafe { reader.read_str(row) }.len() as i32;
        unsafe { writer.write_i32(row, len * 10) };
    }
});

quack_rs::scalar_callback!(write_half_4_2, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i16(row, 150) };
    }
});

#[test]
fn scalar_overloads_each_carry_their_own_logical_return_type() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        ScalarFunctionSetBuilder::try_new("ovl_ret")
            .expect("name")
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .returns_logical(LogicalType::decimal(18, 3))
                    .function(write_scaled_3),
            )
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Varchar)
                    .returns_logical(LogicalType::decimal(9, 1))
                    .function(write_length_9_1),
            )
            // Both set: the logical type wins.
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Double)
                    .returns(TypeId::BigInt)
                    .returns_logical(LogicalType::decimal(4, 2))
                    .function(write_half_4_2),
            )
            .register(fx.con())
    }
    .expect("register ovl_ret");

    for (call, ty, shown) in [
        ("ovl_ret(5)", "DECIMAL(18,3)", "5.000"),
        ("ovl_ret('abc')", "DECIMAL(9,1)", "3.0"),
        ("ovl_ret(0.5::DOUBLE)", "DECIMAL(4,2)", "1.50"),
    ] {
        assert_eq!(text(&fx, &format!("SELECT typeof({call})")), ty, "{call}");
        assert_eq!(
            text(&fx, &format!("SELECT {call}::VARCHAR")),
            shown,
            "{call}"
        );
    }
}

#[test]
fn scalar_overloads_declare_logical_varargs_and_parameters() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        ScalarFunctionSetBuilder::try_new("ovl_va")
            .expect("name")
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Varchar)
                    .varargs_logical(LogicalType::list(TypeId::Integer))
                    .returns(TypeId::BigInt)
                    .function(count_arguments),
            )
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .param_logical(LogicalType::map(TypeId::Varchar, TypeId::Double))
                    .param(TypeId::Date)
                    .returns(TypeId::BigInt)
                    .function(count_arguments),
            )
            .register(fx.con())
    }
    .expect("register ovl_va");
    assert_eq!(
        signatures(&fx, "ovl_va"),
        [
            ("[BIGINT, MAP(VARCHAR, DOUBLE), DATE]".to_owned(), None),
            ("[VARCHAR]".to_owned(), Some("INTEGER[]".to_owned())),
        ]
    );
    assert_eq!(int(&fx, "SELECT ovl_va('a', [1], [2, 3])"), 3);
    assert_eq!(
        int(&fx, "SELECT ovl_va(1, MAP {'k': 2.5}, DATE '2024-01-02')"),
        3
    );
}

/// A scalar overload declaring `slots` in order, returning the number of
/// arguments it was called with (it runs on NULL arguments too).
fn counting_scalar_overload(slots: &[Slot]) -> ScalarOverloadBuilder {
    let mut builder = ScalarOverloadBuilder::new()
        .returns(TypeId::BigInt)
        .null_handling(NullHandling::SpecialNullHandling)
        .function(count_arguments);
    for slot in slots {
        builder = match *slot {
            Slot::Id(id) => builder.param(id),
            Slot::Logical(_, make) => builder.param_logical(make()),
        };
    }
    builder
}

#[test]
fn scalar_param_logical_keeps_declaration_order_with_param() {
    let fx = Fixture::open();
    for (i, slots) in arrangements().iter().enumerate() {
        let name = format!("scalar_single_{i}");
        let mut builder = ScalarFunctionBuilder::try_new(&name)
            .expect("name")
            .returns(TypeId::BigInt)
            .null_handling(NullHandling::SpecialNullHandling)
            .function(count_arguments);
        for slot in slots {
            builder = match *slot {
                Slot::Id(id) => builder.param(id),
                Slot::Logical(_, make) => builder.param_logical(make()),
            };
        }
        // SAFETY: `con` is open; the callback matches its signature.
        unsafe { builder.register(fx.con()) }
            .unwrap_or_else(|e| panic!("register {name} {}: {e}", spelled(slots)));
        assert_eq!(
            signatures(&fx, &name),
            [(spelled(slots), None)],
            "{name}: registered parameter types"
        );
        // A call with exactly those types binds, and the callback sees one
        // column per declared parameter.
        let call = format!("SELECT {name}({})", null_args(slots));
        assert_eq!(int(&fx, &call), slots.len() as i64, "{call}");
    }
}

#[test]
fn scalar_overload_param_logical_keeps_declaration_order_with_param() {
    let fx = Fixture::open();
    let all = arrangements();
    let mut set = ScalarFunctionSetBuilder::try_new("scalar_overloads").expect("name");
    for slots in &all {
        set = set.overload(counting_scalar_overload(slots));
    }
    // SAFETY: `con` is open; the callback matches its signature.
    unsafe { set.register(fx.con()) }.expect("register scalar_overloads");

    let mut expected: Vec<(String, Option<String>)> =
        all.iter().map(|s| (spelled(s), None)).collect();
    expected.sort();
    assert_eq!(signatures(&fx, "scalar_overloads"), expected);
    for slots in &all {
        let call = format!("SELECT scalar_overloads({})", null_args(slots));
        assert_eq!(int(&fx, &call), slots.len() as i64, "{call}");
    }
}

// ─── Table functions ─────────────────────────────────────────────────────────

/// `p0 + 100 * len(p1) + 10_000 * len(p2) + 1_000_000 * len(p3)` over
/// (BIGINT, INTEGER[], VARCHAR, MAP(VARCHAR, BIGINT)), plus the named LIST's
/// length times `100_000_000` when given.
fn table_functions() -> Vec<(&'static str, TableFunctionBuilder)> {
    use quack_rs::value::Value;

    #[derive(Clone)]
    struct Total(i64);

    fn bind_total(bind: &quack_rs::table::BindInfo) -> Total {
        bind.add_result_column("total", TypeId::BigInt);
        // SAFETY: all four positional parameters are declared.
        let (p0, p1, p2, p3) = unsafe {
            (
                bind.get_parameter_value(0),
                bind.get_parameter_value(1),
                bind.get_parameter_value(2),
                bind.get_parameter_value(3),
            )
        };
        // SAFETY: "extra" is a declared named parameter.
        let extra: Value = unsafe { bind.get_named_parameter_value("extra") };
        let extra_len = if extra.is_null() {
            0
        } else {
            extra.list_len() as i64
        };
        Total(
            p0.as_i64_or(0)
                + 100 * p1.list_len() as i64
                + 10_000 * p2.as_str().map_or(0, |s| s.len() as i64)
                + 1_000_000 * p3.map_len() as i64
                + 100_000_000 * extra_len,
        )
    }

    #[allow(clippy::unnecessary_wraps)] // the scan closure's signature
    fn scan_total(
        total: &mut Total,
        chunk: &DataChunk,
    ) -> Result<(), quack_rs::error::ExtensionError> {
        // SAFETY: column 0 is BIGINT; a zero size ends the scan.
        unsafe {
            if total.0 >= 0 {
                chunk.writer(0).write_i64(0, total.0);
                chunk.set_size(1);
                total.0 = -1;
            } else {
                chunk.set_size(0);
            }
        }
        Ok(())
    }

    // Declared on the raw builder, then wrapped...
    let raw = TableFunctionBuilder::new("tbl_raw")
        .param(TypeId::BigInt)
        .param_logical(LogicalType::list(TypeId::Integer))
        .param(TypeId::Varchar)
        .param_logical(LogicalType::map(TypeId::Varchar, TypeId::BigInt))
        .named_param_logical("extra", LogicalType::list(TypeId::Varchar))
        .with_state::<Total, _>(|bind| Ok(bind_total(bind)))
        .scan(scan_total)
        .build()
        .expect("build tbl_raw");
    // ...and declared on the typed builder itself.
    let typed = TableFunctionBuilder::new("tbl_typed")
        .with_state::<Total, _>(|bind| Ok(bind_total(bind)))
        .param(TypeId::BigInt)
        .param_logical(LogicalType::list(TypeId::Integer))
        .param(TypeId::Varchar)
        .param_logical(LogicalType::map(TypeId::Varchar, TypeId::BigInt))
        .named_param_logical("extra", LogicalType::list(TypeId::Varchar))
        .scan(scan_total)
        .build()
        .expect("build tbl_typed");
    vec![("tbl_raw", raw), ("tbl_typed", typed)]
}

#[test]
fn table_param_logical_and_named_param_logical_keep_their_positions() {
    let fx = Fixture::open();
    for (name, builder) in table_functions() {
        // SAFETY: `con` is open.
        unsafe { builder.register(fx.con()) }.unwrap_or_else(|e| panic!("register {name}: {e}"));

        let call = format!("{name}(7, [1, 2, 3], 'ab', MAP {{'k': 1, 'j': 2}})");
        assert_eq!(
            int(&fx, &format!("SELECT total FROM {call}")),
            7 + 300 + 20_000 + 2_000_000,
            "{call}"
        );
        // The named LIST(VARCHAR) arrives as a list of that many items.
        let named = format!("{name}(7, [1, 2, 3], 'ab', MAP {{'k': 1}}, extra := ['a', 'b'])");
        assert_eq!(
            int(&fx, &format!("SELECT total FROM {named}")),
            7 + 300 + 20_000 + 1_000_000 + 200_000_000,
            "{named}"
        );
        // Declared types are enforced at their own positions, and the
        // binder lists the signature that was registered: the MAP's BIGINT
        // value type can only come from it, as neither call passes one.
        for wrong in [
            format!("{name}(7, 1, 'ab', MAP {{'k': 1}})"),
            format!("{name}(7, [1], 'ab', [2])"),
        ] {
            let err = error_of(&fx, &format!("SELECT * FROM {wrong}"));
            assert!(
                err.contains(name) && err.contains("MAP(VARCHAR, BIGINT)"),
                "{wrong}: {err}"
            );
        }
        let err = error_of(
            &fx,
            &format!("SELECT * FROM {name}(7, [1], 'ab', MAP {{'k': 1}}, extra := 5)"),
        );
        assert!(err.contains("VARCHAR[]"), "{err}");
    }
}
