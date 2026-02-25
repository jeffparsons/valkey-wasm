mod bindings;
mod cache;
mod engine;
mod scripting_engine;
mod store;
mod wasm_types;

use valkey_module::alloc::ValkeyAlloc;
use valkey_module::{valkey_module, Context, Status, ValkeyError, ValkeyResult, ValkeyString};

// Placeholder command to validate module loading.
// Will be replaced/augmented by WASM.EVAL_RO in later phases.
fn wasm_ping(_ctx: &Context, args: Vec<ValkeyString>) -> ValkeyResult {
    if args.len() != 1 {
        return Err(ValkeyError::WrongArity);
    }
    Ok("PONG".into())
}

fn wasm_init(ctx: &Context, _args: &[ValkeyString]) -> Status {
    engine::start_epoch_ticker();
    if scripting_engine::register(ctx) == Status::Err {
        return Status::Err;
    }
    Status::Ok
}

fn wasm_deinit(ctx: &Context) -> Status {
    scripting_engine::unregister(ctx)
}

valkey_module! {
    name: "wasm",
    // Module ABI version, not Wasm ABI version.
    version: 1,
    allocator: (ValkeyAlloc, ValkeyAlloc),
    data_types: [],
    init: wasm_init,
    deinit: wasm_deinit,
    commands: [
        ["wasm.ping", wasm_ping, "readonly", 0, 0, 0],
    ],
}
