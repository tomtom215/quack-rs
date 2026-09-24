// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Detecting overloads that accept the same call.
//!
//! `DuckDB` accepts a function set in which two overloads take identical
//! arguments — registration succeeds — and then fails *every* call to it with
//! "Could not choose a best candidate function"; with varargs the same happens
//! for the calls both overloads accept (see [`shapes_overlap`]). Nothing
//! points back at the registration. The set builders call [`reject_duplicate_overloads`] before
//! allocating any `DuckDB` handle so the mistake is reported where it was made.
//!
//! Shared by the scalar and aggregate set builders.

use std::fmt::Write as _;
use std::os::raw::c_char;

use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_array_type_child_type, duckdb_decimal_scale,
    duckdb_decimal_width, duckdb_enum_dictionary_size, duckdb_enum_dictionary_value, duckdb_free,
    duckdb_get_type_id, duckdb_list_type_child_type, duckdb_logical_type,
    duckdb_logical_type_get_alias, duckdb_map_type_key_type, duckdb_map_type_value_type,
    duckdb_scalar_function, duckdb_scalar_function_set_varargs, duckdb_struct_type_child_count,
    duckdb_struct_type_child_name, duckdb_struct_type_child_type, duckdb_union_type_member_count,
    duckdb_union_type_member_name, duckdb_union_type_member_type, idx_t,
    DUCKDB_TYPE_DUCKDB_TYPE_ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL, DUCKDB_TYPE_DUCKDB_TYPE_ENUM,
    DUCKDB_TYPE_DUCKDB_TYPE_LIST, DUCKDB_TYPE_DUCKDB_TYPE_MAP, DUCKDB_TYPE_DUCKDB_TYPE_STRUCT,
    DUCKDB_TYPE_DUCKDB_TYPE_UNION,
};

use crate::error::ExtensionError;
use crate::types::logical_type::SlotCheck;
use crate::types::{LogicalType, TypeId};

/// One declared parameter, however it was declared.
///
/// `L` is always [`LogicalType`] outside this module's tests, which use a
/// stand-in so the interleaving can be checked without a live `DuckDB`.
pub enum ParamRef<'a, L = LogicalType> {
    /// Declared with `param(TypeId)`.
    Id(TypeId),
    /// Declared with `param_logical(LogicalType)`.
    Logical(&'a L),
    /// The overload's varargs type, which `DuckDB` counts as part of the
    /// signature (`ScalarFunction::Equal`). Always last.
    Varargs(&'a L),
    /// As [`Varargs`][Self::Varargs], declared with `varargs(TypeId)`.
    VarargsId(TypeId),
}

/// A varargs type as declared.
///
/// A `TypeId` is kept as one until registration, where it is checked with the
/// other slots and only then turned into a handle, so a composite id is an
/// error from `register` rather than a panic from the setter.
#[derive(Debug)]
pub enum Varargs {
    /// Declared with `varargs(TypeId)`.
    Id(TypeId),
    /// Declared with `varargs_logical(LogicalType)`.
    Logical(LogicalType),
}

impl Varargs {
    /// This varargs type as the last entry of a signature.
    pub const fn param_ref(&self) -> ParamRef<'_> {
        match self {
            Self::Id(id) => ParamRef::VarargsId(*id),
            Self::Logical(lt) => ParamRef::Varargs(lt),
        }
    }

    /// Runs `check` on a `TypeId` varargs type; `slot` names it.
    pub fn check(&self, check: SlotCheck, slot: &str) -> Result<(), ExtensionError> {
        match self {
            Self::Id(id) => check(*id, slot),
            Self::Logical(_) => Ok(()),
        }
    }

    /// Sets this varargs type on `func` (`duckdb_scalar_function_set_varargs`,
    /// which copies the type).
    ///
    /// # Safety
    ///
    /// `func` must be a live scalar function handle, and a `TypeId` varargs
    /// type must have passed [`check`][Self::check], so building it cannot hit
    /// the composite-type panic in [`LogicalType::new`].
    pub unsafe fn set_on(&self, func: duckdb_scalar_function) {
        let built;
        let ty = match self {
            Self::Id(id) => {
                built = LogicalType::new(*id);
                &built
            }
            Self::Logical(lt) => lt,
        };
        // SAFETY: `func` is live per this function's contract, and `ty` is a
        // live handle for the duration of the call.
        unsafe { duckdb_scalar_function_set_varargs(func, ty.as_raw()) };
    }
}

