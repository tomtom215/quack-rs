# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.2.x   | Yes                |
| 0.1.x   | No (end-of-life)   |

## Reporting a Vulnerability

If you discover a security vulnerability in quack-rs, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please use GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
feature on this repository.

### What to include

- A description of the vulnerability
- Steps to reproduce (minimal test case preferred)
- The impact (e.g., memory safety, information disclosure, denial of service)
- The affected version(s)

### Response timeline

- **Acknowledgment**: Within 48 hours of report
- **Assessment**: Within 7 days
- **Fix**: Depends on severity; critical issues are prioritized

### Scope

This security policy covers:

- Memory safety issues in unsafe code (use-after-free, double-free, buffer overflow)
- Undefined behavior in FFI callbacks
- Potential for panic across FFI boundaries (which is UB in Rust)
- Information disclosure through uninitialized memory

This policy does **not** cover:

- Bugs in DuckDB itself (report those to [DuckDB](https://github.com/duckdb/duckdb))
- Bugs in `libduckdb-sys` bindings (report to [duckdb-rs](https://github.com/duckdb/duckdb-rs))
- Logic errors in extension code built with quack-rs (those are the extension author's responsibility)

## Security Design

quack-rs is designed with safety as a primary concern:

1. **`#![deny(unsafe_op_in_unsafe_fn)]`** in `src/lib.rs` and **`unsafe_op_in_unsafe_fn = "warn"`** in `Cargo.toml` (promoted to error in CI via `RUSTFLAGS="-D warnings"`): All unsafe operations require explicit `unsafe` blocks with `// SAFETY:` comments, even inside `unsafe fn`.
2. **No panics across FFI**: All entry points and callbacks use `Result`/`Option`. The release profile sets `panic = "abort"` as defense-in-depth.
3. **Double-free prevention**: `FfiState<T>::destroy_callback` nulls pointers after freeing.
4. **Boolean UB prevention**: `VectorReader::read_bool` reads as `u8 != 0`, never transmutes to `bool`.
5. **RAII for DuckDB handles**: `LogicalType` ensures `duckdb_destroy_logical_type` is always called.
