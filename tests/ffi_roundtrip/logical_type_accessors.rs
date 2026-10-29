// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `LogicalType`'s introspection accessors, held to what the engine reports.
//!
//! Every type shape is built with the constructors and read back through the
//! accessors; the engine is asked for the same type through `typeof()`, a
//! function's declared return type, and a SQL cast, and the three must agree.
//! A [`describe`] that spells a type using nothing but the accessors turns
//! "returned the wrong child" into a string comparison.

use std::panic::{catch_unwind, AssertUnwindSafe};

use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::VectorWriter;

use super::Fixture;

/// Spells `ty` the way `DuckDB`'s `typeof()` does, reading it only through the
/// accessors under test.
///
/// # Safety
///
/// `ty` must be a valid logical type of a kind the match below handles.
unsafe fn describe(ty: &LogicalType) -> String {
    // SAFETY (whole body): each accessor is called on the kind its arm tests.
    unsafe {
        match ty.get_type_id() {
            TypeId::List => format!("{}[]", describe(&ty.list_child_type())),
            TypeId::Array => format!("{}[{}]", describe(&ty.array_child_type()), ty.array_size()),
            TypeId::Map => format!(
                "MAP({}, {})",
                describe(&ty.map_key_type()),
                describe(&ty.map_value_type())
            ),
            TypeId::Struct => {
                let fields: Vec<String> = (0..ty.struct_child_count())
                    .map(|i| {
                        format!(
                            "{} {}",
                            ty.struct_child_name(i),
                            describe(&ty.struct_child_type(i))
                        )
                    })
                    .collect();
                format!("STRUCT({})", fields.join(", "))
            }
            TypeId::Union => {
                let members: Vec<String> = (0..ty.union_member_count())
                    .map(|i| {
                        format!(
                            "{} {}",
                            ty.union_member_name(i),
                            describe(&ty.union_member_type(i))
                        )
                    })
                    .collect();
                format!("UNION({})", members.join(", "))
            }
            TypeId::Decimal => format!("DECIMAL({},{})", ty.decimal_width(), ty.decimal_scale()),
            TypeId::Enum => {
                let members: Vec<String> = (0..u64::from(ty.enum_dictionary_size()))
                    .map(|i| format!("'{}'", ty.enum_dictionary_value(i)))
                    .collect();
                format!("ENUM({})", members.join(", "))
            }
            other => other.sql_name().to_owned(),
        }
    }
}

/// One type shape: how `DuckDB` spells it, and how to build it.
struct Shape {
    sql: &'static str,
    make: fn() -> LogicalType,
}

/// A nested type with every part distinct, so a swapped or repeated child
/// shows up in the spelling.
fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            sql: "BIGINT[]",
            make: || LogicalType::list(TypeId::BigInt),
        },
        Shape {
            sql: "VARCHAR[][]",
            make: || LogicalType::list_from_logical(&LogicalType::list(TypeId::Varchar)),
        },
        Shape {
            sql: "MAP(VARCHAR, BIGINT)",
            make: || LogicalType::map(TypeId::Varchar, TypeId::BigInt),
        },
        Shape {
            sql: "MAP(INTEGER[], STRUCT(x DOUBLE, y BOOLEAN))",
            make: || {
                LogicalType::map_from_logical(
                    &LogicalType::list(TypeId::Integer),
                    &LogicalType::struct_type(&[("x", TypeId::Double), ("y", TypeId::Boolean)]),
                )
            },
        },
        Shape {
            sql: "STRUCT(a BIGINT, b VARCHAR, c DOUBLE[])",
            make: || {
                LogicalType::struct_type_from_logical(&[
                    ("a", LogicalType::new(TypeId::BigInt)),
                    ("b", LogicalType::new(TypeId::Varchar)),
                    ("c", LogicalType::list(TypeId::Double)),
                ])
            },
        },
        Shape {
            sql: "UNION(n BIGINT, s VARCHAR, d DOUBLE)",
            make: || {
                LogicalType::union_type(&[
                    ("n", TypeId::BigInt),
                    ("s", TypeId::Varchar),
                    ("d", TypeId::Double),
                ])
            },
        },
        Shape {
            sql: "UNION(l INTEGER[], m MAP(VARCHAR, BIGINT))",
            make: || {
                LogicalType::union_type_from_logical(&[
                    ("l", LogicalType::list(TypeId::Integer)),
                    ("m", LogicalType::map(TypeId::Varchar, TypeId::BigInt)),
                ])
            },
        },
        Shape {
            sql: "INTEGER[3]",
            make: || LogicalType::array(TypeId::Integer, 3),
        },
        Shape {
            sql: "VARCHAR[2][4]",
            make: || LogicalType::array_from_logical(&LogicalType::array(TypeId::Varchar, 2), 4),
        },
        Shape {
            sql: "DECIMAL(18,3)",
            make: || LogicalType::decimal(18, 3),
        },
        Shape {
            sql: "ENUM('red', 'green', 'blue')",
            make: || LogicalType::enum_type(&["red", "green", "blue"]),
        },
        Shape {
            sql: "STRUCT(m MAP(VARCHAR, UNION(i INTEGER, a BIGINT[2])), e ENUM('x', 'y'))",
            make: || {
                LogicalType::struct_type_from_logical(&[
                    (
                        "m",
                        LogicalType::map_from_logical(
                            &LogicalType::new(TypeId::Varchar),
                            &LogicalType::union_type_from_logical(&[
                                ("i", LogicalType::new(TypeId::Integer)),
                                ("a", LogicalType::array(TypeId::BigInt, 2)),
                            ]),
                        ),
                    ),
                    ("e", LogicalType::enum_type(&["x", "y"])),
                ])
            },
        },
    ]
}

