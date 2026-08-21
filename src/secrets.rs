// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Credential handling for extensions.
//!
//! Extensions that access external services (HTTP APIs, databases, cloud
//! storage) commonly need credentials.
//!
//! # What `DuckDB` does and does not give you
//!
//! `DuckDB` has a native secrets system (`CREATE SECRET`), but **the extension
//! C API exposes none of it** — there is not one `duckdb_secret_*` function
//! among the 546 slots of `duckdb_ext_api_v1` in `DuckDB` 1.5.5. An extension
//! cannot ask `DuckDB` for a credential through the C API at all.
//!
//! What it *can* do is query the `duckdb_secrets()` table function, and
//! [`list_duckdb_secrets`] does exactly that. But that only gets you metadata:
//! `DuckDB` **redacts** sensitive fields there. Verified against `DuckDB` 1.5.5:
//!
//! ```text
//! CREATE SECRET s (TYPE s3, KEY_ID 'AKIAEXAMPLE', SECRET 'super-secret-value');
//! SELECT secret_string FROM duckdb_secrets();
//! -- ...;key_id=AKIAEXAMPLE;secret=redacted
//! ```
//!
//! So an extension that needs the credential itself must obtain it some other
//! way — an environment variable, a config option, a file, its own key store.
//!
//! # What this module is for
//!
//! [`SecretsManager`] is the trait an extension implements over **its own**
//! credential source, and [`SecretEntry`] is what that source returns. It is
//! not a bridge to `DuckDB`'s secrets, because no such bridge is available; it
//! exists so the credential handling an extension has to write anyway comes
//! with redacting `Debug`, zeroize-on-drop and no `PartialEq` already in place.
//!
//! [`list_duckdb_secrets`] complements it by reporting which secrets the user
//! *has* configured, which is enough to pick a scope, warn about a missing
//! secret, or decide which provider to use.
//!
//! # Security considerations
//!
//! `SecretEntry` is designed to minimize accidental credential leakage:
//!
//! - **`Debug` redacts field values** — only field keys are shown, values are
//!   replaced with `"[REDACTED]"`. Use [`get_field`][SecretEntry::get_field] to
//!   access actual values in code.
//! - **`Drop` zeroizes sensitive data** — every field key and value, plus
//!   `provider` and `scope`, is overwritten with zeros using
//!   [`std::ptr::write_volatile`] before deallocation, so secrets do not linger
//!   in freed memory. This covers the buffers a [`SecretEntry`] owns; it cannot
//!   cover a `String` the caller passed in and still holds, nor a buffer a
//!   `String` abandoned when it grew.
//! - **No `PartialEq`** — prevents accidental non-constant-time comparisons of
//!   secret material. Compare individual fields explicitly if needed.
//! - **`Clone` is explicit** — cloning is supported but documented so that
//!   callers are aware they are duplicating sensitive material in memory.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::secrets::{SecretEntry, SecretsManager};
//!
//! struct MySecrets {
//!     // In practice, backed by DuckDB's CREATE SECRET storage
//!     entries: Vec<SecretEntry>,
//! }
//!
//! impl SecretsManager for MySecrets {
//!     fn get_secret(&self, name: &str, secret_type: &str) -> Option<SecretEntry> {
//!         self.entries.iter()
//!             .find(|e| e.name() == name && e.secret_type() == secret_type)
//!             .cloned()
//!     }
//!
//!     fn list_secrets(&self, secret_type: Option<&str>) -> Vec<SecretEntry> {
//!         self.entries.iter()
//!             .filter(|e| secret_type.is_none() || secret_type == Some(e.secret_type()))
//!             .cloned()
//!             .collect()
//!     }
//!
//!     fn remove_secret(&self, _name: &str, _secret_type: &str) -> bool {
//!         false // read-only example
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::fmt;

use libduckdb_sys::duckdb_connection;

use crate::error::ExtensionError;

