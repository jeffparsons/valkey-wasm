use std::sync::OnceLock;

use wasmtime::{Config, Engine};

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Returns the global Wasmtime Engine, initializing it on first call.
///
/// Panics if the engine cannot be created. This is a module-fatal
/// condition (e.g., Cranelift unavailable), not a request-time error.
pub fn engine() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let config = Config::new();
        // Later tickets enable these on `config`:
        // config.consume_fuel(true);       // T2.2
        // config.epoch_interruption(true); // T2.3
        Engine::new(&config).unwrap_or_else(|e| {
            panic!("valkey-wasm: failed to initialize Wasmtime engine: {e}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_returns_same_instance() {
        let a = engine();
        let b = engine();
        assert!(std::ptr::eq(a, b));
    }
}
