// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Structured security warning API for extensions.
//!
//! Extensions that access external resources (network, files, credentials) should
//! emit structured warnings when potentially unsafe operations occur. This module
//! provides [`ExtensionWarning`], [`WarningSeverity`], and a thread-safe
//! [`WarningCollector`] that extensions can use to accumulate warnings during
//! execution and surface them through a consistent interface (e.g., a `DuckDB`
//! table function like `__extension_warnings()`).
//!
//! # Example
//!
//! ```rust
//! use quack_rs::warning::{ExtensionWarning, WarningSeverity, WarningCollector};
//!
//! let collector = WarningCollector::new();
//!
//! collector.emit(ExtensionWarning {
//!     code: "TLS_NO_VERIFY",
//!     severity: WarningSeverity::High,
//!     message: "TLS certificate verification is disabled".into(),
//!     cwe: Some(295),
//! });
//!
//! let warnings = collector.drain();
//! assert_eq!(warnings.len(), 1);
//! assert_eq!(warnings[0].code, "TLS_NO_VERIFY");
//! ```

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Severity level for extension warnings.
///
/// Mirrors common security advisory severity levels, and orders by them:
/// `Info < Low < Medium < High < Critical`, so `severity >= High` selects the
/// warnings that need attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WarningSeverity {
    /// Informational — no security impact, but worth noting.
    Info,
    /// Low severity — minimal security impact.
    Low,
    /// Medium severity — potential security concern.
    Medium,
    /// High severity — significant security risk.
    High,
    /// Critical severity — immediate action recommended.
    Critical,
}

impl fmt::Display for WarningSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => f.write_str("INFO"),
            Self::Low => f.write_str("LOW"),
            Self::Medium => f.write_str("MEDIUM"),
            Self::High => f.write_str("HIGH"),
            Self::Critical => f.write_str("CRITICAL"),
        }
    }
}

/// A structured warning emitted by an extension.
///
/// Warnings carry a machine-readable `code`, a human-readable `message`, a
/// [`WarningSeverity`] level, and an optional [CWE](https://cwe.mitre.org/)
/// identifier for security-related warnings.
#[derive(Debug, Clone)]
pub struct ExtensionWarning {
    /// Machine-readable warning code (e.g., `"TLS_NO_VERIFY"`, `"PLAINTEXT_SECRET"`).
    pub code: &'static str,

    /// Severity level of this warning.
    pub severity: WarningSeverity,

    /// Human-readable description of the warning.
    pub message: String,

    /// Optional [CWE](https://cwe.mitre.org/) identifier (e.g., `295` for
    /// "Improper Certificate Validation").
    pub cwe: Option<u32>,
}

impl fmt::Display for ExtensionWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.code, self.message)?;
        if let Some(cwe) = self.cwe {
            write!(f, " (CWE-{cwe})")?;
        }
        Ok(())
    }
}

/// A thread-safe collector for [`ExtensionWarning`]s.
///
/// Extensions create a single `WarningCollector` (typically stored in their
/// global or bind-data state) and call [`emit`][Self::emit] whenever a warning
/// condition is detected. Warnings can later be surfaced through a table
/// function (e.g., `SELECT * FROM __extension_warnings()`).
///
/// # Thread safety
///
/// `WarningCollector` uses a [`Mutex`] internally and is safe to share across
/// threads via `Arc<WarningCollector>`. A panic on another thread while it held
/// the lock does not lose warnings: every operation is a single `Vec` call, so
/// the list is intact, and the collector keeps using it.
pub struct WarningCollector {
    warnings: Mutex<Vec<ExtensionWarning>>,
}

impl WarningCollector {
    /// Creates a new, empty `WarningCollector`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            warnings: Mutex::new(Vec::new()),
        }
    }

    /// Locks the list, recovering it if a panic poisoned the lock: each
    /// critical section below is one `Vec` operation, which leaves the `Vec`
    /// valid even if it panics, so there is nothing to repair.
    fn lock(&self) -> MutexGuard<'_, Vec<ExtensionWarning>> {
        self.warnings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Emits a warning, adding it to the collector.
    pub fn emit(&self, warning: ExtensionWarning) {
        self.lock().push(warning);
    }

    /// Returns the number of warnings currently collected.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Returns `true` if no warnings have been collected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a snapshot of all collected warnings without clearing them.
    #[must_use]
    pub fn snapshot(&self) -> Vec<ExtensionWarning> {
        self.lock().clone()
    }

    /// Drains all collected warnings, returning them and leaving the collector empty.
    pub fn drain(&self) -> Vec<ExtensionWarning> {
        std::mem::take(&mut *self.lock())
    }

    /// Clears all collected warnings.
    pub fn clear(&self) {
        self.lock().clear();
    }
}