/// Metadata about one secret the user has configured in `DuckDB`.
///
/// Deliberately **not** a [`SecretEntry`]: it carries no credential material,
/// because `DuckDB` does not hand any out. `secret_string` is `DuckDB`'s own
/// rendering with sensitive fields replaced by `redacted`.
///
/// Returned by [`list_duckdb_secrets`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DuckDbSecretInfo {
    /// The secret's name, as given to `CREATE SECRET`.
    pub name: String,
    /// The secret type — `s3`, `http`, `azure`, and so on.
    pub secret_type: String,
    /// The provider that supplied it (`config`, `credential_chain`, …).
    pub provider: String,
    /// Whether the secret is persisted rather than session-scoped.
    pub persistent: bool,
    /// Where a persistent secret is stored.
    pub storage: String,
    /// The URI prefixes the secret applies to, e.g. `s3://bucket/`.
    pub scope: Vec<String>,
    /// `DuckDB`'s own rendering of the secret, **with sensitive fields
    /// redacted**. Useful for diagnostics; useless as a credential source.
    pub secret_string: String,
}

/// Lists the secrets configured in the `DuckDB` instance behind `connection`.
///
/// This queries the `duckdb_secrets()` table function, which is the only route
/// an extension has: the extension C API has no secret functions at all.
///
/// # This cannot return credentials
///
/// `DuckDB` redacts sensitive fields. A secret created as
/// `(TYPE s3, KEY_ID 'AKIA…', SECRET 'super-secret-value')` comes back as
/// `…;key_id=AKIA…;secret=redacted`. Use this to discover *which* secrets exist
/// and what they cover — to choose a scope, or to tell the user which one is
/// missing — not to authenticate with them.
///
/// # Errors
///
/// Returns [`ExtensionError`] if the query fails.
///
/// # Safety
///
/// `connection` must be a valid, open `duckdb_connection`.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::secrets::list_duckdb_secrets;
/// # use libduckdb_sys::duckdb_connection;
/// # unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// // SAFETY: `con` is a valid, open connection.
/// let secrets = unsafe { list_duckdb_secrets(con) }?;
/// if !secrets.iter().any(|s| s.secret_type == "s3") {
///     eprintln!("no S3 secret configured; run CREATE SECRET (TYPE s3, ...)");
/// }
/// # Ok(())
/// # }
/// ```
pub unsafe fn list_duckdb_secrets(
    connection: duckdb_connection,
) -> Result<Vec<DuckDbSecretInfo>, ExtensionError> {
    // `scope` is VARCHAR[]; cast it to a string here rather than walking a LIST
    // vector, then split. `list_aggregate(..., 'string_agg')` would need a
    // separator that cannot appear in a URI prefix, so the array literal syntax
    // is parsed instead — DuckDB renders it as ['a', 'b'].
    const SQL: &str = "SELECT name, type, provider, persistent, storage,                        scope::VARCHAR, secret_string FROM duckdb_secrets()";

    // SAFETY: `connection` is valid per this function's contract.
    let mut result = unsafe { crate::query::query(connection, SQL) }?;

    let mut secrets = Vec::new();
    while let Some(chunk) = result.next_chunk() {
        for row in 0..chunk.size() {
            // SAFETY: the column types are fixed by the SELECT above, and `row`
            // is within the chunk.
            unsafe {
                secrets.push(DuckDbSecretInfo {
                    name: chunk.reader(0).read_str(row).to_owned(),
                    secret_type: chunk.reader(1).read_str(row).to_owned(),
                    provider: chunk.reader(2).read_str(row).to_owned(),
                    persistent: chunk.reader(3).read_bool(row),
                    storage: chunk.reader(4).read_str(row).to_owned(),
                    scope: parse_scope_array(chunk.reader(5).read_str(row)),
                    secret_string: chunk.reader(6).read_str(row).to_owned(),
                });
            }
        }
    }
    Ok(secrets)
}

