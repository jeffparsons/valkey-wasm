use wasmtime::{Store, StoreLimits, StoreLimitsBuilder};

use crate::bindings::valkey::scripting::{server_read_only, types};
use crate::engine::engine;

/// Data stored in each per-invocation Wasmtime Store.
pub struct StoreData {
    limits: StoreLimits,
    // Binary-safe; sizes capped at command parse time.
    pub(crate) keys: Vec<Vec<u8>>,
    pub(crate) argv: Vec<Vec<u8>>,
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

// The types interface has no functions but bindgen requires this impl.
impl types::Host for StoreData {}

impl server_read_only::Host for StoreData {
    fn get_args(&mut self) -> server_read_only::Args {
        server_read_only::Args {
            keys: self.keys.clone(),
            argv: self.argv.clone(),
        }
    }

    // Stub: all keys absent. Real implementation in T3.3.
    fn mget(&mut self, keys: Vec<Vec<u8>>) -> Vec<Option<Vec<u8>>> {
        vec![None; keys.len()]
    }
}

/// Creates a new Wasmtime Store configured with resource limits.
pub fn create_store(
    limits: &InvocationLimits,
    keys: Vec<Vec<u8>>,
    argv: Vec<Vec<u8>>,
) -> Store<StoreData> {
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.max_memory_bytes)
        .memories(1)
        .trap_on_grow_failure(true)
        .build();

    let data = StoreData {
        limits: store_limits,
        keys,
        argv,
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
    use crate::bindings::ReadOnly;
    use crate::engine::start_epoch_ticker;

    static SPIN_COMPONENT: &[u8] = include_bytes!("testdata/spin.component.wasm");
    static BIG_MEMORY_COMPONENT: &[u8] = include_bytes!("testdata/big-memory.component.wasm");
    static ECHO_ARGS_COMPONENT: &[u8] = include_bytes!("testdata/echo-args.component.wasm");

    fn empty_store(limits: &InvocationLimits) -> Store<StoreData> {
        create_store(limits, vec![], vec![])
    }

    #[test]
    fn create_store_default() {
        let store = create_store(&InvocationLimits::default(), vec![], vec![]);
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
        let mut store = empty_store(&limits);
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
        let mut store = empty_store(&limits);
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
        let mut store = empty_store(&limits);
        let component = Component::new(engine(), BIG_MEMORY_COMPONENT)
            .expect("big-memory component should compile");
        let linker = Linker::<StoreData>::new(engine());
        let result = linker.instantiate(&mut store, &component);
        assert!(result.is_err(), "should reject oversized initial memory");
    }

    #[test]
    fn get_args_round_trip() {
        let keys = vec![b"k1".to_vec(), b"k2".to_vec()];
        let argv = vec![b"a1".to_vec()];
        let mut store = create_store(&InvocationLimits::default(), keys, argv);

        let component = Component::new(engine(), ECHO_ARGS_COMPONENT)
            .expect("echo-args component should compile");

        let mut linker = Linker::<StoreData>::new(engine());
        ReadOnly::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |x| x)
            .expect("add_to_linker should succeed");
        let instance = ReadOnly::instantiate(&mut store, &component, &linker)
            .expect("instantiation should succeed");

        let result = instance.valkey_scripting_script().call_run(&mut store)
            .expect("call_run should succeed");
        match result {
            crate::bindings::Value::Ok(s) => {
                assert_eq!(s, r#"keys=["k1", "k2"] argv=["a1"]"#);
            }
            other => panic!("expected Value::Ok, got {other:?}"),
        }
    }

    #[test]
    fn get_args_empty() {
        let mut store = create_store(&InvocationLimits::default(), vec![], vec![]);

        let component = Component::new(engine(), ECHO_ARGS_COMPONENT)
            .expect("echo-args component should compile");

        let mut linker = Linker::<StoreData>::new(engine());
        ReadOnly::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |x| x)
            .expect("add_to_linker should succeed");
        let instance = ReadOnly::instantiate(&mut store, &component, &linker)
            .expect("instantiation should succeed");

        let result = instance.valkey_scripting_script().call_run(&mut store)
            .expect("call_run should succeed");
        match result {
            crate::bindings::Value::Ok(s) => {
                assert_eq!(s, r#"keys=[] argv=[]"#);
            }
            other => panic!("expected Value::Ok, got {other:?}"),
        }
    }

    #[test]
    fn mget_stub_returns_none() {
        let mut data = StoreData {
            limits: StoreLimitsBuilder::new().build(),
            keys: vec![],
            argv: vec![],
        };
        let result = server_read_only::Host::mget(&mut data, vec![b"x".to_vec()]);
        assert_eq!(result, vec![None]);
    }
}
