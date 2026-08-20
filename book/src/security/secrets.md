# Secrets Management

Extensions that access external services (HTTP APIs, databases, cloud storage)
commonly need credentials.

**DuckDB's `CREATE SECRET` system is not reachable from an extension.** There is
no `duckdb_secret_*` function anywhere in `duckdb_ext_api_v1` — not one among the
546 slots in DuckDB 1.5.5 — so an extension cannot ask DuckDB for a credential
through the C API. Querying the `duckdb_secrets()` table function (which
[`list_duckdb_secrets`] does) returns metadata only: DuckDB redacts the
sensitive fields.

```text
CREATE SECRET s (TYPE s3, KEY_ID 'AKIAEXAMPLE', SECRET 'super-secret-value');
SELECT secret_string FROM duckdb_secrets();
-- ...;key_id=AKIAEXAMPLE;secret=redacted
```

So an extension needing the credential itself must get it another way — an
environment variable, a config option, a file, its own key store. The
[`secrets`](https://docs.rs/quack-rs/latest/quack_rs/secrets/index.html) module
gives you leak-resistant types for holding those credentials once you have them;
it is **not** a bridge into DuckDB's secret store.

[`list_duckdb_secrets`]: https://docs.rs/quack-rs/latest/quack_rs/secrets/fn.list_duckdb_secrets.html

## Core Types

### `SecretEntry`

A single secret entry with metadata and key-value fields. Designed to minimize
accidental credential leakage:

- **`Debug` redacts field values** — only keys are shown, values replaced with
  `"[REDACTED]"`
- **`Drop` zeroizes sensitive data** — all field values overwritten with zeros
  using `std::ptr::write_volatile` before deallocation
- **No `PartialEq`** — prevents accidental non-constant-time comparisons of
  secret material
- **`Clone` is explicit** — callers are aware they are duplicating sensitive
  material in memory

### `SecretsManager`

The trait extensions implement to provide secret lookup:

```rust
use quack_rs::secrets::{SecretEntry, SecretsManager};

struct MySecrets {
    entries: Vec<SecretEntry>,
}

impl SecretsManager for MySecrets {
    fn get_secret(&self, name: &str, secret_type: &str) -> Option<SecretEntry> {
        self.entries.iter()
            .find(|e| e.name() == name && e.secret_type() == secret_type)
            .cloned()
    }

    fn list_secrets(&self, secret_type: Option<&str>) -> Vec<SecretEntry> {
        self.entries.iter()
            .filter(|e| secret_type.is_none() || secret_type == Some(e.secret_type()))
            .cloned()
            .collect()
    }

    fn remove_secret(&self, _name: &str, _secret_type: &str) -> bool {
        false // read-only example
    }
}
```

## Building Secret Entries

Use the builder pattern:

```rust
use quack_rs::secrets::SecretEntry;

let entry = SecretEntry::new("my_api_key", "bearer")
    .with_provider("config")
    .with_scope("https://api.example.com")
    .with_field("token", "sk-abc123")
    .with_field("refresh_token", "xyz789");

assert_eq!(entry.name(), "my_api_key");
assert_eq!(entry.secret_type(), "bearer");
assert_eq!(entry.get_field("token"), Some("sk-abc123"));
```

## Safe Diagnostics

Use `field_keys()` for logging without leaking secrets:

```rust
use quack_rs::secrets::SecretEntry;

let entry = SecretEntry::new("key", "s3")
    .with_field("access_key", "AKIA...")
    .with_field("secret_key", "wJalr...");

// Safe for logging — returns keys only, no values
let keys = entry.field_keys();
// keys: ["access_key", "secret_key"]
```

## Debug Output

The `Debug` implementation redacts all sensitive values:

```text
SecretEntry {
    name: "api_key",
    secret_type: "bearer",
    provider: "config",
    scope: "",
    fields: {"token": "[REDACTED]"}
}
```

## Security Best Practices

1. **Never log secret field values** — use `field_keys()` for diagnostics
2. **Drop clones promptly** — minimize the window during which sensitive data
   resides in memory
3. **Implement `remove_secret` with zeroization** — don't just remove the
   reference; zeroize the data before deallocation
4. **Thread safety** — `SecretsManager` implementations must be `Send + Sync`
   as DuckDB may invoke callbacks concurrently
