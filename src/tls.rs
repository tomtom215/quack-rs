// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Type-erased TLS configuration provider for HTTP-capable extensions.
//!
//! Extensions that make outbound HTTPS connections (e.g., `duck_net`) need a way
//! to inject custom TLS configurations — client certificates for mTLS, custom CA
//! bundles, or restricted cipher suites. This module provides the
//! [`TlsConfigProvider`] trait so that extensions can supply their TLS setup
//! through a uniform interface, regardless of which TLS library they use.
//!
//! # Design
//!
//! The trait is **type-erased** via [`std::any::Any`] so that `quack-rs` does not
//! depend on any specific TLS library (e.g., `rustls`, `native-tls`). The
//! implementing extension downcasts the returned `Arc<dyn Any>` to its concrete
//! config type.
//!
//! # Security requirements for implementors
//!
//! Implementations **must**:
//!
//! - Return `false` from [`accepts_invalid_certs`][TlsConfigProvider::accepts_invalid_certs]
//!   unless explicitly configured otherwise by the user. Certificate validation
//!   bypass (CWE-295) should never be the default.
//! - Return [`TlsVersion::Tls12`] or higher from
//!   [`min_tls_version`][TlsConfigProvider::min_tls_version]. TLS 1.0 and 1.1
//!   are deprecated ([RFC 8996](https://datatracker.ietf.org/doc/html/rfc8996)).
//! - Emit an [`ExtensionWarning`][crate::warning::ExtensionWarning] via
//!   [`WarningCollector`][crate::warning::WarningCollector] when certificate
//!   validation is disabled or when using a TLS version below 1.2.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::tls::{TlsConfigProvider, TlsVersion};
//! use quack_rs::error::ExtensionError;
//! use std::any::Any;
//! use std::sync::Arc;
//!
//! struct MyTlsProvider {
//!     // In practice: Arc<rustls::ClientConfig>
//!     config: Arc<String>,
//!     mtls_enabled: bool,
//! }
//!
//! impl TlsConfigProvider for MyTlsProvider {
//!     fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
//!         Ok(self.config.clone())
//!     }
//!
//!     fn provider_name(&self) -> &str {
//!         "my-extension-tls"
//!     }
//!
//!     fn config_type_name(&self) -> &str {
//!         "String" // In practice: "rustls::ClientConfig"
//!     }
//!
//!     fn min_tls_version(&self) -> TlsVersion {
//!         TlsVersion::Tls12
//!     }
//!
//!     fn supports_mtls(&self) -> bool {
//!         self.mtls_enabled
//!     }
//!
//!     fn accepts_invalid_certs(&self) -> bool {
//!         false
//!     }
//! }
//! ```

use std::any::Any;
use std::sync::Arc;

use crate::error::ExtensionError;

/// Minimum TLS protocol version supported by a [`TlsConfigProvider`].
///
/// Used for security auditing and warning generation. Providers that allow
/// versions below [`Tls12`][TlsVersion::Tls12] should emit a warning via
/// [`WarningCollector`][crate::warning::WarningCollector].
///
/// `#[non_exhaustive]`: a later TLS version may be added, so a `match`
/// outside quack-rs needs a wildcard arm; comparisons (`>=
/// TlsVersion::Tls12`) keep working.
///
/// ```rust,compile_fail,E0004
/// use quack_rs::tls::TlsVersion;
///
/// fn modern(version: TlsVersion) -> bool {
///     match version {
///         TlsVersion::Tls10 | TlsVersion::Tls11 => false,
///         TlsVersion::Tls12 | TlsVersion::Tls13 => true,
///         // error[E0004]: a wildcard arm is required
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TlsVersion {
    /// TLS 1.0 — **deprecated** per [RFC 8996](https://datatracker.ietf.org/doc/html/rfc8996).
    Tls10,
    /// TLS 1.1 — **deprecated** per [RFC 8996](https://datatracker.ietf.org/doc/html/rfc8996).
    Tls11,
    /// TLS 1.2 — minimum recommended version.
    Tls12,
    /// TLS 1.3 — preferred version.
    Tls13,
}

impl TlsVersion {
    /// Returns `true` if this version is considered deprecated (TLS 1.0 or 1.1).
    #[must_use]
    #[inline]
    pub const fn is_deprecated(self) -> bool {
        matches!(self, Self::Tls10 | Self::Tls11)
    }
}

impl std::fmt::Display for TlsVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tls10 => f.write_str("TLS 1.0"),
            Self::Tls11 => f.write_str("TLS 1.1"),
            Self::Tls12 => f.write_str("TLS 1.2"),
            Self::Tls13 => f.write_str("TLS 1.3"),
        }
    }
}

