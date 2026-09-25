// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The Arrow layouts `duckdb_data_chunk_from_arrow` imports wrongly, found by
//! walking the array alongside its schema before the import.
//!
//! `DuckDB`'s conversion (`ArrowToDuckDBConversion`, `arrow_conversion.cpp`,
//! the same in 1.5.2–1.5.5 where cited: a `duckdb-1-5-4` build carries the
//! 546-slot API layout, which the default ABI check refuses on 1.5.0 and
//! 1.5.1, whose union and 64-bit run-end paths differ) computes where each
//! node's
//! rows start from two parameters its caller passes: a `parent_offset` and a
//! `nested_offset`, the latter replacing the former inside a `LIST`
//! (`GetEffectiveOffset`). Arrow's rule is that a node's row `i` is element
//! `offset + i` of its buffers, after its ancestors' offsets are applied. The
//! two agree for the layouts `DuckDB`'s own export produces and disagree for
//! valid arrays other producers make:
//!
//! - a `STRUCT` passes only its own `offset` to its children, so a struct
//!   inside a struct whose offset is not zero, or a struct with a nonzero
//!   offset inside a list, imports its fields from the wrong rows
//!   (`docs/upstream-duckdb-reports.md`, item 24);
//! - a `UNION`'s members are converted from row 0 of their arrays, whatever
//!   the union's own offset (item 24);
//! - a dictionary's validity is read without the list's start offset
//!   ("TODO: add support for offsets" in `ArrowToDuckDBList`), and with more
//!   than [`duckdb_vector_size`](libduckdb_sys::duckdb_vector_size) rows and
//!   any NULL — its own or an enclosing struct's — it is copied into a
//!   2048-row mask, past the end of a heap allocation
//!   (items 9 and 25);
//! - a dictionary whose values are dictionary-encoded shares one dictionary
//!   cache with them and imports garbage (item 26);
//! - a run-end-encoded array reads its values' validity from the logical
//!   offset, and one under a fixed-size list or inside another's values is
//!   read as a plain array, from buffers it does not have (item 24);
//! - a list view scans `sum(sizes)` child elements from its lowest offset,
//!   which reads past the child when views overlap or leave gaps (item 27);
//! - a sparse union's type codes are used as member indices, ignoring the
//!   `+us:` code list (item 28);
//! - `null_count = -1` ("not computed") makes a dictionary's NULL rows
//!   import as values, and any nonzero `null_count` on a sparse union makes
//!   its type ids read as validity (items 31 and 32);
//! - a `geoarrow.wkb` array is copied into one 2048-row vector before its
//!   cast, so more rows than that are written past it (item 33).
//!
//! Each is refused with the node it concerns. The walk mirrors `DuckDB`'s
//! calls: [`Ctx`] carries what `DuckDB` passes to a node and where Arrow says
//! the node's rows start, and a node is refused where the two differ.

use super::{ArrowSchema, RawArrowArray};

/// How `DuckDB` reads an Arrow node, from its schema's format string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A type with no children and a data buffer `DuckDB` offsets directly.
    Leaf,
    /// `n`: no buffers, so no offset to get wrong.
    Null,
    /// `+s`.
    Struct,
    /// `+l` (32-bit offsets), `+L` (64-bit offsets) or `+m` (`map: true`).
    List { wide: bool, map: bool },
    /// `+vl` / `+vL`.
    ListView { wide: bool },
    /// `+w:N`.
    FixedList(u64),
    /// `+us:` with type codes `0, 1, …` in order.
    SparseUnion,
    /// `+us:` with any other code list, which `DuckDB` ignores.
    RecodedUnion,
    /// `+r`.
    RunEnd,
    /// A leaf whose Arrow extension type `DuckDB` converts by first copying
    /// the storage into one vector of the standard size (2048), then casting
    /// (`ColumnArrowToDuckDB`'s `arrow_to_duckdb` path): `geoarrow.wkb` from
    /// 1.5.0. More rows than that are written past the vector.
    CopiedIntoOneVector,
}

