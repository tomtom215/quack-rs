// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Scalar functions written as ordinary Rust closures.
//!
//! [`ScalarFunctionBuilder::function`][crate::scalar::ScalarFunctionBuilder::function]
//! takes an `unsafe extern "C" fn`, which means every scalar function — the most
//! common kind of extension function by far — starts with raw pointers, manual
//! offset arithmetic, and the NULL-propagation contract described in
//! [`NullHandling`]. The constructors here take a
//! safe closure instead:
//!
//! ```rust,no_run
//! use quack_rs::scalar::ScalarFunctionBuilder;
//!
//! # fn demo(con: libduckdb_sys::duckdb_connection)
//! # -> Result<(), quack_rs::error::ExtensionError> {
//! // SAFETY: `con` is the connection DuckDB handed the entry point.
//! unsafe {
//!     ScalarFunctionBuilder::map1("double_it", |x: i64| x * 2)?.register(con)?;
//!     ScalarFunctionBuilder::map2("add", |a: i64, b: i64| a + b)?.register(con)?;
//!     ScalarFunctionBuilder::map1_str("shout", |s: &str| s.to_uppercase())?
//!         .register(con)?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # What they take care of
//!
//! - **Types.** Parameter and return types are derived from the closure's
//!   signature, so a mismatch is a compile error rather than a registration
//!   failure or a misread vector.
//! - **NULLs.** `map1` / `map2` / `map1_str` / `map2_str` implement SQL's
//!   NULL-in-NULL-out: a row with any NULL argument is skipped and its result
//!   set NULL. `DuckDB` does *not* do this for you — see
//!   [`DataChunk::propagate_nulls`][crate::data_chunk::DataChunk::propagate_nulls].
//!   The `*_opt` variants hand the closure `Option`s instead and register
//!   [`SpecialNullHandling`][crate::types::NullHandling::SpecialNullHandling].
//! - **Panics and errors.** The closure runs inside `catch_unwind`, and a panic
//!   or an `Err` becomes a `DuckDB` error on that query rather than a process
//!   abort or a wrong answer.
//! - **A fixed signature.** They return a
//!   [`TypedScalarFunctionBuilder`], not a [`ScalarFunctionBuilder`]: it has no
//!   `returns` / `param` / `function` / `extra_info`, because redeclaring the
//!   signature after the fact would make the closure read and write vectors of
//!   the wrong width. Register it with
//!   [`TypedScalarFunctionBuilder::register`] or
//!   [`Registrar::register_typed_scalar`][crate::connection::Registrar::register_typed_scalar].
//!
//! # Cost
//!
//! One indirect call per *chunk*, not per row: the closure is monomorphised into
//! a per-chunk executor when the function is built, and only that executor is
//! reached through `dyn`. The inner row loop is the same code the hand-written
//! callback would have been.
//!
//! # When to drop down
//!
//! Reach for [`scalar_callback!`][crate::scalar_callback] and the raw builder
//! when the function needs `STRUCT` / `LIST` / `MAP` arguments, variable arity,
//! bind-time constant folding, or per-thread local state.

use crate::error::ExtensionError;
use crate::scalar::builder::ScalarFunctionBuilder;
use crate::scalar::typed_builder::TypedScalarFunctionBuilder;
use crate::types::{NullHandling, TypeId};
use crate::vector::{VectorReader, VectorWriter};

/// A Rust type that maps 1:1 onto a `DuckDB` scalar column type.
///
/// Implemented for `bool`, every signed and unsigned integer width `DuckDB` has
/// (`i8`..`i128`, `u8`..`u128`), and `f32` / `f64`. These are the types the
/// closure-based constructors accept as arguments.
///
/// # Safety
///
/// [`read`][Self::read] performs an unchecked typed read of a vector's data
/// buffer. An implementation must return the type that
/// [`type_id`][Self::type_id] names, or reads will silently reinterpret memory.
/// This is why the trait is `unsafe` to implement; using it is entirely safe.
pub unsafe trait ScalarValue: Copy + Send + Sync + 'static {
    /// The `DuckDB` column type this maps to.
    fn type_id() -> TypeId;

    /// Reads row `row` of `reader` as `Self`.
    ///
    /// # Safety
    ///
    /// `reader` must cover a vector whose type is [`type_id`][Self::type_id],
    /// and `row` must be within its row count.
    unsafe fn read(reader: &VectorReader, row: usize) -> Self;
}