/// Parses `DuckDB`'s rendering of a `VARCHAR[]`, e.g. `['s3://', 's3n://']`.
///
/// Returns an empty vector for `[]`, and treats anything that is not a bracketed
/// list as a single element rather than losing it.
fn parse_scope_array(rendered: &str) -> Vec<String> {
    let Some(inner) = rendered.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return if rendered.is_empty() {
            Vec::new()
        } else {
            vec![rendered.to_owned()]
        };
    };
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(", ")
        .map(|item| {
            item.strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .unwrap_or(item)
                .to_owned()
        })
        .collect()
}

/// A single secret entry retrieved from the secrets manager.
///
/// Contains the secret's metadata and key-value pairs. The `fields` map holds
/// the actual secret data (e.g., `"token"`, `"username"`, `"password"`,
/// `"client_cert_path"`).
///
/// # Security
///
/// - [`Debug`] output redacts all field values (shows keys only).
/// - [`Drop`] zeroizes all field values before deallocation.
/// - [`Clone`] is supported but creates a second copy of sensitive data in
///   memory — use sparingly and drop clones promptly.
/// - `PartialEq` / `Eq` are intentionally **not** implemented to prevent
///   accidental non-constant-time comparisons of secret material.
pub struct SecretEntry {
    name: String,
    secret_type: String,
    provider: String,
    scope: String,
    fields: HashMap<String, String>,
}

impl SecretEntry {
    /// Creates a new `SecretEntry` with the given name and type.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::secrets::SecretEntry;
    ///
    /// let entry = SecretEntry::new("my_api_key", "bearer")
    ///     .with_provider("config")
    ///     .with_field("token", "sk-abc123");
    /// assert_eq!(entry.name(), "my_api_key");
    /// assert_eq!(entry.get_field("token"), Some("sk-abc123"));
    /// ```
    #[must_use]
    pub fn new(name: impl Into<String>, secret_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            secret_type: secret_type.into(),
            provider: String::new(),
            scope: String::new(),
            fields: HashMap::new(),
        }
    }

    /// Returns the name of this secret.
    #[must_use]
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the secret type (e.g., `"bearer"`, `"s3"`, `"gcs"`).
    #[must_use]
    #[inline]
    pub fn secret_type(&self) -> &str {
        &self.secret_type
    }

    /// Returns the provider that created this secret.
    #[must_use]
    #[inline]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Returns the scope this secret applies to.
    #[must_use]
    #[inline]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Returns the field key names without exposing values.
    ///
    /// Use this for logging or diagnostics without leaking sensitive data.
    #[must_use]
    pub fn field_keys(&self) -> Vec<&str> {
        self.fields.keys().map(String::as_str).collect()
    }

    /// Sets the provider for this secret entry.
    #[must_use]
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    /// Sets the scope for this secret entry.
    #[must_use]
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Adds a key-value field to this secret entry.
    #[must_use]
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    /// Returns the value of a field, if present.
    #[must_use]
    pub fn get_field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    /// Returns the number of fields in this secret entry.
    #[must_use]
    #[inline]
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Returns `true` if this entry has no fields.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Zeroize a `String`'s buffer using volatile writes, then clear it.
///
/// Uses [`std::ptr::write_volatile`] to ensure the compiler cannot elide the
/// zeroing even if the memory is about to be freed. This is the standard
/// approach used by the `zeroize` crate, implemented inline to avoid adding
/// a dependency.
fn zeroize_string(s: &mut String) {
    // SAFETY: `as_mut_vec()` gives us mutable access to the String's backing
    // buffer. We only write `0u8` bytes, which is valid UTF-8 (NUL chars).
    // The string is cleared immediately after, so no invalid-UTF-8 state
    // is observable.
    unsafe {
        for byte in s.as_mut_vec().iter_mut() {
            std::ptr::write_volatile(byte, 0);
        }
    }
    s.clear();
}

