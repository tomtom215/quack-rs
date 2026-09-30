# TLS Configuration

Extensions that make outbound HTTPS connections (e.g., fetching remote data,
calling REST APIs) need a way to inject TLS configuration — client certificates
for mTLS, custom CA bundles, or restricted cipher suites.

The [`tls`](https://docs.rs/quack-rs/latest/quack_rs/tls/index.html) module
provides the [`TlsConfigProvider`](https://docs.rs/quack-rs/latest/quack_rs/tls/trait.TlsConfigProvider.html) trait so that extensions can supply their TLS
setup through a uniform interface, regardless of which TLS library they use
(`rustls`, `native-tls`, etc.).

## Design

The trait is **type-erased** via `Arc<dyn Any + Send + Sync>` so that `quack-rs`
does not depend on any specific TLS library. The code that consumes the provider
downcasts the returned `Arc` to the concrete config type, after checking
`config_type_name()`. `TlsConfigProvider` requires `Send + Sync`.

## Implementing a TLS Provider

```rust
use quack_rs::tls::{TlsConfigProvider, TlsVersion};
use quack_rs::error::ExtensionError;
use std::any::Any;
use std::sync::Arc;

struct MyTlsProvider {
    // In practice: Arc<rustls::ClientConfig>
    config: Arc<String>,
    mtls_enabled: bool,
}

impl TlsConfigProvider for MyTlsProvider {
    fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
        Ok(self.config.clone())
    }

    fn provider_name(&self) -> &str { "my-extension-tls" }
    fn config_type_name(&self) -> &str { "String" } // in practice: "rustls::ClientConfig"

    fn min_tls_version(&self) -> TlsVersion {
        TlsVersion::Tls12  // Minimum recommended
    }

    fn supports_mtls(&self) -> bool { self.mtls_enabled }

    fn accepts_invalid_certs(&self) -> bool {
        false  // MUST default to false
    }
}
```

## Security Requirements

Implementations **must**:

- Return `false` from `accepts_invalid_certs()` unless explicitly configured
  otherwise by the user. Certificate validation bypass (CWE-295) should never be
  the default.
- Return `TlsVersion::Tls12` or higher from `min_tls_version()`. TLS 1.0 and
  1.1 are deprecated per [RFC 8996](https://datatracker.ietf.org/doc/html/rfc8996).
- Emit an `ExtensionWarning` via `WarningCollector` when certificate validation
  is disabled or when using a TLS version below 1.2.

## Auditing a Provider

`audit_tls_provider()` checks a provider for two common misconfigurations and
returns one `ExtensionWarning` per problem found:

- Certificate verification bypass (CWE-295): code `TLS_NO_VERIFY`, severity
  `High`
- A deprecated minimum version, TLS 1.0 or 1.1 (CWE-327): code
  `TLS_DEPRECATED_VERSION`, severity `Medium`

Feed the result into a `WarningCollector`:

```rust
use quack_rs::error::ExtensionError;
use quack_rs::tls::{audit_tls_provider, TlsConfigProvider, TlsVersion};
use quack_rs::warning::WarningCollector;
use std::any::Any;
use std::sync::Arc;

struct InsecureProvider;

impl TlsConfigProvider for InsecureProvider {
    fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
        Ok(Arc::new(()))
    }
    fn provider_name(&self) -> &str { "insecure" }
    fn config_type_name(&self) -> &str { "()" }
    fn min_tls_version(&self) -> TlsVersion { TlsVersion::Tls10 }
    fn supports_mtls(&self) -> bool { false }
    fn accepts_invalid_certs(&self) -> bool { true }
}

let collector = WarningCollector::new();
for w in audit_tls_provider(&InsecureProvider) {
    collector.emit(w);
}

let codes: Vec<&str> = collector.snapshot().iter().map(|w| w.code).collect();
assert_eq!(codes, ["TLS_NO_VERIFY", "TLS_DEPRECATED_VERSION"]);
```

## Downcasting Safely

Never use `.unwrap()` or `.expect()` when downcasting in FFI callback contexts
(see [Pitfall L3](../reference/pitfalls.md#l3-no-panic-across-ffi-boundaries)).
Handle a failed downcast as an error:

```rust
use std::any::Any;
use std::sync::Arc;
use quack_rs::error::ExtensionError;

// With rustls this would be `downcast_ref::<rustls::ClientConfig>()`.
fn use_config(config: &Arc<dyn Any + Send + Sync>) -> Result<(), ExtensionError> {
    let text = config
        .downcast_ref::<String>()
        .ok_or_else(|| ExtensionError::new("expected a String TLS config"))?;
    let _ = text;
    Ok(())
}

let config: Arc<dyn Any + Send + Sync> = Arc::new(String::from("pem bundle"));
assert!(use_config(&config).is_ok());
let wrong: Arc<dyn Any + Send + Sync> = Arc::new(42_u32);
assert!(use_config(&wrong).is_err());
```
