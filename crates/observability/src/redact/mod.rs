//! Masking for secrets that would otherwise reach debug output or a log sink.

/// Text written in place of a secret or denied value.
pub(crate) const REDACTED: &str = "[REDACTED]";
