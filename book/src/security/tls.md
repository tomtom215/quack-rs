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
does not depend on any specific TLS library. The implementing extension downcasts
the returned `Arc` to its concrete config type.

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
    fn config_type_name(&self) -> &str { "rustls::ClientConfig" }

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

The `audit_tls_provider()` function checks common misconfigurations:

```rust,no_run
use quack_rs::tls::audit_tls_provider;
use quack_rs::warning::WarningCollector;

// let warnings = audit_tls_provider(&my_provider);
// let collector = WarningCollector::new();
// for w in warnings {
//     collector.emit(w);
// }
```

It detects:
- Certificate verification bypass (CWE-295) — emits `TLS_NO_VERIFY` warning
- Deprecated TLS versions (CWE-327) — emits `TLS_DEPRECATED_VERSION` warning

## Downcasting Safely

Never use `.unwrap()` or `.expect()` when downcasting in FFI callback contexts
(see Pitfall L3). Always handle the `None` case gracefully:

```rust,no_run
use std::any::Any;
use std::sync::Arc;
use quack_rs::error::ExtensionError;

fn use_config(config: Arc<dyn Any + Send + Sync>) -> Result<(), ExtensionError> {
    // let rustls_config = config.downcast_ref::<rustls::ClientConfig>()
    //     .ok_or(ExtensionError::new("expected rustls::ClientConfig"))?;
    Ok(())
}
```