/// `kind`, read from a format string whose schema has `children` children and
/// whose metadata names `extension`.
pub(super) fn kind_of(format: &str, children: usize, extension: Option<&str>) -> Kind {
    // `arrow.bool8`, the only other core extension with a conversion step,
    // is converted without a copy, so its storage vector is never written.
    if extension == Some("geoarrow.wkb") && !format.starts_with('+') {
        return Kind::CopiedIntoOneVector;
    }
    match format {
        "n" => Kind::Null,
        "+s" => Kind::Struct,
        "+l" => Kind::List {
            wide: false,
            map: false,
        },
        "+m" => Kind::List {
            wide: false,
            map: true,
        },
        "+L" => Kind::List {
            wide: true,
            map: false,
        },
        "+vl" => Kind::ListView { wide: false },
        "+vL" => Kind::ListView { wide: true },
        "+r" => Kind::RunEnd,
        _ => {
            if let Some(size) = format.strip_prefix("+w:") {
                return stoi_prefix(size).map_or(Kind::Leaf, Kind::FixedList);
            }
            if let Some(codes) = format.strip_prefix("+us:") {
                let identity = codes
                    .split(',')
                    .map(str::parse::<usize>)
                    .enumerate()
                    .all(|(i, code)| code == Ok(i));
                let count = codes.split(',').count();
                return if identity && count == children {
                    Kind::SparseUnion
                } else {
                    Kind::RecodedUnion
                };
            }
            Kind::Leaf
        }
    }
}

/// The size `DuckDB` reads from a fixed-size list's format, which it parses
/// with `std::stoi` (`arrow_duck_schema.cpp`): leading C whitespace and a `+`
/// are skipped, and parsing stops at the first non-digit, so `" 2"`, `"+2"`
/// and `"2x"` are all 2. `None` where `stoi` throws or the size is negative,
/// which fails `schema_from_arrow` before any import.
fn stoi_prefix(text: &str) -> Option<u64> {
    let text = text.trim_start_matches([' ', '\t', '\n', '\x0B', '\x0C', '\r']);
    let text = text.strip_prefix('+').unwrap_or(text);
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    text[..digits]
        .parse::<i32>()
        .ok()
        .and_then(|n| u64::try_from(n).ok())
}

/// A schema node, reduced to what the walk needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shape {
    pub kind: Kind,
    pub children: Vec<Self>,
    pub dictionary: Option<Box<Self>>,
}

impl Shape {
    /// The shape of `schema` and everything below it.
    pub(super) fn of(schema: &ArrowSchema) -> Self {
        let count = schema.child_count();
        Self {
            // SAFETY: a live schema's metadata is null or well-formed Arrow
            // metadata owned by the producer for as long as the schema lives.
            kind: kind_of(
                schema.format().unwrap_or(""),
                count,
                unsafe { extension_name(schema.metadata_ptr()) }.as_deref(),
            ),
            children: (0..count)
                .filter_map(|i| schema.child(i).map(Self::of))
                .collect(),
            dictionary: schema.dictionary().map(|d| Box::new(Self::of(d))),
        }
    }
}

/// The value of the `ARROW:extension:name` key in Arrow C Data Interface
/// `metadata`: an `i32` pair count, then per pair an `i32` byte length and the
/// key, an `i32` byte length and the value, in native byte order.
///
/// # Safety
///
/// `metadata` must be null or point to well-formed metadata in that encoding.
pub(super) unsafe fn extension_name(metadata: *const core::ffi::c_char) -> Option<String> {
    /// Reads the `i32` at `at` and advances past it.
    ///
    /// # Safety
    ///
    /// `at` must point to four readable bytes.
    const unsafe fn read_i32(at: &mut *const u8) -> i32 {
        // SAFETY: forwarded from this function's contract.
        unsafe {
            let value = at.cast::<i32>().read_unaligned();
            *at = at.add(4);
            value
        }
    }
    if metadata.is_null() {
        return None;
    }
    let mut at = metadata.cast::<u8>();
    // SAFETY: each read stays within the well-formed metadata the caller
    // guarantees; lengths are checked non-negative before use.
    unsafe {
        let pairs = read_i32(&mut at);
        for _ in 0..pairs.max(0) {
            let key_len = usize::try_from(read_i32(&mut at)).ok()?;
            let key = std::slice::from_raw_parts(at, key_len);
            at = at.add(key_len);
            let value_len = usize::try_from(read_i32(&mut at)).ok()?;
            let value = std::slice::from_raw_parts(at, value_len);
            at = at.add(value_len);
            if key == b"ARROW:extension:name" {
                return Some(String::from_utf8_lossy(value).into_owned());
            }
        }
    }
    None
}

