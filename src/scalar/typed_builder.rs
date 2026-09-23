// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The builder the closure-based scalar constructors return, and the one
//! `extern "C"` trampoline they all share.
//!
//! # Why a separate builder type
//!
//! A typed closure's trampoline reads each argument column as the Rust type the
//! closure takes and writes the result as the Rust type it returns. Those reads
//! and writes are only sound while the *declared* SQL signature matches. The
//! plain [`ScalarFunctionBuilder`] lets safe code redeclare the signature at any
//! time — `.returns(TypeId::Integer)` after an `i64` closure made `DuckDB`
//! allocate a 4-byte-per-row result vector the trampoline then wrote 8 bytes
//! per row into. [`TypedScalarFunctionBuilder`] therefore exposes only
//! operations that cannot desynchronise the two.
//!
//! As a second line of defence, the trampoline checks every chunk's column
//! count and physical types against the closure's signature before touching a
//! single row, and reports a mismatch as a SQL error.

use std::panic::AssertUnwindSafe;

use libduckdb_sys::{
    duckdb_connection, duckdb_data_chunk, duckdb_function_info, duckdb_get_type_id, duckdb_vector,
};

use crate::data_chunk::DataChunk;
use crate::error::ExtensionError;
use crate::scalar::builder::ScalarFunctionBuilder;
use crate::types::{LogicalType, NullHandling, TypeId};
use crate::vector::{vector_get_column_type, VectorWriter};

/// The per-chunk executor a typed closure is compiled into.
///
/// Boxed once at build time and reached through one indirect call per chunk;
/// the row loop inside is monomorphic.
pub(super) type ChunkExec =
    Box<dyn Fn(&DataChunk, &mut VectorWriter) -> Result<(), ExtensionError> + Send + Sync>;

/// `extra_info` payload for a typed scalar function.
struct TypedScalar {
    params: Vec<TypeId>,
    ret: TypeId,
    exec: ChunkExec,
}

impl TypedScalar {
    /// `extra_info` destructor.
    ///
    /// # Safety
    ///
    /// `ptr` must have come from `Box::into_raw` on a `Box<TypedScalar>`.
    unsafe extern "C" fn destroy(ptr: *mut std::os::raw::c_void) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: `ptr` came from `Box::into_raw` in `from_exec`. The boxed
        // closure captures user data whose `Drop` may panic, and this is an
        // `extern "C"` boundary with no error channel, so contain the unwind.
        drop(crate::callback::catch_ffi_panic(|| unsafe {
            drop(Box::from_raw(ptr.cast::<Self>()));
        }));
    }

    /// Checks that the vectors `DuckDB` handed over have the types the closure
    /// was compiled for.
    ///
    /// # Safety
    ///
    /// `output` must be the valid result vector for `chunk`'s invocation.
    // A mismatch cannot be produced through this crate's public API any more;
    // the end-to-end suite reaches it through a hand-written `Registrar`.
    #[mutants::skip]
    unsafe fn check_signature(
        &self,
        chunk: &DataChunk,
        output: duckdb_vector,
    ) -> Result<(), ExtensionError> {
        if chunk.column_count() != self.params.len() {
            return Err(ExtensionError::new(format!(
                "quack-rs: typed scalar function was declared with {} argument(s) but was \
                 called with {}; its signature must not be changed after construction",
                self.params.len(),
                chunk.column_count()
            )));
        }
        for (i, expected) in self.params.iter().enumerate() {
            // SAFETY: `i` is below the column count checked above.
            let vector = unsafe { chunk.vector(i) };
            // SAFETY: `vector` is a valid column of this chunk.
            unsafe { check_vector(vector, *expected, &format!("argument {i}"))? };
        }
        // SAFETY: `output` is valid per this function's contract.
        unsafe { check_vector(output, self.ret, "result") }
    }
}

/// Fails unless `vector`'s type id is `expected`.
///
/// # Safety
///
/// `vector` must be a valid `duckdb_vector`.
unsafe fn check_vector(
    vector: duckdb_vector,
    expected: TypeId,
    what: &str,
) -> Result<(), ExtensionError> {
    // SAFETY: `vector` is valid per this function's contract.
    let ty = unsafe { vector_get_column_type(vector) };
    // SAFETY: `ty` is a live logical type owned for this scope.
    let actual = unsafe { duckdb_get_type_id(ty.as_raw()) };
    if actual == expected.to_duckdb_type() {
        return Ok(());
    }
    Err(ExtensionError::new(format!(
        "quack-rs: typed scalar function {what} is {actual_name} but the closure handles \
         {expected_name}; its signature must not be changed after construction",
        actual_name =
            TypeId::try_from_duckdb_type(actual).map_or("an unknown type", TypeId::sql_name),
        expected_name = expected.sql_name(),
    )))
}

