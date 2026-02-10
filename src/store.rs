use wasmtime::{Store, StoreLimits, StoreLimitsBuilder};

use crate::engine::engine;

/// Data stored in each per-invocation Wasmtime Store.
pub struct StoreData {
    limits: StoreLimits,
}

/// Limits applied to each Wasm invocation.
pub struct InvocationLimits {
    /// Fuel budget (instruction count proxy).
    pub fuel: u64,
    /// Epoch deadline in ticks beyond the current epoch.
    pub epoch_deadline: u64,
    /// Max linear memory per instance, in bytes.
    pub max_memory_bytes: usize,
}

impl Default for InvocationLimits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000,
            // ~1 second at 100 ms ticks
            epoch_deadline: 10,
            // 10 MiB
            max_memory_bytes: 10 * 1024 * 1024,
        }
    }
}

/// Creates a new Wasmtime Store configured with resource limits.
pub fn create_store(limits: &InvocationLimits) -> Store<StoreData> {
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.max_memory_bytes)
        .memories(1)
        .trap_on_grow_failure(true)
        .build();

    let data = StoreData {
        limits: store_limits,
    };
    let mut store = Store::new(engine(), data);
    store.limiter(|data| &mut data.limits);
    store.set_fuel(limits.fuel).expect("fuel is enabled");
    store.set_epoch_deadline(limits.epoch_deadline);
    store.epoch_deadline_trap();
    store
}

#[cfg(test)]
mod tests {
    use wasmtime::component::{Component, Linker};

    use super::*;
    use crate::engine::start_epoch_ticker;

    static SPIN_COMPONENT: &[u8] = include_bytes!("testdata/spin.component.wasm");
    static BIG_MEMORY_COMPONENT: &[u8] = include_bytes!("testdata/big-memory.component.wasm");

    #[test]
    fn create_store_default() {
        let store = create_store(&InvocationLimits::default());
        assert_eq!(store.get_fuel().unwrap(), 1_000_000);
    }

    #[test]
    fn fuel_exhaustion_traps() {
        let limits = InvocationLimits {
            fuel: 10_000,
            // Large deadline so fuel hits first.
            epoch_deadline: 1_000,
            ..Default::default()
        };
        let mut store = create_store(&limits);
        let component = Component::new(engine(), SPIN_COMPONENT)
            .expect("spin component should compile");
        let linker = Linker::<StoreData>::new(engine());
        let instance = linker.instantiate(&mut store, &component).unwrap();

        let spin = instance
            .get_typed_func::<(), ()>(&mut store, "spin")
            .expect("spin export should exist");
        let err = spin.call(&mut store, ()).unwrap_err();
        let trap = err.downcast_ref::<wasmtime::Trap>();
        assert_eq!(trap, Some(&wasmtime::Trap::OutOfFuel));
    }

    #[test]
    fn epoch_deadline_traps() {
        start_epoch_ticker();
        let limits = InvocationLimits {
            // Huge fuel so epoch hits first.
            fuel: u64::MAX / 2,
            // 1 tick = ~100 ms
            epoch_deadline: 1,
            ..Default::default()
        };
        let mut store = create_store(&limits);
        let component = Component::new(engine(), SPIN_COMPONENT)
            .expect("spin component should compile");
        let linker = Linker::<StoreData>::new(engine());
        let instance = linker.instantiate(&mut store, &component).unwrap();

        let spin = instance
            .get_typed_func::<(), ()>(&mut store, "spin")
            .expect("spin export should exist");
        let err = spin.call(&mut store, ()).unwrap_err();
        let trap = err.downcast_ref::<wasmtime::Trap>();
        assert_eq!(trap, Some(&wasmtime::Trap::Interrupt));
    }

    #[test]
    fn memory_limit_rejects_large_initial_memory() {
        // big-memory.component.wasm declares 512 pages = 32 MiB initial.
        // Our limit is 10 MiB, so instantiation should fail.
        let limits = InvocationLimits::default();
        let mut store = create_store(&limits);
        let component = Component::new(engine(), BIG_MEMORY_COMPONENT)
            .expect("big-memory component should compile");
        let linker = Linker::<StoreData>::new(engine());
        let result = linker.instantiate(&mut store, &component);
        assert!(result.is_err(), "should reject oversized initial memory");
    }
}