/// How `DuckDB` dispatches a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// By physical type: dictionaries and run-end encoding are handled.
    Physical,
    /// A fixed-size list's child: dictionaries are handled, run-end encoding
    /// is not (`ArrowToDuckDBArray`).
    FixedListChild,
    /// A union member: dictionaries and run-end encoding are handled, but
    /// convert from row 0 with no offset.
    UnionMember,
    /// `ColumnArrowToDuckDB` directly: neither is handled.
    Plain,
}

/// What `DuckDB` passes to a node, and where Arrow says its rows start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ctx {
    /// `DuckDB`'s `nested_offset`, `None` for `-1`.
    pub nested: Option<i64>,
    /// `DuckDB`'s `parent_offset` for the node's values.
    pub parent: i64,
    /// `DuckDB`'s `parent_offset` for the node's validity.
    pub vparent: i64,
    /// Arrow's start for the node's rows, before the node's own offset.
    pub inherited: i64,
    /// Rows `DuckDB` converts from the node.
    pub size: u64,
    /// Whether an enclosing struct's validity mask reaches the node
    /// (`parent_mask` not all valid).
    pub struct_nulls: bool,
    /// Whether an enclosing fixed-size list's NULLs were broadcast into the
    /// node's own validity (`ArrowToDuckDBArray`). Only a `STRUCT` passes its
    /// validity on, so only a `STRUCT` reads this.
    pub broadcast_nulls: bool,
    /// Whether the node descends from a `MAP`'s key/value through only
    /// `STRUCT` fields and fixed-size-list children (no `LIST` in between).
    /// `DuckDB` verifies a map by flattening its entries, and a fixed-size
    /// list is flattened over its vector's capacity, which a map over-allocates
    /// to the chunk's size: a dictionary child there is read past the indices
    /// the conversion set (a heap over-read, upstream item 24).
    pub in_map: bool,
    pub route: Route,
}

impl Ctx {
    /// A top-level column of a record batch of `rows` rows.
    pub(super) const fn column(rows: u64) -> Self {
        Self {
            nested: None,
            parent: 0,
            vparent: 0,
            inherited: 0,
            size: rows,
            struct_nulls: false,
            broadcast_nulls: false,
            in_map: false,
            route: Route::Physical,
        }
    }

    /// Where `DuckDB` reads a node at `offset`'s values from; `None` if the
    /// sum overflows.
    const fn duck_start(self, offset: i64) -> Option<i64> {
        match self.nested {
            Some(nested) => offset.checked_add(nested),
            None => offset.checked_add(self.parent),
        }
    }

    /// Where `DuckDB` reads a node at `offset`'s validity from; `None` if the
    /// sum overflows.
    const fn duck_validity(self, offset: i64) -> Option<i64> {
        match self.nested {
            Some(nested) => offset.checked_add(nested),
            None => offset.checked_add(self.vparent),
        }
    }

    /// Where Arrow says a node at `offset`'s rows start; `None` if the sum
    /// overflows.
    const fn arrow_start(self, offset: i64) -> Option<i64> {
        offset.checked_add(self.inherited)
    }
}

/// An offset sum that does not fit in `i64`, as an error.
fn in_range(sum: Option<i64>) -> Result<i64, String> {
    sum.ok_or_else(|| "an offset, added to its parents' offsets, overflows i64".to_owned())
}

/// Whether `node` has a validity buffer and a nonzero null count, the
/// condition under which `GetValidityMask` copies it.
///
/// # Safety
///
/// `node` must be a valid Arrow array.
unsafe fn copies_validity(node: &RawArrowArray) -> bool {
    node.null_count != 0
        && node.n_buffers > 0
        && !node.buffers.is_null()
        // SAFETY: `buffers` is non-null and holds `n_buffers > 0` entries.
        && !unsafe { *node.buffers }.is_null()
}

/// Element `index` of `node`'s buffer `buffer`, read as 32- or 64-bit.
///
/// # Safety
///
/// `node` must be a valid array whose buffer `buffer` holds more than `index`
/// elements of that width.
unsafe fn read_index(node: &RawArrowArray, buffer: usize, index: i64, wide: bool) -> Option<i64> {
    let index = usize::try_from(index).ok()?;
    if node.buffers.is_null() || usize::try_from(node.n_buffers).ok()? <= buffer {
        return None;
    }
    // SAFETY: `buffer < n_buffers` and the caller's contract covers `index`.
    unsafe {
        let data = *node.buffers.add(buffer);
        if data.is_null() {
            return None;
        }
        if wide {
            Some(data.cast::<i64>().add(index).read_unaligned())
        } else {
            Some(i64::from(data.cast::<i32>().add(index).read_unaligned()))
        }
    }
}