/// The single `extern "C"` callback every typed scalar function shares.
///
/// # Safety
///
/// Invoked by `DuckDB` with its own valid handles.
unsafe extern "C" fn typed_trampoline(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    // SAFETY: `info` is the handle DuckDB passed in.
    let fninfo = unsafe { crate::scalar::ScalarFunctionInfo::new(info) };

    let outcome = crate::callback::catch_ffi_panic(AssertUnwindSafe(|| {
        // SAFETY: `extra_info` was set by `from_exec` to a `Box<TypedScalar>`
        // that DuckDB keeps alive until it calls `TypedScalar::destroy`.
        // `TypedScalarFunctionBuilder` offers no way to replace it.
        let raw = unsafe { fninfo.get_extra_info() };
        if raw.is_null() {
            return Err(ExtensionError::new(
                "quack-rs: typed scalar function lost its extra_info",
            ));
        }
        // SAFETY: same provenance as above; shared access only.
        let typed = unsafe { &*raw.cast::<TypedScalar>() };
        // SAFETY: `input` and `output` are valid for this call.
        let chunk = unsafe { DataChunk::from_raw(input) };
        // SAFETY: `output` is this invocation's result vector.
        unsafe { typed.check_signature(&chunk, output)? };
        // SAFETY: as above.
        let mut writer = unsafe { VectorWriter::from_vector(output) };
        (typed.exec)(&chunk, &mut writer)
    }));

    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => fninfo.set_error(e.as_str()),
        Err(message) => {
            fninfo.set_error(&format!("quack-rs: scalar closure panicked: {message}"));
        }
    }
}

/// A scalar function built from a safe Rust closure, ready to register.
///
/// Returned by [`ScalarFunctionBuilder::map1`] and its siblings. The SQL
/// signature is fixed by the closure's type, so this builder deliberately has
/// no `returns`, `param`, `function`, `extra_info`, `bind` or `init`: each of
/// those could make `DuckDB` hand the closure vectors of a different type than
/// it reads and writes.
///
/// ```rust,compile_fail
/// use quack_rs::scalar::ScalarFunctionBuilder;
/// use quack_rs::types::TypeId;
///
/// # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
/// // An i64 result cannot be redeclared as a 4-byte INTEGER.
/// let _ = ScalarFunctionBuilder::map1("widen", |x: i64| x + 1)?.returns(TypeId::Integer);
/// # Ok(())
/// # }
/// ```
///
/// ```rust,compile_fail
/// use quack_rs::scalar::ScalarFunctionBuilder;
///
/// quack_rs::scalar_callback!(other, |_info, _input, _output| {});
///
/// # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
/// // The trampoline cannot be swapped out from under the closure's state.
/// let _ = ScalarFunctionBuilder::map1("f", |x: i64| x)?.function(other);
/// # Ok(())
/// # }
/// ```
#[must_use]
pub struct TypedScalarFunctionBuilder {
    inner: ScalarFunctionBuilder,
}

impl TypedScalarFunctionBuilder {
    /// Builds a typed scalar function from a per-chunk executor.
    ///
    /// The public `map*` constructors on [`ScalarFunctionBuilder`] are thin
    /// wrappers over this.
    pub(super) fn from_exec(
        name: &str,
        params: &[TypeId],
        ret: TypeId,
        null_handling: NullHandling,
        exec: ChunkExec,
    ) -> Result<Self, ExtensionError> {
        let mut builder = ScalarFunctionBuilder::try_new(name)?;
        for (i, id) in params.iter().enumerate() {
            LogicalType::check_slot(*id, &format!("scalar function parameter {i}"))?;
            builder = builder.param(*id);
        }
        LogicalType::check_slot(ret, "scalar function return type")?;
        builder = builder.returns(ret).null_handling(null_handling);

        let typed = TypedScalar {
            params: params.to_vec(),
            ret,
            exec,
        };
        let raw = Box::into_raw(Box::new(typed)).cast::<std::os::raw::c_void>();
        // SAFETY: `raw` is a live `Box<TypedScalar>` and `TypedScalar::destroy`
        // is the matching destructor; DuckDB owns it from here.
        let inner = unsafe {
            builder
                .function(typed_trampoline)
                .extra_info(raw, Some(TypedScalar::destroy))
        };
        Ok(Self { inner })
    }

    /// Returns the function name.
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Marks this function as volatile: re-evaluated for every row even when
    /// its arguments are constant (e.g. a random-number function).
    ///
    /// Only the function's stability changes; its signature does not.
    pub fn volatile(mut self) -> Self {
        self.inner = self.inner.volatile();
        self
    }

    /// Registers the function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if `DuckDB` reports a registration failure
    /// (for example, a function with this name and signature already exists).
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // SAFETY: forwarded from this function's own contract.
        unsafe { self.inner.register(con) }
    }

    /// Unwraps the underlying builder for a [`Registrar`][crate::connection::Registrar]
    /// that only knows [`ScalarFunctionBuilder`].
    ///
    /// Crate-private so the signature cannot be edited through it; the
    /// trampoline's per-chunk type check still guards a `Registrar`
    /// implementation that edits it anyway.
    pub(crate) fn into_inner(self) -> ScalarFunctionBuilder {
        self.inner
    }
}

impl core::fmt::Debug for TypedScalarFunctionBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TypedScalarFunctionBuilder")
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destroy_tolerates_a_null_pointer() {
        // SAFETY: the null case is explicitly handled.
        unsafe { TypedScalar::destroy(std::ptr::null_mut()) };
    }

    #[test]
    fn name_is_the_one_the_builder_was_created_with() {
        // Built directly: `from_exec` checks each slot by creating a
        // `LogicalType`, which needs a live DuckDB.
        let builder = TypedScalarFunctionBuilder {
            inner: ScalarFunctionBuilder::new("double_it"),
        };
        assert_eq!(builder.name(), "double_it");
        // A stability change keeps the name.
        assert_eq!(builder.volatile().name(), "double_it");
    }
}