impl SecretEntry {
    /// Overwrites every owned buffer, leaving the entry empty.
    ///
    /// This is [`Drop`]'s whole body, split out so it can be tested: a `Drop`
    /// impl's effect is only observable in memory that has already been freed,
    /// which no sound test can read.
    fn zeroize_in_place(&mut self) {
        // Zeroize all field values (the sensitive material).
        for value in self.fields.values_mut() {
            zeroize_string(value);
        }
        // Zeroize field keys too — key names can reveal what credentials exist.
        // HashMap doesn't expose mutable key access, so we drain and zeroize.
        for (mut key, mut val) in self.fields.drain() {
            zeroize_string(&mut key);
            zeroize_string(&mut val);
        }

        // Zeroize metadata fields that may contain sensitive context.
        zeroize_string(&mut self.provider);
        zeroize_string(&mut self.scope);
    }
}

impl Drop for SecretEntry {
    // One delegation. Replacing the body with `()` is unobservable from a test —
    // see `zeroize_in_place`, which carries the behaviour and is tested directly.
    #[mutants::skip]
    fn drop(&mut self) {
        self.zeroize_in_place();
    }
}

impl Clone for SecretEntry {
    /// Clones this secret entry, duplicating all sensitive material in memory.
    ///
    /// Callers should be aware that cloning creates a second copy of secret
    /// values. Drop the clone as soon as it is no longer needed to minimize
    /// the window during which sensitive data resides in memory.
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            secret_type: self.secret_type.clone(),
            provider: self.provider.clone(),
            scope: self.scope.clone(),
            fields: self.fields.clone(),
        }
    }
}

impl fmt::Debug for SecretEntry {
    /// Formats the secret entry with field values redacted.
    ///
    /// Only field keys are shown; all values are replaced with `"[REDACTED]"`.
    /// Use [`get_field`][SecretEntry::get_field] to access actual values in code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_fields: HashMap<&str, &str> = self
            .fields
            .keys()
            .map(|k| (k.as_str(), "[REDACTED]"))
            .collect();

        f.debug_struct("SecretEntry")
            .field("name", &self.name)
            .field("secret_type", &self.secret_type)
            .field("provider", &self.provider)
            .field("scope", &self.scope)
            .field("fields", &redacted_fields)
            .finish()
    }
}

impl fmt::Display for SecretEntry {
    /// Formats a human-readable summary without exposing any secret values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Secret(name={:?}, type={:?}, provider={:?}, fields={})",
            self.name,
            self.secret_type,
            self.provider,
            self.fields.len()
        )
    }
}

/// Trait for accessing `DuckDB`'s secrets management system.
///
/// Extensions implement this trait to provide a safe Rust interface over
/// `DuckDB`'s native `CREATE SECRET` / `DROP SECRET` storage. A typical
/// implementation wraps `DuckDB`'s C API or maintains an in-memory cache
/// synchronized with the `DuckDB` catalog.
///
/// # Thread safety
///
/// Implementations must be safe to call from multiple threads. `DuckDB` may
/// invoke extension callbacks concurrently.
///
/// # Security
///
/// Implementations should:
/// - Never log secret field values (use [`SecretEntry::field_keys`] for
///   diagnostics).
/// - Ensure that `remove_secret` zeroizes the secret data, not just
///   removes the reference.
/// - Minimize the lifetime of [`SecretEntry`] clones.
pub trait SecretsManager: Send + Sync {
    /// Retrieves a secret by name and type.
    ///
    /// Returns `None` if no matching secret exists.
    fn get_secret(&self, name: &str, secret_type: &str) -> Option<SecretEntry>;

    /// Lists all secrets, optionally filtered by type.
    ///
    /// If `secret_type` is `None`, all secrets are returned.
    ///
    /// # Security note
    ///
    /// The returned entries contain full secret values. Callers should avoid
    /// storing or logging the result. For diagnostics, iterate and use
    /// [`SecretEntry::field_keys`] instead of [`SecretEntry::get_field`].
    fn list_secrets(&self, secret_type: Option<&str>) -> Vec<SecretEntry>;

    /// Removes a secret by name and type.
    ///
    /// Returns `true` if the secret was found and removed, `false` otherwise.
    /// Implementations should zeroize the secret data before deallocation.
    fn remove_secret(&self, name: &str, secret_type: &str) -> bool;
}

