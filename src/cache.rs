use std::collections::HashMap;

use wasmtime::component::Component;

use crate::engine::engine;
use crate::wasm_types::{Sha256Digest, WasmError};

/// In-memory cache of compiled Wasm Components, keyed by SHA-256.
///
/// Not thread-safe; callers must wrap in `Mutex` or `RwLock`.
pub struct ComponentCache {
    components: HashMap<Sha256Digest, Component>,
    max_entries: usize,
}

impl ComponentCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            components: HashMap::new(),
            max_entries,
        }
    }

    /// Returns a cached component or compiles and caches it.
    ///
    /// Returns `WasmError::CacheFull` if the cache is at capacity and the
    /// component is not already cached. Returns `WasmError::CompileFailed`
    /// if Wasmtime rejects the bytes.
    pub fn compile_or_get(
        &mut self,
        sha: &Sha256Digest,
        component_bytes: &[u8],
    ) -> Result<&Component, WasmError> {
        if self.components.contains_key(sha) {
            return Ok(&self.components[sha]);
        }

        if self.components.len() >= self.max_entries {
            return Err(WasmError::CacheFull {
                max_entries: self.max_entries,
                current: self.components.len(),
            });
        }

        let component = Component::new(engine(), component_bytes)
            .map_err(WasmError::CompileFailed)?;
        self.components.insert(sha.clone(), component);
        Ok(&self.components[sha])
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static MINIMAL_COMPONENT: &[u8] =
        include_bytes!("testdata/minimal.component.wasm");

    #[test]
    fn compile_valid_component() {
        let mut cache = ComponentCache::new(8);
        let sha = Sha256Digest::of(MINIMAL_COMPONENT);
        let result = cache.compile_or_get(&sha, MINIMAL_COMPONENT);
        assert!(result.is_ok());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_hit_on_second_call() {
        let mut cache = ComponentCache::new(8);
        let sha = Sha256Digest::of(MINIMAL_COMPONENT);
        cache.compile_or_get(&sha, MINIMAL_COMPONENT).unwrap();
        // Second call should hit cache, not recompile.
        let result = cache.compile_or_get(&sha, MINIMAL_COMPONENT);
        assert!(result.is_ok());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn compile_garbage_bytes_fails() {
        let mut cache = ComponentCache::new(8);
        let garbage = b"this is not valid wasm";
        let sha = Sha256Digest::of(garbage);
        let result = cache.compile_or_get(&sha, garbage);
        assert!(matches!(result, Err(WasmError::CompileFailed(_))));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn cache_full_rejects_new_entry() {
        let mut cache = ComponentCache::new(1);
        let sha1 = Sha256Digest::of(MINIMAL_COMPONENT);
        cache.compile_or_get(&sha1, MINIMAL_COMPONENT).unwrap();

        // A different key with the same bytes (simulating a "different" component).
        let fake_bytes = [MINIMAL_COMPONENT, b"x"].concat();
        let sha2 = Sha256Digest::of(&fake_bytes);
        let result = cache.compile_or_get(&sha2, MINIMAL_COMPONENT);
        match result {
            Err(WasmError::CacheFull {
                max_entries,
                current,
            }) => {
                assert_eq!(max_entries, 1);
                assert_eq!(current, 1);
            }
            Ok(_) => panic!("expected CacheFull, got Ok"),
            Err(e) => panic!("expected CacheFull, got {e}"),
        }
    }

    #[test]
    fn cache_full_still_returns_existing() {
        let mut cache = ComponentCache::new(1);
        let sha = Sha256Digest::of(MINIMAL_COMPONENT);
        cache.compile_or_get(&sha, MINIMAL_COMPONENT).unwrap();

        // Same key should still work even at capacity.
        let result = cache.compile_or_get(&sha, MINIMAL_COMPONENT);
        assert!(result.is_ok());
    }
}
