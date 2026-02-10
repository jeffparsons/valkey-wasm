use valkey_module::alloc::ValkeyAlloc;
use valkey_module::{valkey_module, Context, ValkeyError, ValkeyResult, ValkeyString};

// Placeholder command to validate module loading.
// Will be replaced/augmented by WASM.EVAL_RO in later phases.
fn wasm_ping(_ctx: &Context, args: Vec<ValkeyString>) -> ValkeyResult {
    if args.len() != 1 {
        return Err(ValkeyError::WrongArity);
    }
    Ok("PONG".into())
}

valkey_module! {
    name: "wasm",
    // Module ABI version, not Wasm ABI version.
    version: 1,
    allocator: (ValkeyAlloc, ValkeyAlloc),
    data_types: [],
    commands: [
        ["wasm.ping", wasm_ping, "readonly", 0, 0, 0],
    ],
}