/// A type-erased provider of TLS client configuration.
///
/// Extensions that make outbound HTTPS connections implement this trait to
/// supply their TLS setup (client certificates, custom CA bundles, etc.)
/// without coupling `quack-rs` to a specific TLS library.
///
/// # Downcasting
///
/// The returned `Arc<dyn Any + Send + Sync>` should be downcast by the
/// consumer to the concrete config type. Use [`config_type_name`][Self::config_type_name]
/// to verify the expected type before downcasting, and handle `None` from
/// `downcast_ref` gracefully (never use `.expect()` or
/// `.unwrap()` in FFI callback contexts — see Pitfall L3).
///
/// ```rust,no_run
/// use std::any::Any;
/// use std::sync::Arc;
/// use quack_rs::error::ExtensionError;
///
/// fn use_config(config: Arc<dyn Any + Send + Sync>) -> Result<(), ExtensionError> {
///     // let rustls_config = config.downcast_ref::<rustls::ClientConfig>()
///     //     .ok_or(ExtensionError::new("expected rustls::ClientConfig"))?;
///     Ok(())
/// }
/// ```
///
/// # Security
///
/// See the [module-level documentation][crate::tls] for security requirements.
/// Implementations that bypass certificate validation or allow deprecated TLS
/// versions should clearly document this and emit warnings.
pub trait TlsConfigProvider: Send + Sync {
    /// Returns the TLS client configuration as a type-erased `Arc`.
    ///
    /// Implementations should return an `Arc` wrapping their concrete TLS
    /// config type (e.g., `Arc<rustls::ClientConfig>`).
    ///
    /// # Errors
    ///
    /// Returns an [`ExtensionError`] if the configuration cannot be created
    /// (e.g., certificate file not found, invalid key format, expired CA).
    fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError>;

    /// Returns a human-readable name for this TLS provider.
    ///
    /// Used in diagnostics, warning messages, and error context.
    fn provider_name(&self) -> &str;

    /// Returns the concrete type name of the config returned by [`client_config`][Self::client_config].
    ///
    /// This allows consumers to verify they have the right provider before
    /// attempting a downcast. For example, a `rustls`-based provider would
    /// return `"rustls::ClientConfig"`.
    fn config_type_name(&self) -> &str;

    /// Returns the minimum TLS protocol version this config allows.
    ///
    /// Implementations should return [`TlsVersion::Tls12`] or higher.
    /// Returning a deprecated version ([`TlsVersion::Tls10`] or
    /// [`TlsVersion::Tls11`]) should trigger a warning via
    /// [`WarningCollector`][crate::warning::WarningCollector].
    fn min_tls_version(&self) -> TlsVersion;

    /// Returns `true` if this config includes a client certificate for mTLS.
    fn supports_mtls(&self) -> bool;

    /// Returns `true` if this config accepts invalid (self-signed, expired,
    /// or hostname-mismatched) server certificates.
    ///
    /// # Security
    ///
    /// This should return `false` by default. Returning `true` disables
    /// server certificate validation (CWE-295: Improper Certificate
    /// Validation) and should **only** be enabled when explicitly requested
    /// by the user (e.g., via a `SET` variable or connection parameter).
    ///
    /// When this returns `true`, the extension should emit an
    /// [`ExtensionWarning`][crate::warning::ExtensionWarning] with
    /// `code: "TLS_NO_VERIFY"`, `severity: High`, and `cwe: Some(295)`.
    fn accepts_invalid_certs(&self) -> bool;
}

