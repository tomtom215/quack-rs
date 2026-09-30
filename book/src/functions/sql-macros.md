# SQL Macros

A DuckDB SQL macro packages a SQL expression or query as a named function, with no
FFI callback behind it. This page shows how to create scalar and table macros from a
Rust extension with quack-rs' `SqlMacro`: you give the macro body as a string and call
`.register(con)`, which runs `CREATE OR REPLACE MACRO`.

---

## Two macro types

| Type | SQL generated | Returns |
|------|--------------|---------|
| **Scalar** | `CREATE OR REPLACE MACRO "name"("a", "b") AS (expression)` | one value per row |
| **Table** | `CREATE OR REPLACE MACRO "name"("a", "b") AS TABLE query` | a result set |

---

## Scalar macros

A scalar macro wraps a SQL expression. Think of it as a parameterized SQL alias:

```rust
# use libduckdb_sys::{duckdb_connection, DuckDBSuccess};
# use quack_rs::error::ExtensionError;
# fn live_connection() -> duckdb_connection {
#     std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
#     let (mut db, mut con) = (std::ptr::null_mut(), std::ptr::null_mut());
#     unsafe {
#         assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), DuckDBSuccess);
#         assert_eq!(libduckdb_sys::duckdb_connect(db, &mut con), DuckDBSuccess);
#     }
#     con
# }
# fn query_i64(con: duckdb_connection, sql: &str) -> i64 {
#     let mut result = unsafe { quack_rs::query::query(con, sql) }.unwrap();
#     let chunk = result.next_chunk().unwrap().unwrap();
#     unsafe { chunk.reader(0).read_i64(0) }
# }
use quack_rs::sql_macro::SqlMacro;

fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        // clamp(x, lo, hi) → greatest(lo, least(hi, x))
        SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))")?
            .register(con)?;

        // golden_ratio() → 1.61803398874989
        SqlMacro::scalar("golden_ratio", &[], "1.61803398874989")?
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
# let con = live_connection();
# register(con).unwrap();
# assert_eq!(query_i64(con, "SELECT clamp(9, 1, 5)::BIGINT"), 5);
# assert_eq!(query_i64(con, "SELECT clamp(-3, 1, 5)::BIGINT"), 1);
# assert_eq!(query_i64(con, "SELECT (golden_ratio() * 1000)::BIGINT"), 1618);
# assert_eq!(query_i64(con, "SELECT count(*) FROM (SELECT safe_div(1, 0) AS v) WHERE v IS NULL"), 1);
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
# use libduckdb_sys::{duckdb_connection, DuckDBSuccess};
# use quack_rs::error::ExtensionError;
# use quack_rs::sql_macro::SqlMacro;
# fn live_connection() -> duckdb_connection {
#     std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
#     let (mut db, mut con) = (std::ptr::null_mut(), std::ptr::null_mut());
#     unsafe {
#         assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), DuckDBSuccess);
#         assert_eq!(libduckdb_sys::duckdb_connect(db, &mut con), DuckDBSuccess);
#     }
#     con
# }
# fn query_i64(con: duckdb_connection, sql: &str) -> i64 {
#     let mut result = unsafe { quack_rs::query::query(con, sql) }.unwrap();
#     let chunk = result.next_chunk().unwrap().unwrap();
#     unsafe { chunk.reader(0).read_i64(0) }
# }
# fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
unsafe {
    // active_users(tbl) → SELECT * FROM query_table(tbl) WHERE active = true
    SqlMacro::table(
        "active_users",
        &["tbl"],
        "SELECT * FROM query_table(tbl) WHERE active = true",
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
# Ok(())
# }
# let con = live_connection();
# unsafe { quack_rs::query::execute(con, "CREATE TABLE users AS SELECT * FROM (VALUES (1, true), (2, false), (3, true)) t(id, active)") }.unwrap();
# unsafe { quack_rs::query::execute(con, "CREATE TABLE orders AS SELECT current_date - 3 AS order_date UNION ALL SELECT current_date - 30") }.unwrap();
# register(con).unwrap();
# assert_eq!(query_i64(con, "SELECT count(*) FROM recent_orders(7)"), 1);
# assert_eq!(query_i64(con, "SELECT count(*) FROM active_users(users)"), 2);
```

A table macro's body is bound when the macro is created, so every table it names
directly (`orders` above) must already exist when `register` runs, or registration
fails with a catalog error. To take a table as a parameter, read it through
`query_table(tbl)`: a bare `FROM tbl` looks for a table literally named `tbl`.

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
# use quack_rs::sql_macro::SqlMacro;
let m = SqlMacro::scalar("add", &["a", "b"], "a + b")?;
assert_eq!(
    m.to_sql(),
    r#"CREATE OR REPLACE MACRO "add"("a", "b") AS (a + b)"#
);