#[test]
fn every_shape_built_by_the_constructors_reads_back_through_the_accessors() {
    let fx = Fixture::open();
    let _ = &fx; // the C API is initialised by the fixture
    for shape in shapes() {
        let ty = (shape.make)();
        // SAFETY: `ty` is a valid logical type of a kind `describe` handles.
        assert_eq!(unsafe { describe(&ty) }, shape.sql);
    }
}

quack_rs::scalar_callback!(write_nulls, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.set_null(row) };
    }
});

/// The same shapes, declared as a function's return type: the engine's own
/// `typeof()` and its result column type must say what the constructors built.
#[test]
fn the_engine_reports_the_type_the_constructors_built() {
    let fx = Fixture::open();
    for (i, shape) in shapes().into_iter().enumerate() {
        let name = format!("lt_shape_{i}");
        // SAFETY: `con` is open; the callback matches the signature.
        unsafe {
            ScalarFunctionBuilder::try_new(&name)
                .expect("name")
                .returns_logical((shape.make)())
                .function(write_nulls)
                .register(fx.con())
        }
        .unwrap_or_else(|e| panic!("register {name} ({}): {e}", shape.sql));

        // The engine's spelling of the declared type.
        let typeof_sql = format!("SELECT typeof({name}())");
        let spelled = fx
            .scalar(&typeof_sql, |r, i| unsafe { r.read_str(i).to_owned() })
            .expect("typeof is never NULL");
        assert_eq!(spelled, shape.sql, "{typeof_sql}");

        // The engine's type object for the result column, read back through
        // the accessors. No row is produced, so no value is built.
        let result = fx.query(&format!("SELECT {name}() FROM range(0)"));
        let column = result.column_logical_type(0).expect("one column");
        // SAFETY: `column` is a valid logical type of a handled kind.
        assert_eq!(unsafe { describe(&column) }, shape.sql, "{name}");
    }
}

/// The other direction: types the engine builds from SQL, read through the
/// accessors, must come out as the same strings the constructors produced.
#[test]
fn types_built_by_the_engine_read_back_through_the_accessors() {
    let fx = Fixture::open();
    for shape in shapes() {
        let result = fx.query(&format!("SELECT NULL::{}", shape.sql));
        let column = result.column_logical_type(0).expect("one column");
        // SAFETY: `column` is a valid logical type of a handled kind.
        assert_eq!(unsafe { describe(&column) }, shape.sql);
    }
}

