// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`DecimalWriter`]: `DECIMAL` output whose width and storage come from the
//! vector's own type.

use crate::error::ExtensionError;
use crate::types::TypeId;
use crate::value::validate_unscaled_decimal;
use crate::vector::VectorWriter;

/// Writes `DECIMAL` values into a vector, checked against the vector's own
/// type.
///
/// Made by [`VectorWriter::decimal`], which reads the vector's logical type
/// once and refuses anything but a `DECIMAL`. [`write`][Self::write] then
/// refuses an unscaled value with more digits than the type's width allows,
/// and stores the rest in the integer `DuckDB` uses for that width (`i16` up to
/// 4 digits, `i32` up to 9, `i64` up to 18, `i128` beyond). Neither the width
/// nor the storage is the caller's to get right, unlike
/// [`VectorWriter::write_decimal`], whose width argument also picks how many
/// bytes it writes.
///
/// The scale is fixed by the type too: [`scale`][Self::scale] says what it is,
/// and a result computed at another scale must be rescaled before it is
/// written.
///
/// ```rust,no_run
/// use quack_rs::vector::VectorWriter;
/// use libduckdb_sys::duckdb_vector;
///
/// /// Writes `cents` into a `DECIMAL(_, 2)` output vector, or NULL where the
/// /// value does not fit the column.
/// unsafe fn write_cents(output: duckdb_vector, cents: &[i128]) -> Result<(), String> {
///     let mut writer = unsafe { VectorWriter::from_vector(output) };
///     let mut decimal = writer.decimal().map_err(|e| e.to_string())?;
///     if decimal.scale() != 2 {
///         return Err(format!("expected scale 2, got {}", decimal.scale()));
///     }
///     for (row, &value) in cents.iter().enumerate() {
///         // SAFETY: `row` is below the output's row count.
///         if unsafe { decimal.write(row, value) }.is_err() {
///             unsafe { decimal.set_null(row) };
///         }
///     }
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct DecimalWriter<'w> {
    writer: &'w mut VectorWriter,
    width: u8,
    scale: u8,
}

impl VectorWriter {
    /// A [`DecimalWriter`] for this vector, whose width and scale are read from
    /// the vector's logical type.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector's type is not `DECIMAL`, or if `DuckDB`
    /// reports a width outside `1..=38`.
    pub fn decimal(&mut self) -> Result<DecimalWriter<'_>, ExtensionError> {
        // SAFETY: `as_raw` is the valid vector this writer's constructor was
        // given (its contract).
        let ty = unsafe { crate::vector::vector_get_column_type(self.as_raw()) };
        // SAFETY: `ty` is a live logical type this function owns.
        let id = unsafe { ty.get_type_id() };
        if id != TypeId::Decimal {
            return Err(ExtensionError::new(format!(
                "VectorWriter::decimal: the vector's type is {}, not DECIMAL",
                id.sql_name()
            )));
        }
        // SAFETY: as above; the type is a DECIMAL.
        let (width, scale) = unsafe { (ty.decimal_width(), ty.decimal_scale()) };
        if !(1..=38).contains(&width) {
            return Err(ExtensionError::new(format!(
                "VectorWriter::decimal: DuckDB reports a DECIMAL width of {width}, outside 1..=38"
            )));
        }
        Ok(DecimalWriter {
            writer: self,
            width,
            scale,
        })
    }
}

impl DecimalWriter<'_> {
    /// The vector's `DECIMAL` width: how many digits it holds.
    #[must_use]
    pub const fn width(&self) -> u8 {
        self.width
    }

    /// The vector's `DECIMAL` scale: how many of the digits follow the point.
    #[must_use]
    pub const fn scale(&self) -> u8 {
        self.scale
    }

    /// Writes the unscaled integer `unscaled` (the value times
    /// 10<sup>scale</sup>) at row `idx`.
    ///
    /// On `Err` nothing is written and row `idx` keeps whatever it held: report
    /// the error, or write a NULL with [`set_null`][Self::set_null], as the
    /// function's contract says.
    ///
    /// # Errors
    ///
    /// Returns an error if `|unscaled|` is `10^width` or more.
    ///
    /// # Safety
    ///
    /// `idx` must be within the vector's capacity.
    pub unsafe fn write(&mut self, idx: usize, unscaled: i128) -> Result<(), ExtensionError> {
        validate_unscaled_decimal(self.width, unscaled)?;
        // SAFETY: `idx` is within capacity (this function's contract); the
        // vector is a DECIMAL of exactly `self.width` (read from its type in
        // `VectorWriter::decimal`); `unscaled` fits that width (checked above).
        unsafe { self.writer.write_decimal(idx, self.width, unscaled) };
        Ok(())
    }

    /// Marks row `idx` as NULL, as [`VectorWriter::set_null`] does.
    ///
    /// # Safety
    ///
    /// `idx` must be within the vector's capacity.
    pub unsafe fn set_null(&mut self, idx: usize) {
        // SAFETY: forwarded; the same contract.
        unsafe { self.writer.set_null(idx) };
    }
}