/// The parameters in the order the set builders hand them to `DuckDB`.
///
/// This mirrors the interleaving loop in both `register` implementations
/// exactly — including what it does with an out-of-order logical position — so
/// the signature compared here is the one `DuckDB` actually receives.
pub fn merged_params<'a, L>(
    params: &'a [TypeId],
    logical: &'a [(usize, L)],
) -> Vec<ParamRef<'a, L>> {
    let mut out = Vec::with_capacity(params.len() + logical.len());
    let mut simple_idx = 0;
    let mut logical_idx = 0;
    for pos in 0..params.len() + logical.len() {
        if logical_idx < logical.len() && logical[logical_idx].0 == pos {
            out.push(ParamRef::Logical(&logical[logical_idx].1));
            logical_idx += 1;
        } else if simple_idx < params.len() {
            out.push(ParamRef::Id(params[simple_idx]));
            simple_idx += 1;
        }
    }
    out
}

/// One overload's argument shape: its fixed parameter types and its varargs
/// type, if any, each rendered to a string that is equal exactly when the
/// types are.
#[derive(Debug, Clone, Copy)]
pub struct Shape<'a> {
    /// The fixed parameter types, in order.
    pub fixed: &'a [String],
    /// The varargs type, if the overload takes varargs.
    pub varargs: Option<&'a str>,
}

impl Shape<'_> {
    /// The type this overload expects at argument `index`, for a call with
    /// more than `index` arguments that it accepts.
    fn at(&self, index: usize) -> Option<&str> {
        self.fixed.get(index).map(String::as_str).or(self.varargs)
    }
}

/// Whether some call matches `a` and `b` with identical argument types, so
/// `DuckDB` can prefer neither.
///
/// A fixed overload accepts exactly `fixed.len()` arguments; a varargs one
/// accepts `fixed.len()` or more, the extra ones of its varargs type (none
/// included: `f(ANY...)` matches `f()`). Two overloads collide when an
/// argument count both accept sees the same type at every position. Types are
/// compared as rendered, so `f(BIGINT)` and `f(ANY...)` do not collide —
/// `DuckDB` resolves that call to the exact match, which is cheaper — while
/// `f(BIGINT)` and `f(BIGINT, BIGINT...)` do, at one argument.
#[must_use]
pub fn shapes_overlap(a: Shape<'_>, b: Shape<'_>) -> bool {
    let (la, lb) = (a.fixed.len(), b.fixed.len());
    let arity = match (a.varargs.is_some(), b.varargs.is_some()) {
        (false, false) if la == lb => la,
        (false, true) if la >= lb => la,
        (true, false) if lb >= la => lb,
        (true, true) => la.max(lb),
        _ => return false,
    };
    (0..arity).all(|i| a.at(i) == b.at(i))
}