impl Default for WarningCollector {
    fn default() -> Self {
        Self::new()
    }
}

// `WarningCollector` is `Send + Sync` automatically: `Mutex<T>` is both when
// `T: Send`, and `ExtensionWarning` holds only `String`, `&'static str` and
// `Copy` fields. No `unsafe impl` is involved; this pins the property.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WarningCollector>();
};

impl core::fmt::Debug for WarningCollector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // `try_lock`, not `lock`: printing a value must never block, and must
        // not deadlock when the caller is already inside `emit`.
        // A poisoned lock still guards an intact list (see `lock`).
        let guard = match self.warnings.try_lock() {
            Ok(guard) => Ok(guard),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => Err(()),
        };
        match guard {
            Ok(warnings) => f
                .debug_struct("WarningCollector")
                .field("warnings", &*warnings)
                .finish(),
            Err(()) => f
                .debug_struct("WarningCollector")
                .field("warnings", &"<locked>")
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warning(code: &'static str, severity: WarningSeverity) -> ExtensionWarning {
        ExtensionWarning {
            code,
            severity,
            message: String::new(),
            cwe: None,
        }
    }

    /// A panic while the lock is held poisons it. The `Vec` behind it is
    /// still whole (every critical section is a single `Vec` operation), so
    /// the collector must keep working rather than drop warnings.
    #[test]
    fn a_poisoned_lock_does_not_lose_warnings() {
        let c = WarningCollector::new();
        c.emit(warning("BEFORE", WarningSeverity::Low));
        let poisoner = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = c.warnings.lock();
            panic!("poison the collector's lock");
        }));
        assert!(poisoner.is_err());
        assert!(c.warnings.is_poisoned());

        c.emit(warning("AFTER", WarningSeverity::High));
        assert!(format!("{c:?}").contains("AFTER"), "Debug reads through it");
        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());
        let codes: Vec<_> = c.snapshot().iter().map(|w| w.code).collect();
        assert_eq!(codes, ["BEFORE", "AFTER"]);
        assert_eq!(c.drain().len(), 2);
        c.emit(warning("AGAIN", WarningSeverity::Info));
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn severities_order_from_info_to_critical() {
        use WarningSeverity::{Critical, High, Info, Low, Medium};
        assert!(Info < Low && Low < Medium && Medium < High && High < Critical);
        let mut shuffled = [High, Info, Critical, Low, Medium];
        shuffled.sort();
        assert_eq!(shuffled, [Info, Low, Medium, High, Critical]);
        assert_eq!([Low, Critical, Medium].iter().max(), Some(&Critical));
    }

    #[test]
    fn severity_display() {
        assert_eq!(WarningSeverity::Info.to_string(), "INFO");
        assert_eq!(WarningSeverity::Low.to_string(), "LOW");
        assert_eq!(WarningSeverity::Medium.to_string(), "MEDIUM");
        assert_eq!(WarningSeverity::High.to_string(), "HIGH");
        assert_eq!(WarningSeverity::Critical.to_string(), "CRITICAL");
    }

    #[test]
    fn warning_display_without_cwe() {
        let w = ExtensionWarning {
            code: "TEST_WARN",
            severity: WarningSeverity::Medium,
            message: "something happened".into(),
            cwe: None,
        };
        assert_eq!(w.to_string(), "[MEDIUM] TEST_WARN: something happened");
    }

    #[test]
    fn warning_display_with_cwe() {
        let w = ExtensionWarning {
            code: "TLS_NO_VERIFY",
            severity: WarningSeverity::High,
            message: "TLS verification disabled".into(),
            cwe: Some(295),
        };
        assert_eq!(
            w.to_string(),
            "[HIGH] TLS_NO_VERIFY: TLS verification disabled (CWE-295)"
        );
    }

    #[test]
    fn collector_emit_and_drain() {
        let c = WarningCollector::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);

        c.emit(ExtensionWarning {
            code: "A",
            severity: WarningSeverity::Low,
            message: "first".into(),
            cwe: None,
        });
        c.emit(ExtensionWarning {
            code: "B",
            severity: WarningSeverity::High,
            message: "second".into(),
            cwe: Some(200),
        });

        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());

        let warnings = c.drain();
        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0].code, "A");
        assert_eq!(warnings[1].code, "B");

        // After drain, collector should be empty.
        assert!(c.is_empty());
    }

    #[test]
    fn collector_snapshot_does_not_clear() {
        let c = WarningCollector::new();
        c.emit(ExtensionWarning {
            code: "X",
            severity: WarningSeverity::Info,
            message: "test".into(),
            cwe: None,
        });

        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        // Snapshot should not clear.
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn collector_clear() {
        let c = WarningCollector::new();
        c.emit(ExtensionWarning {
            code: "Y",
            severity: WarningSeverity::Critical,
            message: "urgent".into(),
            cwe: Some(798),
        });
        assert_eq!(c.len(), 1);

        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn collector_default() {
        let c = WarningCollector::default();
        assert!(c.is_empty());
    }

    #[test]
    fn collector_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WarningCollector>();
    }

    #[test]
    fn severity_clone_eq_hash() {
        use std::collections::HashSet;

        let s1 = WarningSeverity::High;
        let s2 = s1;
        assert_eq!(s1, s2);
        let mut set = HashSet::new();
        set.insert(WarningSeverity::Low);
        set.insert(WarningSeverity::High);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn collector_drain_returns_in_order() {
        let c = WarningCollector::new();
        for i in 0..10 {
            c.emit(ExtensionWarning {
                code: "SEQ",
                severity: WarningSeverity::Info,
                message: format!("msg-{i}"),
                cwe: None,
            });
        }
        let warnings = c.drain();
        assert_eq!(warnings.len(), 10);
        for (i, w) in warnings.iter().enumerate() {
            assert_eq!(w.message, format!("msg-{i}"));
        }
    }

    #[test]
    fn collector_clear_then_emit() {
        let c = WarningCollector::new();
        c.emit(ExtensionWarning {
            code: "A",
            severity: WarningSeverity::Low,
            message: "first".into(),
            cwe: None,
        });
        c.clear();
        c.emit(ExtensionWarning {
            code: "B",
            severity: WarningSeverity::High,
            message: "second".into(),
            cwe: None,
        });
        let warnings = c.drain();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "B");
    }

    #[test]
    fn warning_clone() {
        let w = ExtensionWarning {
            code: "TEST",
            severity: WarningSeverity::Critical,
            message: "important".into(),
            cwe: Some(798),
        };
        let w2 = w.clone();
        assert_eq!(w.code, w2.code);
        assert_eq!(w.severity, w2.severity);
        assert_eq!(w.message, w2.message);
        assert_eq!(w.cwe, w2.cwe);
    }

    #[test]
    fn warning_debug_contains_fields() {
        let w = ExtensionWarning {
            code: "DBG",
            severity: WarningSeverity::Medium,
            message: "test debug".into(),
            cwe: Some(100),
        };
        let debug = format!("{w:?}");
        assert!(debug.contains("DBG"));
        assert!(debug.contains("Medium"));
        assert!(debug.contains("test debug"));
        assert!(debug.contains("100"));
    }

    #[test]
    fn snapshot_after_drain_is_empty() {
        let c = WarningCollector::new();
        c.emit(ExtensionWarning {
            code: "X",
            severity: WarningSeverity::Info,
            message: "test".into(),
            cwe: None,
        });
        let _ = c.drain();
        let snap = c.snapshot();
        assert!(snap.is_empty());
    }

    #[test]
    fn multiple_drains_second_is_empty() {
        let c = WarningCollector::new();
        c.emit(ExtensionWarning {
            code: "X",
            severity: WarningSeverity::Info,
            message: "test".into(),
            cwe: None,
        });
        let first = c.drain();
        assert_eq!(first.len(), 1);
        let second = c.drain();
        assert!(second.is_empty());
    }
}