/// A Rust type a scalar closure can return.
///
/// Implemented for every [`ScalarValue`], plus `String` and `Vec<u8>` for
/// `VARCHAR` and `BLOB` results.
///
/// # Safety
///
/// [`write`][Self::write] performs an unchecked typed write into a vector's
/// data buffer; see [`ScalarValue`].
pub unsafe trait ScalarOut: Send + Sync + 'static {
    /// The `DuckDB` column type this maps to.
    fn type_id() -> TypeId;

    /// Writes `value` into row `row` of `writer`.
    ///
    /// # Safety
    ///
    /// `writer` must cover a vector whose type is [`type_id`][Self::type_id],
    /// and `row` must be within its capacity.
    unsafe fn write(writer: &mut VectorWriter, row: usize, value: Self);
}

macro_rules! impl_scalar_value {
    ($($rust:ty => $variant:ident, $read:ident, $write:ident;)*) => {
        $(
            // SAFETY: `type_id` names the DuckDB type whose physical layout
            // `VectorReader::$read` and `VectorWriter::$write` are documented to
            // read and write.
            unsafe impl ScalarValue for $rust {
                #[inline]
                fn type_id() -> TypeId { TypeId::$variant }
                #[inline]
                unsafe fn read(reader: &VectorReader, row: usize) -> Self {
                    // SAFETY: forwarded from this method's own contract.
                    unsafe { reader.$read(row) }
                }
            }
            // SAFETY: as above.
            unsafe impl ScalarOut for $rust {
                #[inline]
                fn type_id() -> TypeId { TypeId::$variant }
                #[inline]
                unsafe fn write(writer: &mut VectorWriter, row: usize, value: Self) {
                    // SAFETY: forwarded from this method's own contract.
                    unsafe { writer.$write(row, value) };
                }
            }
        )*
    };
}

impl_scalar_value! {
    bool => Boolean,   read_bool, write_bool;
    i8   => TinyInt,   read_i8,   write_i8;
    i16  => SmallInt,  read_i16,  write_i16;
    i32  => Integer,   read_i32,  write_i32;
    i64  => BigInt,    read_i64,  write_i64;
    i128 => HugeInt,   read_i128, write_i128;
    u8   => UTinyInt,  read_u8,   write_u8;
    u16  => USmallInt, read_u16,  write_u16;
    u32  => UInteger,  read_u32,  write_u32;
    u64  => UBigInt,   read_u64,  write_u64;
    u128 => UHugeInt,  read_u128, write_u128;
    f32  => Float,     read_f32,  write_f32;
    f64  => Double,    read_f64,  write_f64;
}

// SAFETY: `VectorWriter::write_varchar` writes a VARCHAR `duckdb_string_t`.
unsafe impl ScalarOut for String {
    #[inline]
    fn type_id() -> TypeId {
        TypeId::Varchar
    }
    #[inline]
    unsafe fn write(writer: &mut VectorWriter, row: usize, value: Self) {
        // SAFETY: forwarded from this method's own contract.
        unsafe { writer.write_varchar(row, &value) };
    }
}

// SAFETY: `VectorWriter::write_blob` writes a BLOB `duckdb_string_t`.
unsafe impl ScalarOut for Vec<u8> {
    #[inline]
    fn type_id() -> TypeId {
        TypeId::Blob
    }
    #[inline]
    unsafe fn write(writer: &mut VectorWriter, row: usize, value: Self) {
        // SAFETY: forwarded from this method's own contract.
        unsafe { writer.write_blob(row, &value) };
    }
}

