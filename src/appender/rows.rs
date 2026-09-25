// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Row-at-a-time appends: `row`, `end_row` and the non-numeric `append_*` methods.

use libduckdb_sys::{
    duckdb_append_blob, duckdb_append_date, duckdb_append_default, duckdb_append_interval,
    duckdb_append_null, duckdb_append_time, duckdb_append_timestamp, duckdb_append_value,
    duckdb_append_varchar_length, duckdb_appender_end_row, duckdb_date, duckdb_interval,
    duckdb_time, duckdb_timestamp, idx_t,
};

use super::{append_error, AppendError, Appender, Lifecycle};
use crate::interval::DuckInterval;
use crate::value::Value;

/// `duckdb_append_varchar_length` narrows its length argument to `uint32_t`
/// with `UnsafeNumericCast`, which is a plain `static_cast` in the release
/// builds `DuckDB` ships. A longer string would be silently truncated to its
/// low 32 bits, so it is refused here instead.
const MAX_VARCHAR_LEN: usize = u32::MAX as usize;

/// Whether `len` bytes fit `duckdb_append_varchar_length`'s `uint32_t`.
const fn fits_append_length(len: usize) -> bool {
    len <= MAX_VARCHAR_LEN
}

impl Appender {
    // ── Row-at-a-time appends ───────────────────────────────────────────

    /// Appends one row, calling [`end_row`][Self::end_row] afterwards.
    ///
    /// The closure appends one value per active column. `end_row` runs only if
    /// the closure succeeded.
    ///
    /// If the closure (or `end_row`, for a missing column) fails after the
    /// row's first value went in, `DuckDB` is left holding a half-written row
    /// it can neither finish nor drop, and every row buffered since the last
    /// flush is lost. The appender is then *poisoned*: see the
    /// [module docs][crate::appender]. A closure that panics after the row's
    /// first value poisons it too. A failure of the automatic flush inside
    /// `end_row` does not poison it (see [`end_row`][Self::end_row]).
    ///
    /// # Errors
    ///
    /// Returns whatever the closure returned, or the [`AppendError`] from
    /// `end_row` — most often "call to `EndRow` before all columns have been
    /// appended to". Also an error, before the closure runs, when the appender
    /// is closed or poisoned, or when a row appended by hand is still open.
    pub fn row<F>(&self, append: F) -> Result<(), AppendError>
    where
        F: FnOnce(&Self) -> Result<(), AppendError>,
    {
        self.usable()?;
        if self.column.get() != 0 {
            return Err(append_error(&format!(
                "row: a row appended by hand is still open ({} value(s) without end_row); \
                 finish it before starting another",
                self.column.get()
            )));
        }
        let unwinding = PoisonOnUnwind(self);
        let result = append(self).and_then(|()| self.end_row());
        core::mem::forget(unwinding);
        if result.is_err() && self.column.get() != 0 {
            self.lifecycle.set(Lifecycle::Poisoned);
        }
        result
    }