/// The range of child elements a list view's rows `start..start + size`
/// cover, as `DuckDB` computes it — `(lowest offset, sum of sizes)` — and
/// the end of the furthest view, or an error if the views do not fit it.
///
/// # Safety
///
/// `node` must be a valid list view whose offset and size buffers cover rows
/// `start..start + size`.
unsafe fn list_view_range(
    node: &RawArrowArray,
    start: i64,
    size: u64,
    wide: bool,
) -> Result<(i64, i64), String> {
    let size = i64::try_from(size).map_err(|_| "row count out of range".to_owned())?;
    let offset = |i| {
        // SAFETY: row `start + i` for `i < size` is covered by the caller's
        // contract.
        unsafe { read_index(node, 1, start + i, wide) }.ok_or("no offsets buffer")
    };
    let length = |i| {
        // SAFETY: as above, for the sizes buffer.
        unsafe { read_index(node, 2, start + i, wide) }.ok_or("no sizes buffer")
    };
    let mut lowest = if size > 0 { offset(0)? } else { 0 };
    let mut total: i64 = 0;
    let mut furthest: i64 = 0;
    for i in 0..size {
        let (o, l) = (offset(i)?, length(i)?);
        total = total.saturating_add(l);
        if l != 0 {
            lowest = lowest.min(o);
            furthest = furthest.max(o.saturating_add(l));
        }
    }
    if furthest > lowest.saturating_add(total) {
        return Err(format!(
            "a list view whose views overlap or leave gaps (child elements {lowest} to \
             {furthest} in use, but DuckDB converts {total} from {lowest}) would be imported \
             from the wrong child rows"
        ));
    }
    Ok((lowest, total))
}

/// The number of `duckdb_vector_size()` rows: `DuckDB`'s validity masks for
/// dictionary indices hold this many.
pub(super) type Limit = u64;

/// Refuses any column of `array` that `DuckDB` would import wrongly; see the
/// [module docs](self). `shapes` holds one shape per column.
///
/// # Safety
///
/// `array` must be a valid, unreleased struct array with non-null children,
/// one per shape, that conforms to the shapes (the contract of
/// [`data_chunk_from_arrow`][super::data_chunk_from_arrow]).
pub(super) unsafe fn check(
    array: &RawArrowArray,
    shapes: &[Shape],
    limit: Limit,
) -> Result<(), String> {
    let columns = usize::try_from(array.n_children).unwrap_or(0);
    if columns != shapes.len() {
        return Err(format!(
            "the batch has {columns} column(s) but {} schema shape(s)",
            shapes.len()
        ));
    }
    let rows = u64::try_from(array.length).unwrap_or(0);
    for (column, shape) in shapes.iter().enumerate() {
        // SAFETY: the caller guarantees one live child per shape.
        let child = unsafe { &**array.children.add(column) };
        // SAFETY: as above; the child conforms to `shape`.
        unsafe { check_node(child, shape, Ctx::column(rows), limit) }
            .map_err(|e| format!("column {column}: {e}"))?;
    }
    Ok(())
}

/// Whether `DuckDB` can hold `rows` elements in one vector under `max`
/// ([`MAX_CAPACITY`](crate::vector::ops::MAX_CAPACITY) on this target),
/// counting the rounding up to a power of two that a list child's reserve
/// adds. Above it the allocation either throws before
/// `duckdb_data_chunk_from_arrow`'s `try` (aborting the process) or, on a
/// 32-bit target, has its byte size narrowed by `malloc` and comes back too
/// small.
const fn fits_one_vector(rows: u64, max: u64) -> bool {
    match rows.checked_next_power_of_two() {
        Some(reserved) => reserved <= max,
        None => false,
    }
}