impl ScalarFunctionBuilder {
    /// A unary scalar function from a safe closure, with SQL NULL propagation.
    ///
    /// Parameter and return types come from the closure's signature. A row whose
    /// argument is NULL yields NULL without calling the closure.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::scalar::ScalarFunctionBuilder;
    ///
    /// # fn demo(con: libduckdb_sys::duckdb_connection)
    /// # -> Result<(), quack_rs::error::ExtensionError> {
    /// // SAFETY: `con` is the connection DuckDB handed the entry point.
    /// unsafe { ScalarFunctionBuilder::map1("double_it", |x: i64| x * 2)?.register(con) }
    /// # }
    /// ```
    pub fn map1<A, R, F>(name: &str, f: F) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        R: ScalarOut,
        F: Fn(A) -> R + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[A::type_id()],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: DuckDB declared one parameter, so column 0 exists and
                // has the type `A::type_id()` named at registration.
                let a = unsafe { chunk.reader(0) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) } {
                        // SAFETY: the column's type was declared as `A`.
                        let value = f(unsafe { A::read(&a, row) });
                        // SAFETY: the result vector's type was declared as `R`.
                        unsafe { R::write(writer, row, value) };
                    } else {
                        // SAFETY: `row` is within the output vector.
                        unsafe { writer.set_null(row) };
                    }
                }
                Ok(())
            }),
        )
    }

    /// A binary scalar function from a safe closure, with SQL NULL propagation.
    ///
    /// A row with either argument NULL yields NULL without calling the closure.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    // The per-row NULL-propagation test `a.is_valid(row) && b.is_valid(row)`
    // lives in a closure that only ever runs inside DuckDB's expression
    // executor, so a `--lib` run cannot reach it and the `&& -> ||` mutant
    // survives there. It does not survive the end-to-end suite:
    // `typed_scalar_closures_cover_the_common_shapes` in
    // `tests/ffi_roundtrip.rs` asserts that map2 NULLs out a row where
    // *either* argument is NULL, reading from a real column rather than a
    // constant-folded literal.
    #[mutants::skip]
    pub fn map2<A, B, R, F>(name: &str, f: F) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        B: ScalarValue,
        R: ScalarOut,
        F: Fn(A, B) -> R + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[A::type_id(), B::type_id()],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: two parameters were declared, so both columns exist
                // with the types named at registration.
                let (a, b) = unsafe { (chunk.reader(0), chunk.reader(1)) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) && b.is_valid(row) } {
                        // SAFETY: the columns' types were declared as `A` and `B`.
                        let value = unsafe { f(A::read(&a, row), B::read(&b, row)) };
                        // SAFETY: the result vector's type was declared as `R`.
                        unsafe { R::write(writer, row, value) };
                    } else {
                        // SAFETY: `row` is within the output vector.
                        unsafe { writer.set_null(row) };
                    }
                }
                Ok(())
            }),
        )
    }

    /// A unary scalar function that **sees** NULLs.
    ///
    /// Registers
    /// [`SpecialNullHandling`][crate::types::NullHandling::SpecialNullHandling],
    /// so `DuckDB` neither expects nor asserts NULL-in-NULL-out. The closure
    /// receives `None` for a NULL argument and returning `None` writes NULL —
    /// which is what `coalesce`-like and `is_null`-like functions need.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    pub fn map1_opt<A, R, F>(name: &str, f: F) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        R: ScalarOut,
        F: Fn(Option<A>) -> Option<R> + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[A::type_id()],
            R::type_id(),
            NullHandling::SpecialNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: one parameter was declared, so column 0 exists.
                let a = unsafe { chunk.reader(0) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk; the column's type was
                    // declared as `A`.
                    let arg = unsafe { a.is_valid(row).then(|| A::read(&a, row)) };
                    match f(arg) {
                        // SAFETY: the result vector's type was declared as `R`.
                        Some(value) => unsafe { R::write(writer, row, value) },
                        // SAFETY: `row` is within the output vector.
                        None => unsafe { writer.set_null(row) },
                    }
                }
                Ok(())
            }),
        )
    }

    /// A binary scalar function that **sees** NULLs.
    ///
    /// See [`map1_opt`][Self::map1_opt].
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    pub fn map2_opt<A, B, R, F>(
        name: &str,
        f: F,
    ) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        B: ScalarValue,
        R: ScalarOut,
        F: Fn(Option<A>, Option<B>) -> Option<R> + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[A::type_id(), B::type_id()],
            R::type_id(),
            NullHandling::SpecialNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: two parameters were declared, so both columns exist.
                let (left, right) = unsafe { (chunk.reader(0), chunk.reader(1)) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk; the columns' types were
                    // declared as `A` and `B`.
                    let args = unsafe {
                        (
                            left.is_valid(row).then(|| A::read(&left, row)),
                            right.is_valid(row).then(|| B::read(&right, row)),
                        )
                    };
                    match f(args.0, args.1) {
                        // SAFETY: the result vector's type was declared as `R`.
                        Some(value) => unsafe { R::write(writer, row, value) },
                        // SAFETY: `row` is within the output vector.
                        None => unsafe { writer.set_null(row) },
                    }
                }
                Ok(())
            }),
        )
    }

    /// A `VARCHAR`-argument scalar function from a safe closure, with SQL NULL
    /// propagation.
    ///
    /// The closure borrows the string straight out of the vector — no allocation
    /// per row. `VARCHAR` needs its own constructor because the argument is a
    /// borrow with the chunk's lifetime, which a plain type parameter cannot
    /// express.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::scalar::ScalarFunctionBuilder;
    ///
    /// # fn demo(con: libduckdb_sys::duckdb_connection)
    /// # -> Result<(), quack_rs::error::ExtensionError> {
    /// // SAFETY: `con` is the connection DuckDB handed the entry point.
    /// unsafe {
    ///     ScalarFunctionBuilder::map1_str("word_count", |s: &str| {
    ///         s.split_whitespace().count() as i64
    ///     })?
    ///     .register(con)
    /// }
    /// # }
    /// ```
    pub fn map1_str<R, F>(name: &str, f: F) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        R: ScalarOut,
        F: for<'a> Fn(&'a str) -> R + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[TypeId::Varchar],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: one VARCHAR parameter was declared, so column 0 exists
                // and holds VARCHAR.
                let a = unsafe { chunk.reader(0) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) } {
                        // SAFETY: the column was declared VARCHAR; the borrow
                        // ends before the chunk does.
                        let value = f(unsafe { a.read_str(row) });
                        // SAFETY: the result vector's type was declared as `R`.
                        unsafe { R::write(writer, row, value) };
                    } else {
                        // SAFETY: `row` is within the output vector.
                        unsafe { writer.set_null(row) };
                    }
                }
                Ok(())
            }),
        )
    }

    /// A two-`VARCHAR` scalar function from a safe closure, with SQL NULL
    /// propagation.
    ///
    /// See [`map1_str`][Self::map1_str].
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    // The per-row NULL-propagation test `a.is_valid(row) && b.is_valid(row)`
    // lives in a closure that only ever runs inside DuckDB's expression
    // executor, so a `--lib` run cannot reach it and the `&& -> ||` mutant
    // survives there. It does not survive the end-to-end suite:
    // `typed_scalar_closures_cover_the_common_shapes` in
    // `tests/ffi_roundtrip.rs` asserts that map2_str NULLs out a row where
    // *either* argument is NULL, reading from a real column rather than a
    // constant-folded literal.
    #[mutants::skip]
    pub fn map2_str<R, F>(name: &str, f: F) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        R: ScalarOut,
        F: for<'a, 'b> Fn(&'a str, &'b str) -> R + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[TypeId::Varchar, TypeId::Varchar],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: two VARCHAR parameters were declared.
                let (a, b) = unsafe { (chunk.reader(0), chunk.reader(1)) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) && b.is_valid(row) } {
                        // SAFETY: both columns were declared VARCHAR.
                        let value = unsafe { f(a.read_str(row), b.read_str(row)) };
                        // SAFETY: the result vector's type was declared as `R`.
                        unsafe { R::write(writer, row, value) };
                    } else {
                        // SAFETY: `row` is within the output vector.
                        unsafe { writer.set_null(row) };
                    }
                }
                Ok(())
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_value_type_ids_match_their_rust_widths() {
        assert_eq!(<i8 as ScalarValue>::type_id(), TypeId::TinyInt);
        assert_eq!(<u64 as ScalarValue>::type_id(), TypeId::UBigInt);
        assert_eq!(<f32 as ScalarValue>::type_id(), TypeId::Float);
        assert_eq!(<i128 as ScalarValue>::type_id(), TypeId::HugeInt);
        assert_eq!(<bool as ScalarValue>::type_id(), TypeId::Boolean);
        assert_eq!(<String as ScalarOut>::type_id(), TypeId::Varchar);
        assert_eq!(<Vec<u8> as ScalarOut>::type_id(), TypeId::Blob);
    }

    /// Building a typed function touches no `DuckDB` API: the only `DuckDB`
    /// work is in `register`. Before the third audit `from_exec` built a
    /// `LogicalType` per slot to check it, so every constructor panicked in a
    /// unit test (no dispatch table) and built handles nobody needed.
    #[test]
    fn every_typed_constructor_builds_without_a_live_duckdb() {
        let built = [
            ScalarFunctionBuilder::map1("m1", |x: i64| x * 2),
            ScalarFunctionBuilder::map2("m2", |a: i32, b: f64| f64::from(a) + b),
            ScalarFunctionBuilder::map1_opt("m1o", |x: Option<u8>| x.map(u16::from)),
            ScalarFunctionBuilder::map2_opt("m2o", |a: Option<i128>, b: Option<bool>| {
                a.filter(|_| b.unwrap_or(false))
            }),
            ScalarFunctionBuilder::map1_str("m1s", |s: &str| s.to_uppercase()),
            ScalarFunctionBuilder::map2_str("m2s", |a: &str, b: &str| [a, b].concat().into_bytes()),
        ];
        let names: Vec<_> = built
            .into_iter()
            .map(|b| {
                b.expect("builds with no DuckDB")
                    .volatile()
                    .name()
                    .to_owned()
            })
            .collect();
        assert_eq!(names, ["m1", "m2", "m1o", "m2o", "m1s", "m2s"]);
    }

    #[test]
    fn an_invalid_name_is_rejected_before_anything_is_allocated() {
        let err = ScalarFunctionBuilder::map1("has spaces", |x: i64| x)
            .expect_err("a name with a space is not a SQL identifier");
        assert_ne!(err.as_str(), "");
    }
}
