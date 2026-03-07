# SQL Macros

SQL macros let you package reusable SQL expressions and queries as named DuckDB functions —
no FFI callbacks required. quack-rs makes this pure Rust: you define the macro body as a
string and call `.register(con)`.

---

## Two macro types

| Type | SQL generated | Returns |
|------|--------------|---------|
| **Scalar** | `CREATE OR REPLACE MACRO name(params) AS (expression)` | one value per row |
| **Table** | `CREATE OR REPLACE MACRO name(params) AS TABLE query` | a result set |

---

## Scalar macros

A scalar macro wraps a SQL expression. Think of it as a parameterized SQL alias:

```rust
use quack_rs::sql_macro::SqlMacro;

fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        // clamp(x, lo, hi) → greatest(lo, least(hi, x))
        SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))")?
            .register(con)?;

        // pi() → 3.14159265358979
        SqlMacro::scalar("pi", &[], "3.14159265358979")?
            .register(con)?;

        // safe_div(a, b) → CASE WHEN b = 0 THEN NULL ELSE a / b END
        SqlMacro::scalar(
            "safe_div",
            &["a", "b"],
            "CASE WHEN b = 0 THEN NULL ELSE a / b END",
        )?
        .register(con)?;
    }
    Ok(())
}
```

Use in DuckDB:

```sql
SELECT clamp(rating, 1, 5) FROM reviews;
SELECT safe_div(revenue, orders) FROM monthly_stats;
```

---

## Table macros

A table macro wraps a SQL query that returns rows:

```rust
unsafe {
    // active_users(tbl) → SELECT * FROM tbl WHERE active = true
    SqlMacro::table(
        "active_users",
        &["tbl"],
        "SELECT * FROM tbl WHERE active = true",
    )?
    .register(con)?;

    // recent_orders(days) → last N days of orders
    SqlMacro::table(
        "recent_orders",
        &["days"],
        "SELECT * FROM orders WHERE order_date >= current_date - INTERVAL (days) DAY",
    )?
    .register(con)?;
}
```

Use in DuckDB:

```sql
SELECT * FROM active_users(users);
SELECT count(*) FROM recent_orders(7);
```

---

## Inspecting the generated SQL

`to_sql()` returns the `CREATE OR REPLACE MACRO` statement without requiring a live connection.
Use it for logging, debugging, or assertions in tests:

```rust
let m = SqlMacro::scalar("add", &["a", "b"], "a + b")?;
assert_eq!(
    m.to_sql(),
    "CREATE OR REPLACE MACRO add(a, b) AS (a + b)"
);

let t = SqlMacro::table("active_users", &["tbl"], "SELECT * FROM tbl WHERE active = true")?;
assert_eq!(
    t.to_sql(),
    "CREATE OR REPLACE MACRO active_users(tbl) AS TABLE SELECT * FROM tbl WHERE active = true"
);
```

---

## Name and parameter validation

Macro names and parameter names are validated against the same rules as function names:
- Must match `[a-z_][a-z0-9_]*`
- Not exceed 256 characters
- No null bytes

```rust
SqlMacro::scalar("MyMacro", &[], "1")   // ❌ Err — uppercase
SqlMacro::scalar("my-macro", &[], "1") // ❌ Err — hyphen
SqlMacro::scalar("f", &["X"], "1")     // ❌ Err — uppercase param
SqlMacro::scalar("f", &["_x"], "1")    // ✅ Ok  — underscore prefix allowed
```

---

## SQL injection safety

Macro and parameter **names** are restricted to `[a-z_][a-z0-9_]*`, preventing SQL
injection at the identifier level. They are interpolated literally (no quoting required,
since the character set is already safe).

The **body** (expression or query) is your own extension code — it is included verbatim.
**Never build macro bodies from untrusted user input.**

---

## How it works under the hood

`SqlMacro::register` executes the `CREATE OR REPLACE MACRO` statement via `duckdb_query`:

```rust
pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
    let sql = self.to_sql();
    unsafe { execute_sql(con, &sql) }
}
```

`execute_sql` zero-initializes a `duckdb_result`, calls `duckdb_query`, extracts any error
message via `duckdb_result_error`, and always calls `duckdb_destroy_result` — even on failure.

---

## Choosing between macros and scalar functions

| Scenario | Use |
|----------|-----|
| Logic expressible in SQL | SQL macro — simpler, no FFI |
| Logic needs Rust code (algorithms, external crates, etc.) | Scalar function |
| Best performance for simple expressions | SQL macro (no FFI overhead) |
| Type-specific overloads | Scalar function with multiple registrations |
| Returning a table | SQL table macro |
