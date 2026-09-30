# Secrets Management

Extensions that access external services (HTTP APIs, databases, cloud storage)
commonly need credentials. DuckDB has a native secrets system (`CREATE SECRET`),
but the extension C API exposes none of it: `duckdb_ext_api_v1` has no
`duckdb_secret_*` functions, so an extension cannot ask DuckDB for a credential.

The [`secrets`](https://docs.rs/quack-rs/latest/quack_rs/secrets/index.html)
module therefore offers two separate things:

- [`list_duckdb_secrets`](#listing-duckdbs-secrets) reads the *metadata* of the
  secrets the user has configured, with sensitive fields redacted by DuckDB.
- `SecretsManager` and `SecretEntry` are a trait and a type for the credential
  source an extension has to provide itself (an environment variable, a config
  option, a file, its own key store), with leak-resistant defaults already in
  place.

## Listing DuckDB's secrets

`list_duckdb_secrets` queries the `duckdb_secrets()` table function and returns
one `DuckDbSecretInfo` per secret: `name`, `secret_type`, `provider`,
`persistent`, `storage`, `scope` (the URI prefixes it applies to) and
`secret_string`. DuckDB redacts sensitive fields in that table, so a secret
created with `SECRET 'super-secret-value'` comes back as `...;secret=redacted`.
Use it to find out which secrets exist and what they cover, not to
authenticate:

```rust,no_run
use quack_rs::secrets::list_duckdb_secrets;
# use libduckdb_sys::duckdb_connection;
# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
// SAFETY: `con` is a valid, open connection.
let secrets = unsafe { list_duckdb_secrets(con) }?;
if !secrets.iter().any(|s| s.secret_type == "s3") {
    eprintln!("no S3 secret configured; run CREATE SECRET (TYPE s3, ...)");
}
# Ok(())
# }
```

## Core Types

### `SecretEntry`

A single secret entry with metadata and key-value fields. Designed to minimize
accidental credential leakage:

- **`Debug` redacts field values and the scope** — field keys are shown, and
  every value and a non-empty scope are replaced with `"[REDACTED]"`
- **`Drop` zeroizes sensitive data** — every field key and value, the provider
  and the scope are overwritten with zeros using `std::ptr::write_volatile`
  before deallocation. This covers the buffers a `SecretEntry` owns, not a
  `String` the caller passed in and still holds
- **No `PartialEq`** — prevents accidental non-constant-time comparisons of
  secret material
- **`Clone` duplicates the secret** — it is supported, but each clone is
  another copy of the credential in memory

### `SecretsManager`

The trait an extension implements over its own credential source. It requires
`Send + Sync`:

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
let mut keys = entry.field_keys();
keys.sort_unstable(); // the fields are a HashMap, so the order is unspecified
assert_eq!(keys, ["access_key", "secret_key"]);
```

## Debug Output

The `Debug` implementation redacts field values and the scope. For an entry
with a scope and one field, `{:#?}` prints:

```text
SecretEntry {
    name: "api_key",
    secret_type: "bearer",
    provider: "config",
    scope: "[REDACTED]",
    fields: {
        "token": "[REDACTED]",
    },
}
```

## Security Best Practices

1. **Never log secret field values** — use `field_keys()` for diagnostics
2. **Drop clones promptly** — minimize the window during which sensitive data
   resides in memory
3. **Implement `remove_secret` with zeroization** — don't just remove the
   reference; zeroize the data before deallocation
4. **Thread safety** — `SecretsManager` requires `Send + Sync`, because DuckDB
   may run the callbacks that consult it on several threads at once