#[cfg(test)]
mod tests {
    // `parse_scope_array` reads DuckDB's rendering of a VARCHAR[] out of
    // `duckdb_secrets()`. It is pure string handling and the only part of this
    // module that does not need a database, so pin its shape here rather than
    // relying on whatever a live secret happens to carry.

    #[test]
    fn a_scope_array_round_trips_its_elements() {
        assert_eq!(
            parse_scope_array("['s3://', 's3n://']"),
            vec!["s3://".to_owned(), "s3n://".to_owned()]
        );
        assert_eq!(parse_scope_array("['only']"), vec!["only".to_owned()]);
    }

    #[test]
    fn an_empty_scope_array_yields_no_elements() {
        assert!(parse_scope_array("[]").is_empty());
        assert!(parse_scope_array("[ ]").is_empty());
        assert!(parse_scope_array("").is_empty());
    }

    #[test]
    fn an_unbracketed_scope_is_kept_as_a_single_element_rather_than_lost() {
        assert_eq!(parse_scope_array("s3://"), vec!["s3://".to_owned()]);
    }

    #[test]
    fn scope_elements_keep_their_contents_when_unquoted() {
        // DuckDB quotes each element; anything that is not quoted is taken
        // verbatim rather than having its first and last character stripped.
        assert_eq!(parse_scope_array("[bare]"), vec!["bare".to_owned()]);
    }

    use super::*;

    #[test]
    fn secret_entry_builder() {
        let entry = SecretEntry::new("test", "bearer")
            .with_provider("config")
            .with_scope("https://api.example.com")
            .with_field("token", "abc123")
            .with_field("refresh_token", "xyz789");

        assert_eq!(entry.name(), "test");
        assert_eq!(entry.secret_type(), "bearer");
        assert_eq!(entry.provider(), "config");
        assert_eq!(entry.scope(), "https://api.example.com");
        assert_eq!(entry.get_field("token"), Some("abc123"));
        assert_eq!(entry.get_field("refresh_token"), Some("xyz789"));
        assert_eq!(entry.get_field("nonexistent"), None);
        assert_eq!(entry.field_count(), 2);
        assert!(!entry.is_empty());
    }

    #[test]
    fn zeroizing_an_entry_clears_every_buffer_it_owns() {
        // `Drop` runs exactly this, and the crate advertises zeroize-on-drop as
        // a security property — so it is worth an assertion rather than a
        // reading of the code.
        let mut entry = SecretEntry::new("test", "bearer")
            .with_provider("config")
            .with_scope("https://api.example.com")
            .with_field("token", "abc123");

        entry.zeroize_in_place();

        assert_eq!(entry.field_count(), 0, "fields must be drained");
        assert_eq!(entry.get_field("token"), None);
        assert_eq!(entry.provider(), "", "provider must be overwritten");
        assert_eq!(entry.scope(), "", "scope must be overwritten");
    }

    #[test]
    fn secret_entry_new_defaults() {
        let entry = SecretEntry::new("s", "s3");
        assert_eq!(entry.provider(), "");
        assert_eq!(entry.scope(), "");
        assert!(entry.is_empty());
        assert_eq!(entry.field_count(), 0);
    }

    #[test]
    #[allow(clippy::redundant_clone)]
    fn secret_entry_clone() {
        let e1 = SecretEntry::new("a", "b").with_field("k", "v");
        let e2 = e1.clone();
        assert_eq!(e2.name(), "a");
        assert_eq!(e2.get_field("k"), Some("v"));
    }