let t = SqlMacro::table("active_users", &["tbl"], "SELECT * FROM query_table(tbl) WHERE active = true")?;
assert_eq!(
    t.to_sql(),
    r#"CREATE OR REPLACE MACRO "active_users"("tbl") AS TABLE SELECT * FROM query_table(tbl) WHERE active = true"#
);
# Ok::<(), quack_rs::error::ExtensionError>(())
```

---

## Name and parameter validation

Macro names are validated with
[`validate_function_name`](https://docs.rs/quack-rs/latest/quack_rs/validate/function_name/fn.validate_function_name.html),
the same rules as function names, and parameter names with
[`validate_parameter_name`](https://docs.rs/quack-rs/latest/quack_rs/validate/function_name/fn.validate_parameter_name.html).
A name must:

- start with an ASCII letter or underscore, followed by ASCII letters, digits or
  underscores;
- be at most 256 characters long;
- not be a DuckDB keyword that cannot be used in that position unquoted. For a
  macro, that is a keyword it cannot be *called* by (`order`, `coalesce`); for a
  parameter, one the body cannot *refer* to it by (`order`, `left`). The two lists
  differ: a parameter may be called `columns`, which a macro may not.

Case is not restricted — DuckDB identifiers are case-insensitive, so a macro
registered as `MyMacro` is callable as `mymacro(...)` or `MYMACRO(...)`.

```rust
# use quack_rs::sql_macro::SqlMacro;
assert!(SqlMacro::scalar("1f", &[], "1").is_err());       // ❌ starts with a digit
assert!(SqlMacro::scalar("my-macro", &[], "1").is_err()); // ❌ hyphen
assert!(SqlMacro::scalar("f", &["a b"], "1").is_err());   // ❌ space in param
assert!(SqlMacro::scalar("f", &["order"], "1").is_err()); // ❌ reserved keyword as param
assert!(SqlMacro::scalar("f", &["left"], "1").is_err());  // ❌ a body cannot refer to it
assert!(SqlMacro::scalar("columns", &[], "1").is_err());  // ❌ cannot be called as a macro
assert!(SqlMacro::scalar("f", &["columns"], "1").is_ok()); // ✅ fine as a parameter
assert!(SqlMacro::scalar("MyMacro", &[], "1").is_ok());   // ✅ mixed case allowed
assert!(SqlMacro::scalar("f", &["X"], "1").is_ok());      // ✅ mixed-case param allowed
assert!(SqlMacro::scalar("f", &["_x"], "1").is_ok());     // ✅ underscore prefix allowed
```

---

## SQL injection safety

Macro and parameter **names** are restricted to ASCII letters, digits and underscores,
preventing SQL injection at the identifier level. `to_sql()` additionally emits every
name as a double-quoted identifier (`"name"`) — the validated character set cannot
contain `"`, so no escaping is needed. Quoting does not make names case-sensitive
in DuckDB.

The **body** (expression or query) is your own extension code — it is included verbatim.
**Never build macro bodies from untrusted user input.** `register` does refuse a body
that turns the statement into several (`1); DROP TABLE t; SELECT (1`) — it counts
statements with DuckDB's own parser (`duckdb_extract_statements`) and executes
nothing if there is more than one — but a body can still change the meaning of the
single statement it is part of.

A scalar body containing `--` gets a newline before the closing parenthesis, so a
trailing line comment (`"x + 1 -- plus one"`) does not comment it out.

---

## Where a macro lives

A macro is not a function registration: `register` runs `CREATE OR REPLACE MACRO`,
so the macro is an ordinary catalog object in the connection's default database and
schema — the **user's** database:

- **It persists.** In a database file the macro is still there in the next session,
  even if the extension is never loaded again. Reloading is fine: `OR REPLACE`
  replaces it.
- **It needs a writable database.** On a read-only database the `CREATE` fails; an
  entry point that propagates that error with `?` makes `LOAD` fail. Decide whether
  a macro is essential there.
- **It silently replaces a user's macro of the same name.** Prefix macro names with
  your extension's name.
- **It can shadow a built-in.** A macro named `abs` in the default schema is found
  before the built-in `abs`, so `abs(-1)` calls the macro.

---

## How it works under the hood

`SqlMacro::register` counts the statements in `to_sql()` with
`duckdb_extract_statements`, refuses more than one, and executes the
`CREATE OR REPLACE MACRO` statement via `duckdb_query`.

The query result is zero-initialized before `duckdb_query`, any error message is read
with `duckdb_result_error`, and `duckdb_destroy_result` runs on success and failure
alike.

---

## Choosing between macros and scalar functions

| Scenario | Use |
|----------|-----|
| Logic expressible in SQL | SQL macro — simpler, no FFI |
| Logic needs Rust code (algorithms, external crates, etc.) | Scalar function |
| Simple expressions | SQL macro (expanded inline, no callback per chunk) |
| Type-specific overloads | Scalar function set (`ScalarFunctionSetBuilder`) |
| Returning a table | SQL table macro |
