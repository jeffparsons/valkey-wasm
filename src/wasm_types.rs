use std::fmt;

use sha2::{Digest, Sha256};

/// SHA-256 digest used as cache key for compiled components.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Internal error type for Wasm compilation and caching.
/// Not directly exposed to Valkey clients; command handlers
/// map these to user-facing error strings.
pub enum WasmError {
    CompileFailed(anyhow::Error),
    CacheFull {
        max_entries: usize,
        current: usize,
    },
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WasmError::CompileFailed(e) => write!(f, "component compilation failed: {e}"),
            WasmError::CacheFull {
                max_entries,
                current,
            } => write!(
                f,
                "component cache full ({current}/{max_entries} entries)"
            ),
        }
    }
}

impl fmt::Debug for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_value() {
        // SHA-256 of empty input is well-known.
        let digest = Sha256Digest::of(b"");
        assert_eq!(
            digest.to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_debug_is_hex() {
        let digest = Sha256Digest::of(b"hello");
        let debug = format!("{digest:?}");
        let display = format!("{digest}");
        assert_eq!(debug, display);
        assert_eq!(debug.len(), 64);
    }

    #[test]
    fn sha256_equality() {
        let a = Sha256Digest::of(b"abc");
        let b = Sha256Digest::of(b"abc");
        let c = Sha256Digest::of(b"xyz");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