/// Validates a [`TlsConfigProvider`] and returns a list of security concerns.
///
/// This is a convenience function that checks common misconfigurations and
/// returns human-readable warning messages. Extensions can call this during
/// initialization and feed the results into a
/// [`WarningCollector`][crate::warning::WarningCollector].
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::tls::audit_tls_provider;
/// use quack_rs::warning::{WarningCollector, ExtensionWarning, WarningSeverity};
///
/// // let warnings = audit_tls_provider(&my_provider);
/// // let collector = WarningCollector::new();
/// // for w in warnings {
/// //     collector.emit(w);
/// // }
/// ```
#[must_use]
pub fn audit_tls_provider(
    provider: &dyn TlsConfigProvider,
) -> Vec<crate::warning::ExtensionWarning> {
    let mut warnings = Vec::new();

    if provider.accepts_invalid_certs() {
        warnings.push(crate::warning::ExtensionWarning {
            code: "TLS_NO_VERIFY",
            severity: crate::warning::WarningSeverity::High,
            message: format!(
                "TLS provider {:?} has certificate verification disabled",
                provider.provider_name()
            ),
            cwe: Some(295),
        });
    }

    let min_version = provider.min_tls_version();
    if min_version.is_deprecated() {
        warnings.push(crate::warning::ExtensionWarning {
            code: "TLS_DEPRECATED_VERSION",
            severity: crate::warning::WarningSeverity::Medium,
            message: format!(
                "TLS provider {:?} allows deprecated {} (RFC 8996)",
                provider.provider_name(),
                min_version,
            ),
            cwe: Some(327), // CWE-327: Use of a Broken or Risky Cryptographic Algorithm
        });
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SecureProvider {
        config: Arc<String>,
    }

    #[allow(clippy::unnecessary_literal_bound)]
    impl TlsConfigProvider for SecureProvider {
        fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
            Ok(self.config.clone())
        }

        fn provider_name(&self) -> &str {
            "test-secure"
        }

        fn config_type_name(&self) -> &str {
            "String"
        }

        fn min_tls_version(&self) -> TlsVersion {
            TlsVersion::Tls12
        }

        fn supports_mtls(&self) -> bool {
            false
        }

        fn accepts_invalid_certs(&self) -> bool {
            false
        }
    }

    struct InsecureProvider;

    #[allow(clippy::unnecessary_literal_bound)]
    impl TlsConfigProvider for InsecureProvider {
        fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
            Ok(Arc::new(42u32))
        }

        fn provider_name(&self) -> &str {
            "test-insecure"
        }

        fn config_type_name(&self) -> &str {
            "u32"
        }

        fn min_tls_version(&self) -> TlsVersion {
            TlsVersion::Tls10
        }

        fn supports_mtls(&self) -> bool {
            false
        }

        fn accepts_invalid_certs(&self) -> bool {
            true
        }
    }

    struct FailingProvider;

    #[allow(clippy::unnecessary_literal_bound)]
    impl TlsConfigProvider for FailingProvider {
        fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
            Err(ExtensionError::new("certificate file not found"))
        }

        fn provider_name(&self) -> &str {
            "test-failing"
        }

        fn config_type_name(&self) -> &str {
            "never"
        }

        fn min_tls_version(&self) -> TlsVersion {
            TlsVersion::Tls13
        }

        fn supports_mtls(&self) -> bool {
            true
        }

        fn accepts_invalid_certs(&self) -> bool {
            false
        }
    }

    #[test]
    fn secure_provider_returns_config() {
        let provider = SecureProvider {
            config: Arc::new("test-config".to_string()),
        };
        let config = provider.client_config().unwrap();
        let s = config.downcast_ref::<String>().unwrap();
        assert_eq!(s, "test-config");
    }

    #[test]
    fn secure_provider_metadata() {
        let provider = SecureProvider {
            config: Arc::new(String::new()),
        };
        assert_eq!(provider.provider_name(), "test-secure");
        assert_eq!(provider.config_type_name(), "String");
        assert_eq!(provider.min_tls_version(), TlsVersion::Tls12);
        assert!(!provider.supports_mtls());
        assert!(!provider.accepts_invalid_certs());
    }

    #[test]
    fn failing_provider_returns_error() {
        let provider = FailingProvider;
        let err = provider.client_config().unwrap_err();
        assert_eq!(err.as_str(), "certificate file not found");
        assert!(provider.supports_mtls());
    }

    #[test]
    fn downcast_wrong_type_returns_none() {
        let provider = SecureProvider {
            config: Arc::new("hello".to_string()),
        };
        let config = provider.client_config().unwrap();
        // Trying to downcast String to u32 should return None, not panic.
        assert!(config.downcast_ref::<u32>().is_none());
    }

    #[test]
    fn audit_secure_provider_no_warnings() {
        let provider = SecureProvider {
            config: Arc::new(String::new()),
        };
        let warnings = audit_tls_provider(&provider);
        assert!(warnings.is_empty());
    }

    #[test]
    fn audit_insecure_provider_flags_issues() {
        let provider = InsecureProvider;
        let warnings = audit_tls_provider(&provider);

        assert_eq!(warnings.len(), 2);

        // Should flag invalid cert acceptance (CWE-295).
        let cert_warning = warnings.iter().find(|w| w.code == "TLS_NO_VERIFY");
        assert!(cert_warning.is_some());
        assert_eq!(cert_warning.unwrap().cwe, Some(295));
        assert_eq!(
            cert_warning.unwrap().severity,
            crate::warning::WarningSeverity::High
        );

        // Should flag deprecated TLS version (CWE-327).
        let version_warning = warnings.iter().find(|w| w.code == "TLS_DEPRECATED_VERSION");
        assert!(version_warning.is_some());
        assert_eq!(version_warning.unwrap().cwe, Some(327));
        assert!(version_warning.unwrap().message.contains("TLS 1.0"));
    }

    #[test]
    fn tls_version_ordering() {
        assert!(TlsVersion::Tls10 < TlsVersion::Tls11);
        assert!(TlsVersion::Tls11 < TlsVersion::Tls12);
        assert!(TlsVersion::Tls12 < TlsVersion::Tls13);
    }

    #[test]
    fn tls_version_deprecated() {
        assert!(TlsVersion::Tls10.is_deprecated());
        assert!(TlsVersion::Tls11.is_deprecated());
        assert!(!TlsVersion::Tls12.is_deprecated());
        assert!(!TlsVersion::Tls13.is_deprecated());
    }

    #[test]
    fn tls_version_display() {
        assert_eq!(TlsVersion::Tls10.to_string(), "TLS 1.0");
        assert_eq!(TlsVersion::Tls11.to_string(), "TLS 1.1");
        assert_eq!(TlsVersion::Tls12.to_string(), "TLS 1.2");
        assert_eq!(TlsVersion::Tls13.to_string(), "TLS 1.3");
    }

    #[test]
    fn provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SecureProvider>();
    }

    #[test]
    fn trait_object_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn TlsConfigProvider>();
    }

    #[test]
    fn tls11_is_deprecated() {
        // TLS 1.1 must also be flagged as deprecated (not just TLS 1.0)
        struct Tls11Provider;

        #[allow(clippy::unnecessary_literal_bound)]
        impl TlsConfigProvider for Tls11Provider {
            fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
                Ok(Arc::new(()))
            }
            fn provider_name(&self) -> &str {
                "tls11-test"
            }
            fn config_type_name(&self) -> &str {
                "()"
            }
            fn min_tls_version(&self) -> TlsVersion {
                TlsVersion::Tls11
            }
            fn supports_mtls(&self) -> bool {
                false
            }
            fn accepts_invalid_certs(&self) -> bool {
                false
            }
        }

        let warnings = audit_tls_provider(&Tls11Provider);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "TLS_DEPRECATED_VERSION");
        assert!(warnings[0].message.contains("TLS 1.1"));
    }

    #[test]
    fn tls13_no_warnings() {
        struct Tls13Provider;

        #[allow(clippy::unnecessary_literal_bound)]
        impl TlsConfigProvider for Tls13Provider {
            fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
                Ok(Arc::new(()))
            }
            fn provider_name(&self) -> &str {
                "tls13-test"
            }
            fn config_type_name(&self) -> &str {
                "()"
            }
            fn min_tls_version(&self) -> TlsVersion {
                TlsVersion::Tls13
            }
            fn supports_mtls(&self) -> bool {
                true
            }
            fn accepts_invalid_certs(&self) -> bool {
                false
            }
        }

        let warnings = audit_tls_provider(&Tls13Provider);
        assert!(warnings.is_empty());
    }

    #[test]
    fn audit_only_invalid_certs_not_deprecated_version() {
        // Provider with valid TLS version but invalid certs accepted
        struct CertOnlyInsecure;

        #[allow(clippy::unnecessary_literal_bound)]
        impl TlsConfigProvider for CertOnlyInsecure {
            fn client_config(&self) -> Result<Arc<dyn Any + Send + Sync>, ExtensionError> {
                Ok(Arc::new(()))
            }
            fn provider_name(&self) -> &str {
                "cert-insecure"
            }
            fn config_type_name(&self) -> &str {
                "()"
            }
            fn min_tls_version(&self) -> TlsVersion {
                TlsVersion::Tls13
            }
            fn supports_mtls(&self) -> bool {
                false
            }
            fn accepts_invalid_certs(&self) -> bool {
                true
            }
        }

        let warnings = audit_tls_provider(&CertOnlyInsecure);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "TLS_NO_VERIFY");
        assert_eq!(warnings[0].cwe, Some(295));
    }

    #[test]
    fn tls_version_is_not_deprecated() {
        assert!(!TlsVersion::Tls12.is_deprecated());
        assert!(!TlsVersion::Tls13.is_deprecated());
    }

    #[test]
    fn failing_provider_metadata() {
        let provider = FailingProvider;
        assert_eq!(provider.provider_name(), "test-failing");
        assert_eq!(provider.config_type_name(), "never");
        assert_eq!(provider.min_tls_version(), TlsVersion::Tls13);
        assert!(!provider.accepts_invalid_certs());
    }
}
