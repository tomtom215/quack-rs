# Structured Warnings

Extensions that access external resources (network, files, credentials) should
emit structured warnings when potentially unsafe operations occur. The
[`warning`](https://docs.rs/quack-rs/latest/quack_rs/warning/index.html) module
provides a consistent, thread-safe API for collecting and surfacing security
warnings.

## Core Types

### `ExtensionWarning`

A structured warning with:

| Field | Type | Description |
|-------|------|-------------|
| `code` | `&'static str` | Machine-readable code (e.g., `"TLS_NO_VERIFY"`) |
| `severity` | `WarningSeverity` | Info / Low / Medium / High / Critical |
| `message` | `String` | Human-readable description |
| `cwe` | `Option<u32>` | Optional [CWE](https://cwe.mitre.org/) identifier |

### `WarningSeverity`

Five levels mirroring common security advisory severity. `WarningSeverity`
implements `Ord` in this order, so `severity >= WarningSeverity::High` selects
the warnings that need attention:

- **Info** — no security impact, but worth noting
- **Low** — minimal security impact
- **Medium** — potential security concern
- **High** — significant security risk
- **Critical** — immediate action recommended

### `WarningCollector`

A thread-safe collector backed by `Mutex<Vec<ExtensionWarning>>`. Share it
across threads via `Arc<WarningCollector>`. A panic on another thread while it
held the lock does not lose warnings: the collector recovers the list from the
poisoned lock and keeps using it.

## Usage

```rust
use quack_rs::warning::{ExtensionWarning, WarningSeverity, WarningCollector};

let collector = WarningCollector::new();

// Emit a warning when detecting an insecure configuration
collector.emit(ExtensionWarning {
    code: "TLS_NO_VERIFY",
    severity: WarningSeverity::High,
    message: "TLS certificate verification is disabled".into(),
    cwe: Some(295),
});

// Check warnings
assert_eq!(collector.len(), 1);
assert!(!collector.is_empty());

// Read without clearing
let snapshot = collector.snapshot();
assert_eq!(snapshot.len(), 1);
assert_eq!(collector.len(), 1);  // still there

// Consume all warnings
let warnings = collector.drain();
assert_eq!(warnings.len(), 1);
assert!(collector.is_empty());  // now empty
```

## Display Format

`ExtensionWarning` implements `Display` as `[SEVERITY] CODE: message (CWE-nnn)`;
the CWE suffix is omitted when `cwe` is `None`:

```text
[HIGH] TLS_NO_VERIFY: TLS certificate verification is disabled (CWE-295)
[MEDIUM] TLS_DEPRECATED_VERSION: TLS provider "my-tls" allows deprecated TLS 1.0 (RFC 8996) (CWE-327)
```

## Integration with TLS Auditing

`tls::audit_tls_provider()` returns a `Vec<ExtensionWarning>` that can be fed
straight into a `WarningCollector`; see
[Auditing a Provider](tls.md#auditing-a-provider) for a complete example.

## Best Practices

- Create a single `WarningCollector` per extension (typically in global or
  bind-data state)
- Use `snapshot()` for read-only diagnostics; use `drain()` when consuming
  warnings for output
- Include a CWE identifier whenever one applies
- Surface collected warnings through a table function of your own
  (for example `SELECT * FROM __extension_warnings()`)
