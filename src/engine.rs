use std::sync::OnceLock;
use std::time::Duration;

use wasmtime::{Config, Engine};

static ENGINE: OnceLock<Engine> = OnceLock::new();

const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Returns the global Wasmtime Engine, initializing it on first call.
///
/// Panics if the engine cannot be created. This is a module-fatal
/// condition (e.g., Cranelift unavailable), not a request-time error.
pub fn engine() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        // 512 KiB — the default, but explicit for documentation.
        config.max_wasm_stack(512 * 1024);
        Engine::new(&config).unwrap_or_else(|e| {
            panic!("valkey-wasm: failed to initialize Wasmtime engine: {e}")
        })
    })
}

/// Starts a daemon thread that increments the engine epoch at a
/// fixed interval. Must be called once at module init time.
/// Subsequent calls are no-ops.
pub fn start_epoch_ticker() {
    use std::sync::Once;
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let engine = engine().clone();
        std::thread::Builder::new()
            .name("wasm-epoch-ticker".into())
            .spawn(move || loop {
                std::thread::sleep(EPOCH_TICK_INTERVAL);
                engine.increment_epoch();
            })
            .expect("valkey-wasm: failed to start epoch ticker thread");
    });
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

    #[test]
    fn engine_has_fuel_enabled() {
        let mut store = wasmtime::Store::new(engine(), ());
        // set_fuel only succeeds when fuel is enabled on the config.
        store.set_fuel(100).expect("fuel should be enabled");
    }

    #[test]
    fn epoch_ticker_starts() {
        // Calling twice should be idempotent (no panic).
        start_epoch_ticker();
        start_epoch_ticker();
        std::thread::sleep(Duration::from_millis(50));
    }
}