    /// Finishes the current row.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if fewer values were appended than the
    /// appender has active columns (append the rest and call it again), or if
    /// the appender is closed or poisoned. Also when the automatic flush
    /// `DuckDB` runs every 204,800 rows fails — a constraint violation in any
    /// buffered row. The row has then ended and the appender stays usable, but
    /// the rows that failed stay buffered: the next flush reports the same
    /// error until `clear` (`duckdb-1-5`) discards them.
    pub fn end_row(&self) -> Result<(), AppendError> {
        self.usable()?;
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_appender_end_row(self.handle) };
        let result = self.check(state);
        // `BaseAppender::EndRow` throws before ending the row only when a
        // column is missing; with every column in, it resets its counter and
        // counts the row first, and only the automatic flush that follows
        // (`Flush`, reached through `FlushChunk` once 204,800 rows are
        // buffered: `DEFAULT_FLUSH_COUNT`, `appender.hpp`) can fail. The C API
        // sets no memory threshold, so no other flush runs from here in
        // 1.4.4 through 1.5.5. Such a failure leaves
        // no half-written row, so it must not look like one.
        if result.is_ok() || self.column.get() == self.column_count() {
            self.column.set(0);
            self.buffered.set(self.buffered.get().saturating_add(1));
        }
        result
    }

    /// Appends SQL `NULL` to the current row, whatever the column's type.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_null(&self) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_null(self.handle) })
    }

    /// Appends the column's `DEFAULT` value to the current row.
    ///
    /// A column declared without a `DEFAULT` gets `NULL`, as it would from an
    /// `INSERT` that leaves it out.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the column's `DEFAULT` is not a constant
    /// the appender can evaluate up front — `nextval('seq')`, `random()`,
    /// `now()`: `DuckDB`'s appender evaluates defaults once, when it is
    /// created, and keeps none for an expression that is not foldable
    /// (`Appender::InitializeChunk`, `appender.cpp`), so appending it fails
    /// with "`AppendDefault` is not supported". Also if the append itself fails.
    pub fn append_default(&self) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_default(self.handle) })
    }

    /// Appends a `VARCHAR`.
    ///
    /// Uses `duckdb_append_varchar_length`, so **interior NUL bytes are
    /// preserved** — unlike the NUL-terminated `duckdb_append_varchar`, which
    /// would stop at the first one.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails, or if `value` is longer
    /// than `u32::MAX` bytes — a length `DuckDB` narrows to 32 bits without
    /// checking in its release builds.
    pub fn append_str(&self, value: &str) -> Result<(), AppendError> {
        self.append_bytes_as(value.as_bytes(), true)
    }

    /// Appends a `BLOB`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails, or if `value` is longer
    /// than `u32::MAX` bytes. `duckdb_append_blob` stores the length through a
    /// 32-bit narrowing with no check in release builds: a 4 GiB + 3 byte blob
    /// was stored as 3 bytes, with no error.
    pub fn append_bytes(&self, value: &[u8]) -> Result<(), AppendError> {
        self.append_bytes_as(value, false)
    }

    /// Refuses a 64-bit temporal payload `DuckDB` cannot render, before it is
    /// appended.
    fn check_temporal(
        &self,
        type_id: crate::types::TypeId,
        v: i64,
        what: &str,
    ) -> Result<(), AppendError> {
        self.usable()?;
        if crate::value::temporal_checks::temporal_in_range(type_id, v) {
            Ok(())
        } else {
            Err(append_error(&format!(
                "{what}({v}) is outside DuckDB's range for {}; DuckDB would store it \
                 unchecked and then crash or fail reading it",
                type_id.sql_name()
            )))
        }
    }

    fn append_bytes_as(&self, value: &[u8], varchar: bool) -> Result<(), AppendError> {
        self.usable()?;
        if varchar {
            if !fits_append_length(value.len()) {
                return Err(append_error(&format!(
                    "VARCHAR of {} bytes exceeds DuckDB's {MAX_VARCHAR_LEN}-byte appender limit",
                    value.len()
                )));
            }
            // SAFETY: self.handle is valid; the pointer/length pair describes
            // `value`, which outlives the call.
            let state = unsafe {
                duckdb_append_varchar_length(
                    self.handle,
                    value.as_ptr().cast::<std::os::raw::c_char>(),
                    value.len() as idx_t,
                )
            };
            return self.record_append(state);
        }
        if !fits_append_length(value.len()) {
            return Err(append_error(&format!(
                "BLOB of {} bytes exceeds DuckDB's {MAX_VARCHAR_LEN}-byte appender limit",
                value.len()
            )));
        }
        // SAFETY: as above; DuckDB copies the bytes into a BLOB value.
        let state = unsafe {
            duckdb_append_blob(
                self.handle,
                value.as_ptr().cast::<std::os::raw::c_void>(),
                value.len() as idx_t,
            )
        };
        self.record_append(state)
    }

    /// Appends a `DATE` as days since 1970-01-01.
    ///
    /// Every `i32` is accepted, as by [`Value::date`][crate::value::Value::date];
    /// days outside the range `DuckDB`'s SQL produces render as text it cannot
    /// parse again.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_date(&self, days: i32) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_date(self.handle, duckdb_date { days }) })
    }

    /// Appends a `TIME` as microseconds since midnight.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails, or — before `DuckDB` is
    /// called — if `micros` is outside `0..=86_400_000_000` (`00:00:00` to
    /// `24:00:00`). `duckdb_append_time` checks nothing; appending `i64::MIN`
    /// crashed the process when the row was read, and `i64::MAX` invalidated
    /// the database.
    pub fn append_time(&self, micros: i64) -> Result<(), AppendError> {
        self.check_temporal(crate::types::TypeId::Time, micros, "append_time")?;
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_time(self.handle, duckdb_time { micros }) })
    }

    /// Appends a `TIMESTAMP` as microseconds since the epoch.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails, or — before `DuckDB` is
    /// called — if `micros` is below `290309-12-22 (BC)` and not `-infinity`,
    /// the range of [`Value::timestamp`][crate::value::Value::timestamp].
    /// `DuckDB` stores such a value unchecked, and every later read of the row
    /// fails with "Date out of range".
    pub fn append_timestamp(&self, micros: i64) -> Result<(), AppendError> {
        self.check_temporal(crate::types::TypeId::Timestamp, micros, "append_timestamp")?;
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe {
            duckdb_append_timestamp(self.handle, duckdb_timestamp { micros })
        })
    }

    /// Appends an `INTERVAL`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_interval(&self, value: DuckInterval) -> Result<(), AppendError> {
        let raw = duckdb_interval {
            months: value.months,
            days: value.days,
            micros: value.micros,
        };
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_interval(self.handle, raw) })
    }

    /// Appends an arbitrary [`Value`], letting `DuckDB` cast it to the column's
    /// type.
    ///
    /// This is the escape hatch for types with no dedicated `append_*`:
    /// `LIST`, `STRUCT`, `MAP`, `UUID`, `DECIMAL`, `ENUM`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if `value` holds a null handle — which
    /// `duckdb_append_value` would dereference — or if the append fails.
    pub fn append_value(&self, value: &Value) -> Result<(), AppendError> {
        self.usable()?;
        if value.as_raw().is_null() {
            // duckdb_append_value dereferences its argument with no null check.
            return Err(append_error("cannot append a null duckdb_value handle"));
        }
        // SAFETY: self.handle is valid and value.as_raw() is non-null.
        self.record_append(unsafe { duckdb_append_value(self.handle, value.as_raw()) })
    }
}

/// Poisons the appender if a [`row`][Appender::row] closure panics after the
/// row's first value, as an error there does. Forgotten on the normal path,
/// so its `drop` runs only while unwinding.
struct PoisonOnUnwind<'a>(&'a Appender);

impl Drop for PoisonOnUnwind<'_> {
    fn drop(&mut self) {
        if self.0.column.get() != 0 {
            self.0.lifecycle.set(Lifecycle::Poisoned);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fits_append_length;

    /// The limit is `u32::MAX` bytes inclusive: a string of exactly that
    /// length is passed to `DuckDB` intact, one byte more would be truncated.
    #[test]
    fn the_append_length_limit_is_u32_max_inclusive() {
        assert!(fits_append_length(0));
        assert!(fits_append_length(u32::MAX as usize));
        #[cfg(target_pointer_width = "64")]
        assert!(!fits_append_length(u32::MAX as usize + 1));
    }
}