/// Fails if two overloads accept the same argument list.
///
/// `signatures[i]` is overload `i`'s merged parameter list. Types are compared
/// structurally — `DECIMAL(18,2)` and `DECIMAL(18,3)` differ, as do two
/// `STRUCT`s with different field names, two `ENUM`s with different members,
/// or two types with different aliases — and varargs are expanded as
/// [`shapes_overlap`] describes, so only a set `DuckDB` genuinely cannot
/// resolve is rejected: `{f(BIGINT), f(BIGINT, BIGINT...)}` is, because
/// `f(1)` matches both.
///
/// # Safety
///
/// Every [`ParamRef::Logical`] and [`ParamRef::Varargs`] must hold a live
/// logical type, and the C API
/// must be initialised if any is present. A signature made only of
/// [`ParamRef::Id`]s makes no `DuckDB` call.
pub unsafe fn reject_duplicate_overloads(
    name: &str,
    signatures: &[Vec<ParamRef<'_>>],
) -> Result<(), ExtensionError> {
    let parts: Vec<(Vec<String>, Option<String>)> = signatures
        .iter()
        // SAFETY: forwarded from this function's own contract.
        .map(|params| unsafe { signature_parts(params) })
        .collect();
    let shapes: Vec<Shape<'_>> = parts
        .iter()
        .map(|(fixed, varargs)| Shape {
            fixed,
            varargs: varargs.as_deref(),
        })
        .collect();
    match first_overlap(&shapes) {
        None => Ok(()),
        Some((first, second)) => Err(ExtensionError::new(format!(
            "function set '{name}': overload {first} ({a}) and overload {second} ({b}) accept \
             the same arguments; DuckDB would accept the set but fail every such call with \
             \"Could not choose a best candidate function\"",
            a = display_shape(shapes[first]),
            b = display_shape(shapes[second]),
        ))),
    }
}

/// The indices of the first pair of overlapping shapes, lowest first.
fn first_overlap(shapes: &[Shape<'_>]) -> Option<(usize, usize)> {
    shapes.iter().enumerate().find_map(|(j, shape)| {
        shapes[..j]
            .iter()
            .position(|earlier| shapes_overlap(*earlier, *shape))
            .map(|i| (i, j))
    })
}

/// Renders a shape for an error message: `BIGINT, VARCHAR...`, or `()` for
/// none.
fn display_shape(shape: Shape<'_>) -> String {
    let mut out = shape.fixed.join(", ");
    if let Some(varargs) = shape.varargs {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(varargs);
        out.push_str("...");
    }
    if out.is_empty() {
        out.push_str("()");
    }
    out
}

/// Renders each parameter, and the varargs type, as comparable,
/// human-readable strings.
///
/// # Safety
///
/// As [`reject_duplicate_overloads`].
unsafe fn signature_parts(params: &[ParamRef<'_>]) -> (Vec<String>, Option<String>) {
    let mut fixed = Vec::with_capacity(params.len());
    let mut varargs = None;
    for param in params {
        let mut out = String::new();
        match param {
            ParamRef::Id(id) => fixed.push(id.sql_name().to_owned()),
            ParamRef::Logical(lt) => {
                // SAFETY: the handle is live per this function's contract.
                unsafe { describe(lt.as_raw(), &mut out) };
                fixed.push(out);
            }
            ParamRef::Varargs(lt) => {
                // SAFETY: as above.
                unsafe { describe(lt.as_raw(), &mut out) };
                varargs = Some(out);
            }
            ParamRef::VarargsId(id) => varargs = Some(id.sql_name().to_owned()),
        }
    }
    (fixed, varargs)
}

/// A child logical type handle, destroyed on drop.
///
/// A non-null handle is also held as a [`LogicalType`], whose own `Drop`
/// destroys it; a null one (an accessor that could not describe the child) is
/// never passed to `duckdb_destroy_logical_type`.
struct Owned {
    /// The handle, or null.
    raw: duckdb_logical_type,
    /// Destroys `raw` on drop; `None` exactly when `raw` is null.
    _guard: Option<LogicalType>,
}

impl Owned {
    /// Takes ownership of `raw`.
    ///
    /// # Safety
    ///
    /// `raw` must be null, or a handle returned by a `*_child_type` style
    /// accessor, which transfers ownership to the caller.
    unsafe fn new(raw: duckdb_logical_type) -> Self {
        Self {
            raw,
            // SAFETY: non-null, and ours to destroy, per this function's contract.
            _guard: (!raw.is_null()).then(|| unsafe { LogicalType::from_raw(raw) }),
        }
    }
}

/// Copies and frees a `DuckDB`-allocated C string. Null becomes `"?"`.
///
/// # Safety
///
/// `ptr` must be null or a string `DuckDB` allocated for the caller to free.
unsafe fn take_c_string(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        return String::from("?");
    }
    // SAFETY: `ptr` is a live NUL-terminated string per this function's contract.
    let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `ptr` was allocated by DuckDB for us to free, and is freed once.
    unsafe { duckdb_free(ptr.cast()) };
    s
}

/// Appends a structural rendering of `ty` to `out`.
///
/// # Safety
///
/// `ty` must be a live logical type handle.
unsafe fn describe(ty: duckdb_logical_type, out: &mut String) {
    // SAFETY: (whole body) `ty` is a live handle per this function's `# Safety`
    // clause; `duckdb_logical_type_get_alias` and `duckdb_get_type_id` need
    // only that. Every other accessor is called in the arm for the type id it
    // checks (logical_types-c.cpp), so none returns null here: each child
    // handle is a fresh `new LogicalType` owned by an `Owned` guard, and is
    // live when it is passed on to the recursive `describe`. The loop indices
    // stay below the count DuckDB reported (`child_count`, `member_count`,
    // `dictionary_size`), which the index accessors only `D_ASSERT`. Every
    // returned string is a `strdup` copy that `take_c_string` frees once.
    unsafe {
        let alias = duckdb_logical_type_get_alias(ty);
        if !alias.is_null() {
            let _ = write!(out, "{}:", take_c_string(alias));
        }
        let raw = duckdb_get_type_id(ty);
        match raw {
            DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL => {
                let _ = write!(
                    out,
                    "DECIMAL({},{})",
                    duckdb_decimal_width(ty),
                    duckdb_decimal_scale(ty)
                );
            }
            DUCKDB_TYPE_DUCKDB_TYPE_LIST => {
                let child = Owned::new(duckdb_list_type_child_type(ty));
                out.push_str("LIST(");
                describe(child.raw, out);
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_ARRAY => {
                let child = Owned::new(duckdb_array_type_child_type(ty));
                out.push_str("ARRAY(");
                describe(child.raw, out);
                let _ = write!(out, ", {})", duckdb_array_type_array_size(ty));
            }
            DUCKDB_TYPE_DUCKDB_TYPE_MAP => {
                let key = Owned::new(duckdb_map_type_key_type(ty));
                let value = Owned::new(duckdb_map_type_value_type(ty));
                out.push_str("MAP(");
                describe(key.raw, out);
                out.push_str(", ");
                describe(value.raw, out);
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_STRUCT => {
                out.push_str("STRUCT(");
                for i in 0..duckdb_struct_type_child_count(ty) {
                    describe_member(
                        out,
                        i,
                        duckdb_struct_type_child_name(ty, i),
                        duckdb_struct_type_child_type(ty, i),
                    );
                }
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_UNION => {
                out.push_str("UNION(");
                for i in 0..duckdb_union_type_member_count(ty) {
                    describe_member(
                        out,
                        i,
                        duckdb_union_type_member_name(ty, i),
                        duckdb_union_type_member_type(ty, i),
                    );
                }
                out.push(')');
            }
            DUCKDB_TYPE_DUCKDB_TYPE_ENUM => {
                out.push_str("ENUM(");
                for i in 0..idx_t::from(duckdb_enum_dictionary_size(ty)) {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(
                        out,
                        "{:?}",
                        take_c_string(duckdb_enum_dictionary_value(ty, i))
                    );
                }
                out.push(')');
            }
            other => match TypeId::try_from_duckdb_type(other) {
                Some(id) => out.push_str(id.sql_name()),
                None => {
                    let _ = write!(out, "TYPE#{other}");
                }
            },
        }
    }
}

/// Appends `name type` for one `STRUCT` field or `UNION` member, taking
/// ownership of both handles.
///
/// # Safety
///
/// `name` must be null or a `DuckDB`-allocated string, and `ty` a logical type
/// handle the caller owns.
unsafe fn describe_member(out: &mut String, i: idx_t, name: *mut c_char, ty: duckdb_logical_type) {
    // SAFETY: forwarded from this function's own contract.
    let ty = unsafe { Owned::new(ty) };
    if i > 0 {
        out.push_str(", ");
    }
    // SAFETY: forwarded from this function's own contract.
    let name = unsafe { take_c_string(name) };
    let _ = write!(out, "{name:?} ");
    // SAFETY: `ty` is a live handle owned by the guard above.
    unsafe { describe(ty.raw, out) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Owned parts for a [`Shape`]: fixed types and an optional varargs type.
    fn parts(fixed: &[&str], varargs: Option<&str>) -> (Vec<String>, Option<String>) {
        (
            fixed.iter().map(|t| (*t).to_owned()).collect(),
            varargs.map(str::to_owned),
        )
    }

    fn shape(p: &(Vec<String>, Option<String>)) -> Shape<'_> {
        Shape {
            fixed: &p.0,
            varargs: p.1.as_deref(),
        }
    }

    fn overlap(a: &(Vec<String>, Option<String>), b: &(Vec<String>, Option<String>)) -> bool {
        let forward = shapes_overlap(shape(a), shape(b));
        assert_eq!(forward, shapes_overlap(shape(b), shape(a)), "symmetric");
        forward
    }

    #[test]
    fn fixed_signatures_overlap_only_when_identical() {
        let bigint = parts(&["BIGINT"], None);
        assert!(overlap(&bigint, &bigint));
        assert!(!overlap(&bigint, &parts(&["INTEGER"], None)));
        assert!(!overlap(&bigint, &parts(&["BIGINT", "BIGINT"], None)));
        assert!(!overlap(&bigint, &parts(&[], None)));
        assert!(overlap(&parts(&[], None), &parts(&[], None)));
    }

    /// The three shapes the fourth audit's F6 registered against `DuckDB`
    /// 1.5.5, each of which failed calls with "Could not choose a best
    /// candidate function".
    #[test]
    fn varargs_overlap_the_way_duckdb_found_them_ambiguous() {
        assert!(overlap(
            &parts(&["BIGINT"], None),
            &parts(&["BIGINT"], Some("BIGINT"))
        ));
        assert!(overlap(&parts(&[], None), &parts(&[], Some("ANY"))));
        assert!(overlap(
            &parts(&[], Some("BIGINT")),
            &parts(&["BIGINT"], Some("BIGINT"))
        ));
    }

    #[test]
    fn varargs_overlap_only_where_an_argument_count_sees_equal_types() {
        // An exact match is cheaper than ANY, so DuckDB resolves it.
        assert!(!overlap(
            &parts(&["BIGINT"], None),
            &parts(&[], Some("ANY"))
        ));
        // f(VARCHAR, BIGINT...) takes at least one argument.
        assert!(!overlap(
            &parts(&[], None),
            &parts(&["VARCHAR"], Some("BIGINT"))
        ));
        // Different varargs types still meet where neither has extra
        // arguments: both accept `f('x')`.
        assert!(overlap(
            &parts(&["VARCHAR"], Some("BIGINT")),
            &parts(&["VARCHAR"], Some("DOUBLE"))
        ));
        // Same varargs, different fixed prefix.
        assert!(!overlap(
            &parts(&["VARCHAR"], Some("BIGINT")),
            &parts(&["DOUBLE"], Some("BIGINT"))
        ));
        // A fixed type that differs from the other's varargs type.
        assert!(!overlap(
            &parts(&["BIGINT", "DOUBLE"], None),
            &parts(&["BIGINT"], Some("BIGINT"))
        ));
    }

    #[test]
    fn two_varargs_meet_at_the_longer_fixed_prefix() {
        assert!(overlap(
            &parts(&["VARCHAR"], Some("BIGINT")),
            &parts(&["VARCHAR", "BIGINT", "BIGINT"], Some("BIGINT"))
        ));
        assert!(!overlap(
            &parts(&["VARCHAR"], Some("BIGINT")),
            &parts(&["VARCHAR", "DOUBLE"], Some("BIGINT"))
        ));
    }

    #[test]
    fn the_first_overlapping_pair_is_reported_lowest_first() {
        let all = [
            parts(&["DOUBLE"], None),
            parts(&["BIGINT"], None),
            parts(&["VARCHAR"], None),
            parts(&["BIGINT"], None),
            parts(&["DOUBLE"], None),
        ];
        let shapes: Vec<Shape<'_>> = all
            .iter()
            .map(|p| Shape {
                fixed: &p.0,
                varargs: p.1.as_deref(),
            })
            .collect();
        // Overload 3 is the first to overlap an earlier one (1).
        assert_eq!(first_overlap(&shapes), Some((1, 3)));
        assert_eq!(first_overlap(&shapes[..3]), None);
    }

    #[test]
    fn shapes_display_as_signatures() {
        let p = parts(&["BIGINT", "VARCHAR"], Some("DOUBLE"));
        let shape = Shape {
            fixed: &p.0,
            varargs: p.1.as_deref(),
        };
        assert_eq!(display_shape(shape), "BIGINT, VARCHAR, DOUBLE...");
        let empty = parts(&[], None);
        assert_eq!(
            display_shape(Shape {
                fixed: &empty.0,
                varargs: None
            }),
            "()"
        );
        let only_varargs = parts(&[], Some("ANY"));
        assert_eq!(
            display_shape(Shape {
                fixed: &only_varargs.0,
                varargs: only_varargs.1.as_deref()
            }),
            "ANY..."
        );
    }

    #[test]
    fn the_error_names_both_overloads_and_the_signature() {
        let sigs = vec![
            vec![ParamRef::Id(TypeId::BigInt), ParamRef::Id(TypeId::Varchar)],
            vec![ParamRef::Id(TypeId::Double)],
            vec![ParamRef::Id(TypeId::BigInt), ParamRef::Id(TypeId::Varchar)],
        ];
        // SAFETY: only `ParamRef::Id`, so no DuckDB call is made.
        let err = unsafe { reject_duplicate_overloads("my_fn", &sigs) }.expect_err("duplicate");
        let msg = err.as_str();
        assert!(msg.contains("my_fn"), "{msg}");
        assert!(
            msg.contains("overload 0 (BIGINT, VARCHAR) and overload 2 (BIGINT, VARCHAR)"),
            "{msg}"
        );
    }

    /// `merged_params` with `&str` stand-ins for the logical types, rendered
    /// as `Id(..)` / `Logical(..)` so a whole ordering compares at once.
    fn merged(params: &[TypeId], logical: &[(usize, &'static str)]) -> Vec<String> {
        merged_params(params, logical)
            .iter()
            .map(|p| match p {
                ParamRef::Id(id) => format!("Id({})", id.sql_name()),
                ParamRef::Logical(name) => format!("Logical({name})"),
                ParamRef::Varargs(name) => format!("Varargs({name})"),
                ParamRef::VarargsId(id) => format!("VarargsId({})", id.sql_name()),
            })
            .collect()
    }

    /// A varargs type declared by `TypeId` renders without `DuckDB` and takes
    /// part in the comparison: `f(BIGINT...)` accepts one `BIGINT`, so beside
    /// `f(BIGINT)` the call `f(1)` is ambiguous, while `f(DOUBLE...)` is not.
    #[test]
    fn varargs_by_type_id_is_part_of_the_signature() {
        let sigs = vec![
            vec![ParamRef::Id(TypeId::BigInt)],
            vec![ParamRef::VarargsId(TypeId::BigInt)],
        ];
        // SAFETY: no logical handles, so no DuckDB call is made.
        let err = unsafe { reject_duplicate_overloads("f", &sigs) }
            .expect_err("f(BIGINT) vs f(BIGINT...)");
        assert!(
            err.as_str().contains("(BIGINT) and overload 1 (BIGINT...)"),
            "{err}"
        );
        let sigs = vec![
            vec![ParamRef::Id(TypeId::BigInt)],
            vec![ParamRef::VarargsId(TypeId::Double)],
        ];
        // SAFETY: as above.
        unsafe { reject_duplicate_overloads("f", &sigs) }.expect("f(BIGINT) vs f(DOUBLE...)");

        let sigs = vec![
            vec![
                ParamRef::Id(TypeId::Varchar),
                ParamRef::VarargsId(TypeId::BigInt),
            ],
            vec![
                ParamRef::Id(TypeId::Varchar),
                ParamRef::VarargsId(TypeId::BigInt),
            ],
        ];
        // SAFETY: as above.
        let err = unsafe { reject_duplicate_overloads("f", &sigs) }.expect_err("duplicate");
        assert!(err.as_str().contains("(VARCHAR, BIGINT...)"), "{err}");
    }

    #[test]
    fn merged_params_places_logical_params_at_their_positions() {
        assert_eq!(
            merged(
                &[TypeId::BigInt, TypeId::Varchar, TypeId::Double],
                &[(0, "a"), (2, "b")]
            ),
            vec![
                "Logical(a)",
                "Id(BIGINT)",
                "Logical(b)",
                "Id(VARCHAR)",
                "Id(DOUBLE)",
            ]
        );
        // A trailing logical parameter, after every simple one.
        assert_eq!(
            merged(&[TypeId::BigInt, TypeId::Varchar], &[(2, "z")]),
            vec!["Id(BIGINT)", "Id(VARCHAR)", "Logical(z)"]
        );
        // Only logical parameters.
        assert_eq!(
            merged(&[], &[(0, "a"), (1, "b")]),
            vec!["Logical(a)", "Logical(b)"]
        );
        assert_eq!(merged(&[], &[]), Vec::<String>::new());
    }

    #[test]
    fn merged_params_mirrors_register_for_out_of_order_positions() {
        // `register` walks positions 0..total and only takes a logical type
        // when the next one's position is the current one. A position past the
        // end is never reached, so that logical type is left out and the slot
        // it would have filled stays empty.
        assert_eq!(
            merged(&[TypeId::BigInt], &[(5, "late")]),
            vec!["Id(BIGINT)"]
        );
        // Positions listed out of order: (2) blocks the queue, so (0) is never
        // reached either, and the simple parameters fill the first slots.
        assert_eq!(
            merged(&[TypeId::BigInt], &[(2, "two"), (0, "zero")]),
            vec!["Id(BIGINT)", "Logical(two)"]
        );
    }

    #[test]
    fn a_null_child_handle_is_held_but_never_destroyed() {
        // SAFETY: null is allowed; dropping it must not reach
        // `duckdb_destroy_logical_type`, which would panic without a live
        // DuckDB here.
        let child = unsafe { Owned::new(std::ptr::null_mut()) };
        assert!(child.raw.is_null());
        drop(child);
    }

    #[test]
    fn merged_params_keeps_simple_params_in_order() {
        let merged = merged_params::<LogicalType>(&[TypeId::BigInt, TypeId::Varchar], &[]);
        let ids: Vec<_> = merged
            .iter()
            .map(|p| match p {
                ParamRef::Id(id) => Some(*id),
                ParamRef::Logical(_) | ParamRef::Varargs(_) | ParamRef::VarargsId(_) => None,
            })
            .collect();
        assert_eq!(ids, vec![Some(TypeId::BigInt), Some(TypeId::Varchar)]);
    }

    /// A composite varargs id is refused by `check`, naming the slot and the
    /// type, so `register` reports it instead of `set_on` hitting the
    /// composite-type panic in `LogicalType::new`. The refusal happens before
    /// any `DuckDB` call, so no live runtime is needed.
    #[test]
    fn a_composite_varargs_id_is_refused_naming_the_slot() {
        for id in [TypeId::List, TypeId::Struct, TypeId::Decimal] {
            let err = Varargs::Id(id)
                .check(LogicalType::check_slot, "scalar function varargs")
                .expect_err("a composite id cannot be built from the id alone");
            let msg = err.as_str();
            assert!(msg.starts_with("scalar function varargs: "), "{msg}");
            assert!(msg.contains(id.sql_name()), "{msg}");
            // Points at the `*_logical` setter that does accept the type.
            assert!(msg.contains("_logical"), "{msg}");
        }
    }
}