    #[test]
    fn debug_redacts_field_values() {
        let entry =
            SecretEntry::new("api_key", "bearer").with_field("token", "super-secret-value-12345");

        let debug_output = format!("{entry:?}");

        // The actual secret value must NOT appear in debug output.
        assert!(
            !debug_output.contains("super-secret-value-12345"),
            "Debug output must not contain secret values: {debug_output}"
        );
        // The field key SHOULD appear (it's metadata, not the secret).
        assert!(
            debug_output.contains("token"),
            "Debug should show field keys"
        );
        // The redaction marker should appear.
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug should show [REDACTED] for values"
        );
    }

    #[test]
    fn display_does_not_leak_values() {
        let entry =
            SecretEntry::new("api_key", "bearer").with_field("token", "super-secret-value-12345");

        let display_output = format!("{entry}");

        assert!(
            !display_output.contains("super-secret-value-12345"),
            "Display must not contain secret values: {display_output}"
        );
        assert!(
            display_output.contains("api_key"),
            "Display should show the secret name"
        );
    }

    #[test]
    fn field_keys_returns_keys_only() {
        let entry = SecretEntry::new("x", "y")
            .with_field("token", "secret1")
            .with_field("key_id", "secret2");

        let keys = entry.field_keys();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"token"));
        assert!(keys.contains(&"key_id"));
    }

    #[test]
    fn drop_zeroizes_field_values() {
        // We cannot observe memory after drop, but we can verify that
        // `zeroize_string` really overwrote the buffer rather than just
        // shortening the string.
        let mut s = String::from("sensitive-data-here");
        let len = s.len();

        zeroize_string(&mut s);
        assert!(s.is_empty(), "String should be empty after zeroize");

        // Look at the buffer *through the String*, after the mutation. Taking
        // `s.as_ptr()` beforehand and reading it here would be a stale
        // borrow: `zeroize_string`'s `&mut` retag invalidates it, which Miri
        // reports as undefined behaviour under Stacked Borrows and which the
        // compiler is free to reorder around.
        //
        // `clear()` does not deallocate, so bytes `0..len` are still allocated
        // and still initialised — by zeroes, if the function did its job.
        // SAFETY: `len <= capacity` (the string only shrank), and every byte in
        // that range was initialised before `clear()` and overwritten with 0,
        // which is valid UTF-8. `set_len(0)` restores the invariant before the
        // borrow ends.
        unsafe {
            let buf = s.as_mut_vec();
            assert!(len <= buf.capacity());
            buf.set_len(len);
            assert!(
                buf.iter().all(|&b| b == 0),
                "All bytes should be zero after zeroize"
            );
            buf.set_len(0);
        }
    }

    #[test]
    fn zeroize_empty_string_is_safe() {
        let mut s = String::new();
        zeroize_string(&mut s);
        assert!(s.is_empty());
    }

    struct InMemorySecrets {
        entries: Vec<SecretEntry>,
    }

    impl SecretsManager for InMemorySecrets {
        fn get_secret(&self, name: &str, secret_type: &str) -> Option<SecretEntry> {
            self.entries
                .iter()
                .find(|e| e.name() == name && e.secret_type() == secret_type)
                .cloned()
        }

        fn list_secrets(&self, secret_type: Option<&str>) -> Vec<SecretEntry> {
            self.entries
                .iter()
                .filter(|e| secret_type.is_none() || secret_type == Some(e.secret_type()))
                .cloned()
                .collect()
        }

        fn remove_secret(&self, _name: &str, _secret_type: &str) -> bool {
            false
        }
    }

    #[test]
    fn in_memory_secrets_manager() {
        let mgr = InMemorySecrets {
            entries: vec![
                SecretEntry::new("api_key", "bearer").with_field("token", "t1"),
                SecretEntry::new("bucket", "s3").with_field("key_id", "k1"),
            ],
        };

        assert!(mgr.get_secret("api_key", "bearer").is_some());
        assert!(mgr.get_secret("api_key", "s3").is_none());
        assert!(mgr.get_secret("missing", "bearer").is_none());

        let all = mgr.list_secrets(None);
        assert_eq!(all.len(), 2);

        let s3_only = mgr.list_secrets(Some("s3"));
        assert_eq!(s3_only.len(), 1);
        assert_eq!(s3_only[0].name(), "bucket");
    }

    #[test]
    fn secrets_manager_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InMemorySecrets>();
    }

    #[test]
    fn secret_entry_empty_name_and_type() {
        let entry = SecretEntry::new("", "");
        assert_eq!(entry.name(), "");
        assert_eq!(entry.secret_type(), "");
    }

    #[test]
    fn with_field_overwrites_existing_key() {
        let entry = SecretEntry::new("s", "t")
            .with_field("token", "old-value")
            .with_field("token", "new-value");
        assert_eq!(entry.get_field("token"), Some("new-value"));
        assert_eq!(entry.field_count(), 1);
    }

    #[test]
    fn debug_redacts_empty_field_value() {
        let entry = SecretEntry::new("s", "t").with_field("key", "");
        let debug = format!("{entry:?}");
        // Even empty field values must be redacted in debug output
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("key"));
    }

    #[test]
    fn display_shows_field_count_not_values() {
        let entry = SecretEntry::new("s", "t")
            .with_field("a", "secret1")
            .with_field("b", "secret2")
            .with_field("c", "secret3");
        let display = format!("{entry}");
        assert!(display.contains("fields=3"));
        assert!(!display.contains("secret1"));
        assert!(!display.contains("secret2"));
        assert!(!display.contains("secret3"));
    }

    /// Reads back the bytes `zeroize_string` wrote.
    ///
    /// Every pointer into the buffer is derived **after** `zeroize_string`
    /// returns. Taking one beforehand — from `&s` *or* from `&mut s` — and
    /// reading it afterwards is undefined behaviour under Stacked Borrows,
    /// because the `&mut` retag inside `zeroize_string` pops the earlier tag off
    /// the borrow stack. Miri catches exactly this; the CI `miri` job exists
    /// because two earlier versions of this helper did not.
    fn zeroize_and_read_back(text: &str) -> Vec<u8> {
        let mut s = String::from(text);
        let len = s.len();
        zeroize_string(&mut s);
        assert!(s.is_empty());
        assert!(s.capacity() >= len, "the buffer must not have been freed");
        // SAFETY: `clear()` sets the length to zero without deallocating, so
        // bytes `0..len` are still allocated and still initialised — with
        // zeroes, if the function did its job. NUL is valid UTF-8, and
        // `set_len(0)` restores the String's invariant before the borrow ends.
        let bytes = unsafe {
            let buf = s.as_mut_vec();
            buf.set_len(len);
            let copy = buf.clone();
            buf.set_len(0);
            copy
        };
        drop(s);
        bytes
    }

    #[test]
    fn zeroize_string_with_special_characters() {
        let bytes = zeroize_and_read_back("p@$$w0rd!#%^&*()_+-=[]{}|;':\",./<>?");
        assert!(!bytes.is_empty());
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn zeroize_string_with_unicode() {
        let bytes = zeroize_and_read_back("pässwörd🔑秘密");
        assert!(bytes.len() > "pässwörd🔑秘密".chars().count(), "multi-byte");
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn drop_zeroizes_metadata_fields() {
        // Verify that provider and scope are zeroized on drop
        let mut entry = SecretEntry::new("name", "type")
            .with_provider("my-provider")
            .with_scope("https://example.com");

        // Zeroize directly to test
        zeroize_string(&mut entry.provider);
        assert!(entry.provider.is_empty());
        zeroize_string(&mut entry.scope);
        assert!(entry.scope.is_empty());
    }

    #[test]
    fn list_secrets_with_no_matching_type() {
        let mgr = InMemorySecrets {
            entries: vec![SecretEntry::new("a", "bearer").with_field("token", "t1")],
        };
        let result = mgr.list_secrets(Some("s3"));
        assert!(result.is_empty());
    }

    #[test]
    fn remove_secret_returns_false() {
        let mgr = InMemorySecrets {
            entries: vec![SecretEntry::new("a", "b")],
        };
        assert!(!mgr.remove_secret("a", "b"));
    }

    #[test]
    fn secret_entry_many_fields() {
        let mut entry = SecretEntry::new("s", "t");
        for i in 0..100 {
            entry = entry.with_field(format!("key_{i}"), format!("value_{i}"));
        }
        assert_eq!(entry.field_count(), 100);
        assert_eq!(entry.get_field("key_50"), Some("value_50"));
        assert_eq!(entry.field_keys().len(), 100);
    }
}