/// A `MAP` is a `LIST` of `STRUCT(key, value)` underneath, and
/// `duckdb_list_type_child_type` serves both.
#[test]
fn a_maps_list_child_is_its_key_value_struct() {
    let fx = Fixture::open();
    let _ = &fx;
    let map = LogicalType::map(TypeId::Varchar, TypeId::BigInt);
    // SAFETY: `map` is a valid MAP type.
    unsafe {
        let entry = map.list_child_type();
        assert_eq!(entry.get_type_id(), TypeId::Struct);
        assert_eq!(entry.struct_child_count(), 2);
        assert_eq!(entry.struct_child_name(0), "key");
        assert_eq!(entry.struct_child_name(1), "value");
        assert_eq!(entry.struct_child_type(0).get_type_id(), TypeId::Varchar);
        assert_eq!(entry.struct_child_type(1).get_type_id(), TypeId::BigInt);
        // And the dedicated accessors agree with it.
        assert_eq!(map.map_key_type().get_type_id(), TypeId::Varchar);
        assert_eq!(map.map_value_type().get_type_id(), TypeId::BigInt);
    }
}

#[test]
fn a_decimals_width_scale_and_storage_type_follow_its_width() {
    let fx = Fixture::open();
    let _ = &fx;
    for (width, scale, storage) in [
        (1_u8, 0_u8, TypeId::SmallInt),
        (4, 4, TypeId::SmallInt),
        (5, 0, TypeId::Integer),
        (9, 2, TypeId::Integer),
        (10, 5, TypeId::BigInt),
        (18, 3, TypeId::BigInt),
        (19, 0, TypeId::HugeInt),
        (38, 38, TypeId::HugeInt),
    ] {
        let built = LogicalType::decimal(width, scale);
        let result = fx.query(&format!("SELECT NULL::DECIMAL({width},{scale})"));
        let engine = result.column_logical_type(0).expect("one column");
        for (source, ty) in [("constructor", &built), ("engine", &engine)] {
            // SAFETY: both are valid DECIMAL types.
            unsafe {
                assert_eq!(ty.get_type_id(), TypeId::Decimal, "{source}");
                assert_eq!(
                    ty.decimal_width(),
                    width,
                    "{source} DECIMAL({width},{scale})"
                );
                assert_eq!(
                    ty.decimal_scale(),
                    scale,
                    "{source} DECIMAL({width},{scale})"
                );
                assert_eq!(
                    ty.decimal_internal_type(),
                    storage,
                    "{source} DECIMAL({width},{scale})"
                );
            }
        }
    }
}

