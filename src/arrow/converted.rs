// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ArrowConvertedSchema` — an Arrow schema translated into `DuckDB`'s own type descriptors.

use libduckdb_sys::{duckdb_arrow_converted_schema, duckdb_destroy_arrow_converted_schema};

use super::import_layout::Shape;
use super::{ArrowConvertedSchema, ArrowSchema};

impl ArrowConvertedSchema {
    /// Takes ownership of a raw converted schema, recording the column count
    /// and the shape of `schema`, the Arrow schema it was built from.
    ///
    /// # Safety
    ///
    /// - `raw` must be a non-null handle the caller is responsible for
    ///   destroying, and nobody else may destroy it.
    /// - `raw` must have been built from `schema` (by
    ///   `duckdb_schema_from_arrow`). A different schema defeats the bounds and
    ///   layout checks in [`data_chunk_from_arrow`][super::data_chunk_from_arrow].
    #[must_use]
    pub unsafe fn from_raw(raw: duckdb_arrow_converted_schema, schema: &ArrowSchema) -> Self {
        let column_count = schema.child_count();
        let shapes = (0..column_count)
            .filter_map(|i| schema.child(i).map(Shape::of))
            .collect();
        Self {
            raw,
            column_count,
            shapes,
        }
    }

    /// The shape of each column's Arrow schema.
    pub(super) fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// The raw handle, still owned by this value.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_arrow_converted_schema {
        self.raw
    }

    /// How many columns this schema describes.
    #[inline]
    #[must_use]
    pub const fn column_count(&self) -> usize {
        self.column_count
    }
}

impl Drop for ArrowConvertedSchema {
    #[mutants::skip] // frees a DuckDB handle; nothing observable without a runtime
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }
        // SAFETY: `self.raw` was owned by this value and is destroyed once;
        // DuckDB nulls it.
        unsafe { duckdb_destroy_arrow_converted_schema(&raw mut self.raw) };
    }
}

impl core::fmt::Debug for ArrowConvertedSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArrowConvertedSchema")
            .field("raw", &self.raw)
            .field("column_count", &self.column_count)
            .field("shapes", &self.shapes)
            .finish()
    }
}