/// Checks `node`, reached with `ctx`, and everything below it.
///
/// # Safety
///
/// As [`check`], for `node` and `shape`.
#[allow(clippy::too_many_lines)]
unsafe fn check_node(
    node: &RawArrowArray,
    shape: &Shape,
    ctx: Ctx,
    limit: Limit,
) -> Result<(), String> {
    if node.length < 0 || node.offset < 0 {
        return Err(format!(
            "the array has a negative length ({}) or offset ({})",
            node.length, node.offset
        ));
    }
    let max = crate::vector::ops::MAX_CAPACITY as u64;
    if !fits_one_vector(ctx.size, max) {
        return Err(format!(
            "{} rows here is more rows than one DuckDB vector can hold on this target \
             ({max}); DuckDB would size the vector before its error handling starts",
            ctx.size
        ));
    }
    let children = usize::try_from(node.n_children).unwrap_or(0);
    if children != shape.children.len() {
        return Err(format!(
            "the array has {children} child array(s) where its schema has {}",
            shape.children.len()
        ));
    }
    if node.dictionary.is_null() != shape.dictionary.is_none() {
        return Err("the array and its schema disagree on dictionary encoding".to_owned());
    }
    let child = |i: usize| {
        if i >= children {
            return Err(format!("child {i} is missing"));
        }
        // SAFETY: `i < n_children`, and a valid array's children are live.
        unsafe { (*node.children.add(i)).as_ref() }.ok_or_else(|| format!("child {i} is null"))
    };

    if let Some(values) = &shape.dictionary {
        // Which parameters reach the dictionary function depends on the route.
        let ctx = if ctx.route == Route::UnionMember {
            Ctx {
                nested: None,
                parent: 0,
                vparent: 0,
                ..ctx
            }
        } else {
            ctx
        };
        if ctx.route == Route::Plain {
            return Err(
                "a dictionary-encoded array where DuckDB reads a plain one (a run-end-encoded \
                 array's values, or the child of an empty list) would be imported from its \
                 indices"
                    .to_owned(),
            );
        }
        if ctx.in_map && ctx.route == Route::FixedListChild {
            return Err(
                "a dictionary-encoded array as a fixed-size list's child under a MAP: DuckDB \
                 verifies the map by flattening the fixed-size list over its vector's capacity \
                 (which a map over-allocates to the chunk's size), reading the dictionary \
                 indices past the ones the conversion set (upstream item 24). Decode the \
                 dictionary before importing"
                    .to_owned(),
            );
        }
        let arrow = in_range(ctx.arrow_start(node.offset))?;
        let duck = in_range(ctx.duck_start(node.offset))?;
        if ctx.size > 0 && duck != arrow {
            return Err(format!(
                "dictionary indices would be read from row {duck} where Arrow puts them at row \
                 {arrow}"
            ));
        }
        // SAFETY: `node` is a valid array.
        let own_nulls = unsafe { copies_validity(node) };
        // `GetValidityMask` copies a bitmap when `null_count != 0`, but
        // `CanContainNull` (which decides whether the indices consult it)
        // tests `null_count > 0`.
        if own_nulls && node.null_count < 0 {
            return Err(format!(
                "a dictionary-encoded array with null_count {} (not computed) would have its \
                 NULL rows imported as values: DuckDB builds the selection without its \
                 validity. Set null_count to the number of NULLs",
                node.null_count
            ));
        }
        // `GetValidityMask` is passed `parent_offset` but not `nested_offset`.
        let validity = in_range(node.offset.checked_add(ctx.parent))?;
        if own_nulls && ctx.size > 0 && validity != arrow {
            return Err(format!(
                "the validity of dictionary indices would be read from row {validity} where \
                 Arrow puts it at row {arrow} (DuckDB ignores the list's offset here)"
            ));
        }
        if (own_nulls || ctx.struct_nulls) && ctx.size > limit {
            return Err(format!(
                "a dictionary-encoded array of {} rows that can hold NULLs (its own or an \
                 enclosing struct's) would have its validity copied into a {limit}-row mask, \
                 past the end of a heap allocation. Split the batch so that no \
                 dictionary-encoded array, including a list's child, is read as more than \
                 {limit} rows, or decode the dictionary before importing",
                ctx.size
            ));
        }
        if values.dictionary.is_some() {
            return Err(
                "a dictionary whose values are dictionary-encoded shares one dictionary cache \
                 with them in DuckDB and would import the wrong values"
                    .to_owned(),
            );
        }
        // SAFETY: a valid dictionary-encoded array's dictionary is live.
        let dict = unsafe { node.dictionary.as_ref() }.ok_or("the dictionary is null")?;
        let dict_ctx = Ctx {
            size: u64::try_from(dict.length).unwrap_or(0),
            ..Ctx::column(0)
        };
        // SAFETY: the dictionary conforms to `values`.
        return unsafe { check_node(dict, values, dict_ctx, limit) }
            .map_err(|e| format!("dictionary: {e}"));
    }

    // A union has no validity bitmap; with any nonzero `null_count`,
    // `GetValidityMask` reads a sparse union's type ids (its buffer 0) as one.
    if matches!(shape.kind, Kind::SparseUnion | Kind::RecodedUnion) && node.null_count != 0 {
        return Err(format!(
            "a union with null_count {} would have its type ids read as a validity bitmap \
             (a union has none, so its null_count must be 0)",
            node.null_count
        ));
    }

    // Union members that are run-end encoded convert from row 0 too.
    let ctx = if ctx.route == Route::UnionMember && shape.kind == Kind::RunEnd {
        Ctx {
            nested: None,
            parent: 0,
            vparent: 0,
            ..ctx
        }
    } else {
        ctx
    };
    if shape.kind == Kind::Null {
        return Ok(());
    }
    // A node `DuckDB` converts as zero rows reads none of its own, but its
    // descendants are still converted (a run-end-encoded array below it is
    // expanded at full length), so the walk goes on; only the row checks
    // are skipped.
    let rows = ctx.size > 0;
    // SAFETY: `node` is a valid array.
    let has_validity = unsafe { copies_validity(node) };
    let arrow = in_range(ctx.arrow_start(node.offset))?;
    let validity = in_range(ctx.duck_validity(node.offset))?;
    if rows && has_validity && validity != arrow {
        return Err(format!(
            "validity would be read from row {validity} where Arrow puts it at row {arrow}"
        ));
    }
    let reads_values = !matches!(shape.kind, Kind::Struct | Kind::FixedList(_));
    let start = in_range(ctx.duck_start(node.offset))?;
    if rows && reads_values && start != arrow {
        return Err(format!(
            "rows would be read from row {start} where Arrow puts them at row {arrow}"
        ));
    }

    match shape.kind {
        Kind::CopiedIntoOneVector if ctx.size > limit => Err(format!(
            "a geoarrow.wkb array of {} rows would have its storage copied into a {limit}-row \
             vector before the cast to GEOMETRY, past the end of a heap allocation. Split the \
             batch so that no geoarrow.wkb array, including a list's child, is read as more \
             than {limit} rows",
            ctx.size
        )),
        Kind::Leaf | Kind::Null | Kind::CopiedIntoOneVector => Ok(()),
        Kind::Struct => {
            let mask = ctx.struct_nulls || ctx.broadcast_nulls || has_validity;
            for (i, s) in shape.children.iter().enumerate() {
                let c = Ctx {
                    nested: ctx.nested,
                    parent: node.offset,
                    vparent: node.offset,
                    inherited: arrow,
                    size: ctx.size,
                    struct_nulls: mask,
                    broadcast_nulls: false,
                    in_map: ctx.in_map,
                    route: Route::Physical,
                };
                // SAFETY: the child conforms to `s`.
                unsafe { check_node(child(i)?, s, c, limit) }
                    .map_err(|e| format!("field {i}: {e}"))?;
            }
            Ok(())
        }
        Kind::List { wide, .. } | Kind::ListView { wide } => {
            let size = i64::try_from(ctx.size).map_err(|_| "row count out of range")?;
            // A list `DuckDB` converts as zero rows is empty to it whatever its
            // offsets say: `ConvertArrowListOffsetsTemplated` returns 0 and 0
            // for `size == 0`, and `ArrowToDuckDBList` then reads the child as
            // a plain array (upstream item 24).
            let (first, total) = if ctx.size == 0 {
                (0, 0)
            } else if let Kind::ListView { .. } = shape.kind {
                // SAFETY: the list view's buffers cover its rows.
                unsafe { list_view_range(node, start, ctx.size, wide) }?
            } else {
                // SAFETY: a list's offsets buffer holds `size + 1` entries
                // from its start.
                let first =
                    unsafe { read_index(node, 1, start, wide) }.ok_or("no offsets buffer")?;
                // SAFETY: as above.
                let last = unsafe { read_index(node, 1, start + size, wide) }
                    .ok_or("no offsets buffer")?;
                (first, last - first)
            };
            let item = child(0)?;
            if let Kind::ListView { .. } = shape.kind {
                if first.saturating_add(total) > item.length {
                    return Err(format!(
                        "a list view whose sizes add up to {total} from child element {first} \
                         would be read past its child's {} elements",
                        item.length
                    ));
                }
            }
            let total = u64::try_from(total).unwrap_or(0);
            let empty = total == 0 && first == 0;
            // `ArrowToDuckDBList` passes the list's offset for the child's
            // validity (overridden by `nested_offset`) and none to the child's
            // dictionary or plain conversion.
            let c = Ctx {
                nested: if empty { None } else { Some(first) },
                parent: 0,
                vparent: node.offset,
                inherited: first,
                size: total,
                struct_nulls: false,
                broadcast_nulls: false,
                in_map: matches!(shape.kind, Kind::List { map: true, .. }),
                route: if empty { Route::Plain } else { Route::Physical },
            };
            // SAFETY: the child conforms to the list's child shape.
            unsafe { check_node(item, &shape.children[0], c, limit) }
                .map_err(|e| format!("list child: {e}"))
        }
        Kind::FixedList(n) => {
            let n = i64::try_from(n).map_err(|_| "array size out of range")?;
            let first = start.checked_mul(n).ok_or("child offset out of range")?;
            let total = ctx.size.saturating_mul(u64::try_from(n).unwrap_or(0));
            let empty = total == 0 && first == 0;
            // As for a list: `ArrowToDuckDBArray` passes no `parent_offset`
            // to the child's conversion.
            let c = Ctx {
                nested: if empty { None } else { Some(first) },
                parent: 0,
                vparent: node.offset,
                inherited: arrow.checked_mul(n).ok_or("child offset out of range")?,
                size: total,
                struct_nulls: false,
                // Its own NULLs and an enclosing struct's are broadcast into
                // the child's validity.
                broadcast_nulls: ctx.struct_nulls || ctx.broadcast_nulls || has_validity,
                in_map: ctx.in_map,
                route: if empty {
                    Route::Plain
                } else {
                    Route::FixedListChild
                },
            };
            // SAFETY: the child conforms to the array's child shape.
            unsafe { check_node(child(0)?, &shape.children[0], c, limit) }
                .map_err(|e| format!("array child: {e}"))
        }
        Kind::SparseUnion => {
            for (i, s) in shape.children.iter().enumerate() {
                let c = Ctx {
                    nested: ctx.nested,
                    parent: 0,
                    vparent: ctx.parent,
                    inherited: arrow,
                    size: ctx.size,
                    struct_nulls: false,
                    broadcast_nulls: false,
                    in_map: false,
                    route: Route::UnionMember,
                };
                // SAFETY: the member conforms to `s`.
                unsafe { check_node(child(i)?, s, c, limit) }
                    .map_err(|e| format!("member {i}: {e}"))?;
            }
            Ok(())
        }
        Kind::RecodedUnion => Err(
            "a sparse union whose type codes are not 0, 1, … in order would have its codes \
             used as member indices"
                .to_owned(),
        ),
        Kind::RunEnd => {
            if matches!(ctx.route, Route::Plain | Route::FixedListChild) {
                return Err(
                    "a run-end-encoded array where DuckDB reads a plain one (a fixed-size \
                     list's child, or a run-end-encoded array's values) would be read from \
                     buffers it does not have"
                        .to_owned(),
                );
            }
            let values = child(1)?;
            // SAFETY: `values` is a valid array.
            let value_nulls = unsafe { copies_validity(values) };
            let context = ctx.nested.unwrap_or(ctx.parent);
            if value_nulls && context != 0 {
                return Err(format!(
                    "the validity of a run-end-encoded array's values would be read from run \
                     {context} on (its logical offset), not from run 0"
                ));
            }
            let runs = Ctx {
                size: u64::try_from(child(0)?.length).unwrap_or(0),
                ..Ctx::column(0)
            };
            // SAFETY: the run ends conform to their shape.
            unsafe { check_node(child(0)?, &shape.children[0], runs, limit) }
                .map_err(|e| format!("run ends: {e}"))?;
            let vals = Ctx {
                size: u64::try_from(values.length).unwrap_or(0),
                route: Route::Plain,
                ..Ctx::column(0)
            };
            // SAFETY: the values conform to their shape.
            unsafe { check_node(values, &shape.children[1], vals, limit) }
                .map_err(|e| format!("values: {e}"))
        }
    }
}

#[cfg(test)]
mod tests;