#[test]
fn an_enums_dictionary_and_storage_type_follow_its_size() {
    let fx = Fixture::open();
    let _ = &fx;
    // DuckDB stores an ENUM in the smallest unsigned integer that holds its
    // index: 255 members fit a byte, 65535 fit two, more need four.
    for (size, storage) in [
        (1_usize, TypeId::UTinyInt),
        (255, TypeId::UTinyInt),
        (256, TypeId::USmallInt),
        (65_535, TypeId::USmallInt),
        (65_536, TypeId::UInteger),
    ] {
        let names: Vec<String> = (0..size).map(|i| format!("member_{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let ty = LogicalType::enum_type(&refs);
        // SAFETY: `ty` is a valid ENUM type and every index is in range.
        unsafe {
            assert_eq!(ty.get_type_id(), TypeId::Enum);
            assert_eq!(u64::from(ty.enum_dictionary_size()), size as u64);
            assert_eq!(ty.enum_internal_type(), storage, "{size} members");
            for index in [0, size / 2, size - 1] {
                assert_eq!(
                    ty.enum_dictionary_value(index as u64),
                    names[index],
                    "member {index} of {size}"
                );
            }
        }
    }
}

#[test]
fn struct_and_union_members_are_read_by_position() {
    let fx = Fixture::open();
    let _ = &fx;
    let union = LogicalType::union_type(&[
        ("first", TypeId::Integer),
        ("second", TypeId::Varchar),
        ("third", TypeId::Double),
    ]);
    let record = LogicalType::struct_type(&[
        ("x", TypeId::Float),
        ("y", TypeId::Blob),
        ("z", TypeId::Date),
        ("w", TypeId::Uuid),
    ]);
    // SAFETY: both are valid types and every index is in range.
    unsafe {
        assert_eq!(union.union_member_count(), 3);
        let members: Vec<(String, TypeId)> = (0..3)
            .map(|i| {
                (
                    union.union_member_name(i),
                    union.union_member_type(i).get_type_id(),
                )
            })
            .collect();
        assert_eq!(
            members,
            [
                ("first".to_owned(), TypeId::Integer),
                ("second".to_owned(), TypeId::Varchar),
                ("third".to_owned(), TypeId::Double),
            ]
        );
        assert_eq!(record.struct_child_count(), 4);
        let fields: Vec<(String, TypeId)> = (0..4)
            .map(|i| {
                (
                    record.struct_child_name(i),
                    record.struct_child_type(i).get_type_id(),
                )
            })
            .collect();
        assert_eq!(
            fields,
            [
                ("x".to_owned(), TypeId::Float),
                ("y".to_owned(), TypeId::Blob),
                ("z".to_owned(), TypeId::Date),
                ("w".to_owned(), TypeId::Uuid),
            ]
        );
    }
}

#[test]
fn an_arrays_size_is_exact_up_to_the_limit() {
    let fx = Fixture::open();
    let _ = &fx;
    for size in [1_u64, 2, 3, 1000, 99_999] {
        let ty = LogicalType::array(TypeId::SmallInt, size);
        // SAFETY: `ty` is a valid ARRAY type.
        unsafe {
            assert_eq!(ty.get_type_id(), TypeId::Array);
            assert_eq!(ty.array_size(), size);
            assert_eq!(ty.array_child_type().get_type_id(), TypeId::SmallInt);
        }
    }
}

/// Asking for the wrong kind is either a zero (documented as safe at each
/// accessor) or the `from_raw` null-pointer panic — never a crash or a
/// plausible-looking wrong answer.
#[test]
fn an_accessor_on_the_wrong_kind_answers_zero_or_panics_cleanly() {
    let fx = Fixture::open();
    let _ = &fx;
    let plain = LogicalType::new(TypeId::BigInt);
    // SAFETY: `plain` is valid; the C API checks the kind itself for each of
    // these and answers zero.
    unsafe {
        assert_eq!(plain.decimal_width(), 0);
        assert_eq!(plain.decimal_scale(), 0);
        assert_eq!(plain.enum_dictionary_size(), 0);
        assert_eq!(plain.struct_child_count(), 0);
        assert_eq!(plain.union_member_count(), 0);
        assert_eq!(plain.array_size(), 0);
    }

    type Accessor = fn(&LogicalType);
    let child_accessors: [(&str, Accessor); 4] = [
        ("list_child_type", |t| {
            // SAFETY: a valid type; the C API returns null for the wrong kind.
            drop(unsafe { t.list_child_type() });
        }),
        ("map_key_type", |t| {
            // SAFETY: as above.
            drop(unsafe { t.map_key_type() });
        }),
        ("map_value_type", |t| {
            // SAFETY: as above.
            drop(unsafe { t.map_value_type() });
        }),
        ("array_child_type", |t| {
            // SAFETY: as above.
            drop(unsafe { t.array_child_type() });
        }),
    ];
    for (name, call) in child_accessors {
        let panic = catch_unwind(AssertUnwindSafe(|| call(&plain)))
            .expect_err(&format!("{name} on a BIGINT must panic, not wrap a null"));
        let message = quack_rs::callback::panic_message(&panic);
        assert!(message.contains("null pointer"), "{name}: {message}");
    }

    // The storage-type accessors get INVALID from the C API for the wrong
    // kind, which has no `TypeId`, so they panic rather than invent one.
    for (name, call) in [
        (
            "decimal_internal_type",
            (|t: &LogicalType| {
                // SAFETY: a valid type; the C API answers INVALID.
                let _ = unsafe { t.decimal_internal_type() };
            }) as Accessor,
        ),
        (
            "enum_internal_type",
            (|t: &LogicalType| {
                // SAFETY: as above.
                let _ = unsafe { t.enum_internal_type() };
            }) as Accessor,
        ),
    ] {
        let panic = catch_unwind(AssertUnwindSafe(|| call(&plain)))
            .expect_err(&format!("{name} on a BIGINT has no TypeId to return"));
        let message = quack_rs::callback::panic_message(&panic);
        assert!(message.contains("unknown DUCKDB_TYPE"), "{name}: {message}");
    }
}

#[test]
fn the_try_constructors_build_what_their_panicking_forms_do() {
    let fx = Fixture::open();
    let _ = &fx;
    let list = LogicalType::try_list(TypeId::Varchar).expect("list");
    let map = LogicalType::try_map(TypeId::Integer, TypeId::Double).expect("map");
    let decimal = LogicalType::try_decimal(12, 4).expect("decimal");
    let array = LogicalType::try_array(TypeId::Float, 5).expect("array");
    // SAFETY: each is a valid type of the kind its accessors are called on.
    unsafe {
        assert_eq!(describe(&list), "VARCHAR[]");
        assert_eq!(describe(&map), "MAP(INTEGER, DOUBLE)");
        assert_eq!(describe(&decimal), "DECIMAL(12,4)");
        assert_eq!(describe(&array), "FLOAT[5]");
    }
    // A composite element needs the `*_from_logical` form and says so.
    let err = LogicalType::try_list(TypeId::Struct).expect_err("a bare STRUCT id");
    assert!(err.to_string().contains("struct_type"), "{err}");
    assert!(LogicalType::try_map(TypeId::Varchar, TypeId::List).is_err());
}

#[test]
fn an_alias_is_set_read_and_seen_by_the_engine() {
    let fx = Fixture::open();
    let mood = LogicalType::enum_type(&["sad", "ok", "happy"]);
    // SAFETY: `mood` is a valid logical type throughout.
    unsafe {
        assert_eq!(mood.get_alias(), None, "a fresh type has no alias");
        // A refused alias leaves the type as it was.
        assert!(mood.try_set_alias("bad\0alias").is_err());
        assert_eq!(mood.get_alias(), None);
        mood.try_set_alias("temp_mood").expect("a plain alias");
        assert_eq!(mood.get_alias().as_deref(), Some("temp_mood"));
        // Setting it again replaces it.
        mood.set_alias("mood");
        assert_eq!(mood.get_alias().as_deref(), Some("mood"));
        // The Debug form names the alias and the decoded type.
        let debug = format!("{mood:?}");
        assert!(
            debug.contains("Enum") && debug.contains("\"mood\""),
            "{debug}"
        );
        mood.register(fx.con()).expect("register the alias");
    }

    // The engine now knows the type by that name, and a value of it carries
    // the alias back out.
    let typeof_mood = fx
        .scalar("SELECT typeof('happy'::mood)", |r, i| unsafe {
            r.read_str(i).to_owned()
        })
        .expect("typeof is never NULL");
    assert_eq!(typeof_mood, "mood");
    let result = fx.query("SELECT 'ok'::mood");
    let column = result.column_logical_type(0).expect("one column");
    // SAFETY: `column` is a valid ENUM type.
    unsafe {
        assert_eq!(column.get_alias().as_deref(), Some("mood"));
        assert_eq!(column.get_type_id(), TypeId::Enum);
        assert_eq!(column.enum_dictionary_size(), 3);
        assert_eq!(column.enum_dictionary_value(2), "happy");
        // An alias is not part of the plain enum type next to it.
        let plain = fx.query("SELECT 'ok'::ENUM('sad', 'ok', 'happy')");
        let plain = plain.column_logical_type(0).expect("one column");
        assert_eq!(plain.get_alias(), None);
    }
}

#[test]
fn a_raw_handle_round_trips_through_into_raw_and_from_raw() {
    let fx = Fixture::open();
    let _ = &fx;
    let ty = LogicalType::map(TypeId::Varchar, TypeId::Date);
    let as_raw = ty.as_raw();
    let raw = ty.into_raw();
    assert_eq!(raw, as_raw, "into_raw hands back the handle as_raw showed");
    // SAFETY: `raw` is the live handle `into_raw` released, owned again here.
    let back = unsafe { LogicalType::from_raw(raw) };
    // SAFETY: `back` is a valid MAP type.
    assert_eq!(unsafe { describe(&back) }, "MAP(VARCHAR, DATE)");
}
