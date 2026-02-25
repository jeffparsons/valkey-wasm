//! Valkey scripting engine integration.
//!
//! Registers "wasm" as a first-class scripting engine via the
//! `ValkeyModule_RegisterScriptingEngine` API (Valkey 8.1+), enabling
//! `FUNCTION LOAD` / `FCALL` workflows for WASM components.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;
use std::sync::Mutex;

use valkey_module::raw::{
    ValkeyModuleCtx, ValkeyModuleScriptingEngineCallableLazyEvalReset,
    ValkeyModuleScriptingEngineCompiledFunction, ValkeyModuleScriptingEngineCtx,
    ValkeyModuleScriptingEngineMemoryInfo, ValkeyModuleScriptingEngineMethods,
    ValkeyModuleScriptingEngineServerRuntimeCtx,
    ValkeyModuleScriptingEngineSubsystemType,
    ValkeyModuleScriptingEngineSubsystemType_VMSE_FUNCTION, ValkeyModuleString,
    ValkeyModule_CreateString, ValkeyModule_RegisterScriptingEngine,
    ValkeyModule_ReplyWithError, ValkeyModule_ReplyWithLongLong,
    ValkeyModule_ReplyWithNull, ValkeyModule_ReplyWithStringBuffer,
    ValkeyModule_StringPtrLen, ValkeyModule_UnregisterScriptingEngine,
    VALKEYMODULE_SCRIPTING_ENGINE_ABI_COMPILED_FUNCTION_VERSION,
    VALKEYMODULE_SCRIPTING_ENGINE_ABI_VERSION,
};
use valkey_module::{Context, Status};
use wasmtime::component::{Component, Linker};

use crate::bindings::{ReadOnly, Value};
use crate::cache::ComponentCache;
use crate::engine::engine;
use crate::store::{create_store, InvocationLimits, StoreData};
use crate::wasm_types::Sha256Digest;

const ENGINE_NAME: &[u8] = b"wasm\0";
const MAX_CACHED_COMPONENTS: usize = 256;

/// Opaque context passed through all scripting engine callbacks.
///
/// Heap-allocated and cast to/from `*mut ValkeyModuleScriptingEngineCtx`
/// (which is `*mut c_void`).
struct EngineContext {
    cache: Mutex<ComponentCache>,
    linker: Linker<StoreData>,
}

impl EngineContext {
    fn new() -> Result<Self, String> {
        let mut linker = Linker::<StoreData>::new(engine());
        ReadOnly::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |x| x)
            .map_err(|e| format!("failed to set up linker: {e}"))?;
        Ok(Self {
            cache: Mutex::new(ComponentCache::new(MAX_CACHED_COMPONENTS)),
            linker,
        })
    }
}

// SAFETY: EngineContext is only accessed from Valkey's main thread (single-threaded
// callback model). The Mutex on cache is for API correctness, not concurrent access.
unsafe impl Send for EngineContext {}
unsafe impl Sync for EngineContext {}

/// Extracts a `&[u8]` from a `*const ValkeyModuleString`.
///
/// # Safety
///
/// `s` must be a valid, non-null ValkeyModuleString pointer.
unsafe fn vm_string_as_bytes(s: *const ValkeyModuleString) -> &'static [u8] {
    let mut len: usize = 0;
    let ptr = unsafe { ValkeyModule_StringPtrLen.unwrap()(s, &mut len) };
    unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }
}

/// Creates a `*mut ValkeyModuleString` from a byte slice.
///
/// # Safety
///
/// `ctx` must be a valid module context or null.
unsafe fn vm_create_string(
    ctx: *mut ValkeyModuleCtx,
    bytes: &[u8],
) -> *mut ValkeyModuleString {
    unsafe {
        ValkeyModule_CreateString.unwrap()(ctx, bytes.as_ptr().cast::<c_char>(), bytes.len())
    }
}

// ---------------------------------------------------------------------------
// Callback implementations
// ---------------------------------------------------------------------------

/// Called by Valkey when `FUNCTION LOAD "#!wasm name=mylib\n<bytes>"` is issued.
///
/// Compiles the WASM component bytes and returns a single `CompiledFunction`.
unsafe extern "C" fn compile_code(
    module_ctx: *mut ValkeyModuleCtx,
    engine_ctx: *mut ValkeyModuleScriptingEngineCtx,
    type_: ValkeyModuleScriptingEngineSubsystemType,
    code: *const c_char,
    _timeout: usize,
    out_num_compiled_functions: *mut usize,
    err: *mut *mut ValkeyModuleString,
) -> *mut *mut ValkeyModuleScriptingEngineCompiledFunction {
    // We only support the FUNCTION subsystem.
    if type_ != ValkeyModuleScriptingEngineSubsystemType_VMSE_FUNCTION {
        unsafe {
            let msg = b"ERR wasm engine only supports FUNCTION subsystem\0";
            *err = vm_create_string(module_ctx, &msg[..msg.len() - 1]);
            *out_num_compiled_functions = 0;
        }
        return ptr::null_mut();
    }

    let engine_ctx = unsafe { &*(engine_ctx as *const EngineContext) };

    // The C API passes code as `const char*` (no length). This uses strlen
    // semantics, which truncates at the first null byte. WASM binaries contain
    // interior nulls, so this won't work for most real components.
    // TODO: Upstream a Valkey API change to pass code length, or use base64.
    let code_bytes = unsafe { CStr::from_ptr(code).to_bytes() };

    // Compile the component.
    let sha = Sha256Digest::of(code_bytes);
    let component = {
        let mut cache = engine_ctx.cache.lock().unwrap();
        match cache.compile_or_get(&sha, code_bytes) {
            Ok(component) => component.clone(),
            Err(e) => unsafe {
                let msg = format!("ERR {e}");
                *err = vm_create_string(module_ctx, msg.as_bytes());
                *out_num_compiled_functions = 0;
                return ptr::null_mut();
            },
        }
    };

    // Build the CompiledFunction struct.
    // The function name is "run" — our WIT exports a single `run()` function.
    let compiled_fn =
        Box::new(ValkeyModuleScriptingEngineCompiledFunction {
            version: VALKEYMODULE_SCRIPTING_ENGINE_ABI_COMPILED_FUNCTION_VERSION as u64,
            name: unsafe { vm_create_string(module_ctx, b"run") },
            function: Box::into_raw(Box::new(component)) as *mut c_void,
            desc: ptr::null_mut(),
            f_flags: 0,
        });

    let compiled_fn_ptr = Box::into_raw(compiled_fn);

    // Return an array of one pointer.
    let result = Box::into_raw(Box::new(compiled_fn_ptr));
    unsafe { *out_num_compiled_functions = 1 };
    result
}

/// Called by Valkey on `FCALL`.
///
/// Extracts keys/args, instantiates the WASM component, calls `run()`,
/// and replies via the module context.
unsafe extern "C" fn call_function(
    module_ctx: *mut ValkeyModuleCtx,
    engine_ctx: *mut ValkeyModuleScriptingEngineCtx,
    _server_ctx: *mut ValkeyModuleScriptingEngineServerRuntimeCtx,
    compiled_function: *mut ValkeyModuleScriptingEngineCompiledFunction,
    _type: ValkeyModuleScriptingEngineSubsystemType,
    keys: *mut *mut ValkeyModuleString,
    nkeys: usize,
    args: *mut *mut ValkeyModuleString,
    nargs: usize,
) {
    let engine_ctx = unsafe { &*(engine_ctx as *const EngineContext) };
    let compiled = unsafe { &*compiled_function };
    let component = unsafe { &*(compiled.function as *const Component) };

    // Convert keys and args to Vec<Vec<u8>>.
    let keys_vec: Vec<Vec<u8>> = if nkeys > 0 {
        let keys_slice = unsafe { std::slice::from_raw_parts(keys, nkeys) };
        keys_slice
            .iter()
            .map(|s| unsafe { vm_string_as_bytes(*s).to_vec() })
            .collect()
    } else {
        vec![]
    };

    let args_vec: Vec<Vec<u8>> = if nargs > 0 {
        let args_slice = unsafe { std::slice::from_raw_parts(args, nargs) };
        args_slice
            .iter()
            .map(|s| unsafe { vm_string_as_bytes(*s).to_vec() })
            .collect()
    } else {
        vec![]
    };

    let limits = InvocationLimits::default();
    let mut store = create_store(&limits, keys_vec, args_vec);

    let instance = match ReadOnly::instantiate(&mut store, component, &engine_ctx.linker) {
        Ok(inst) => inst,
        Err(e) => {
            let msg = format!("ERR wasm instantiation failed: {e}\0");
            unsafe {
                ValkeyModule_ReplyWithError.unwrap()(module_ctx, msg.as_ptr().cast::<c_char>());
            }
            return;
        }
    };

    let result = match instance
        .valkey_scripting_script()
        .call_run(&mut store)
    {
        Ok(val) => val,
        Err(e) => {
            let wasm_err = crate::wasm_types::classify_execution_error(e);
            let msg = format!("ERR {wasm_err}\0");
            unsafe {
                ValkeyModule_ReplyWithError.unwrap()(module_ctx, msg.as_ptr().cast::<c_char>());
            }
            return;
        }
    };

    // Translate the WIT Value variant to a Valkey reply.
    reply_with_value(module_ctx, &result);
}

/// Translates a WIT `Value` variant to a Valkey RESP reply.
fn reply_with_value(ctx: *mut ValkeyModuleCtx, value: &Value) {
    unsafe {
        match value {
            Value::Nil => {
                ValkeyModule_ReplyWithNull.unwrap()(ctx);
            }
            Value::Ok(s) => {
                ValkeyModule_ReplyWithStringBuffer.unwrap()(
                    ctx,
                    s.as_ptr().cast::<c_char>(),
                    s.len(),
                );
            }
            Value::Error(s) => {
                // Must be null-terminated for ReplyWithError.
                let msg = format!("ERR {s}\0");
                ValkeyModule_ReplyWithError.unwrap()(ctx, msg.as_ptr().cast::<c_char>());
            }
            Value::Int(n) => {
                ValkeyModule_ReplyWithLongLong.unwrap()(ctx, *n);
            }
            Value::Bytes(b) => {
                ValkeyModule_ReplyWithStringBuffer.unwrap()(
                    ctx,
                    b.as_ptr().cast::<c_char>(),
                    b.len(),
                );
            }
        }
    }
}

/// Called by Valkey when a function library is removed.
///
/// Frees the Component and CompiledFunction struct. Name/desc strings are
/// owned by Valkey after `compile_code` returns — we must not free them here.
unsafe extern "C" fn free_function(
    _module_ctx: *mut ValkeyModuleCtx,
    _engine_ctx: *mut ValkeyModuleScriptingEngineCtx,
    _type: ValkeyModuleScriptingEngineSubsystemType,
    compiled_function: *mut ValkeyModuleScriptingEngineCompiledFunction,
) {
    let cf = unsafe { Box::from_raw(compiled_function) };
    // Drop the Component.
    let _ = unsafe { Box::from_raw(cf.function as *mut Component) };
    // cf is dropped here, freeing the CompiledFunction struct itself.
}

/// Returns a rough memory estimate for the compiled function.
unsafe extern "C" fn get_function_memory_overhead(
    _module_ctx: *mut ValkeyModuleCtx,
    _compiled_function: *mut ValkeyModuleScriptingEngineCompiledFunction,
) -> usize {
    // Component is an opaque Wasmtime type; return a rough estimate.
    std::mem::size_of::<Component>()
}

/// No-op: WASM stores are per-invocation, so there's no global eval
/// environment to reset.
unsafe extern "C" fn reset_eval_env(
    _module_ctx: *mut ValkeyModuleCtx,
    _engine_ctx: *mut ValkeyModuleScriptingEngineCtx,
    _is_async: c_int,
) -> *mut ValkeyModuleScriptingEngineCallableLazyEvalReset {
    ptr::null_mut()
}

/// Returns memory usage info for the scripting engine.
unsafe extern "C" fn get_memory_info(
    _module_ctx: *mut ValkeyModuleCtx,
    engine_ctx: *mut ValkeyModuleScriptingEngineCtx,
    _type: ValkeyModuleScriptingEngineSubsystemType,
) -> ValkeyModuleScriptingEngineMemoryInfo {
    let engine_ctx = unsafe { &*(engine_ctx as *const EngineContext) };
    let cache_entries = engine_ctx.cache.lock().unwrap().len();

    ValkeyModuleScriptingEngineMemoryInfo {
        version: 1,
        used_memory: cache_entries * std::mem::size_of::<Component>(),
        engine_memory_overhead: std::mem::size_of::<EngineContext>(),
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Registers the "wasm" scripting engine with Valkey.
///
/// Must be called during module init. Returns `Status::Err` if the server
/// does not support the scripting engine API (Valkey < 8.1).
pub fn register(ctx: &Context) -> Status {
    let register_fn = unsafe { ValkeyModule_RegisterScriptingEngine };
    let register_fn = match register_fn {
        Some(f) => f,
        None => {
            ctx.log_warning(
                "ValkeyModule_RegisterScriptingEngine not available; \
                 requires Valkey 8.1+",
            );
            return Status::Err;
        }
    };

    let engine_ctx = match EngineContext::new() {
        Ok(ec) => ec,
        Err(e) => {
            ctx.log_warning(&format!("failed to create wasm engine context: {e}"));
            return Status::Err;
        }
    };

    // Heap-allocate so it lives for the duration of module load.
    let engine_ctx_ptr = Box::into_raw(Box::new(engine_ctx));

    // Build the methods struct. Stored on the stack here but Valkey copies it
    // during registration.
    let mut methods = ValkeyModuleScriptingEngineMethods {
        version: VALKEYMODULE_SCRIPTING_ENGINE_ABI_VERSION as u64,
        compile_code: Some(compile_code),
        free_function: Some(free_function),
        call_function: Some(call_function),
        get_function_memory_overhead: Some(get_function_memory_overhead),
        reset_eval_env: Some(reset_eval_env),
        get_memory_info: Some(get_memory_info),
    };

    let result = unsafe {
        register_fn(
            ctx.ctx.cast::<ValkeyModuleCtx>(),
            ENGINE_NAME.as_ptr().cast::<c_char>(),
            engine_ctx_ptr as *mut ValkeyModuleScriptingEngineCtx,
            &mut methods,
        )
    };

    if result != 0 {
        // Registration failed — clean up.
        let _ = unsafe { Box::from_raw(engine_ctx_ptr) };
        ctx.log_warning("failed to register wasm scripting engine");
        return Status::Err;
    }

    // Store the engine context pointer so we can free it on unload.
    ENGINE_CTX.set(engine_ctx_ptr).unwrap_or_else(|_| {
        panic!("wasm scripting engine registered twice");
    });

    ctx.log_notice("wasm scripting engine registered");
    Status::Ok
}

/// Unregisters the "wasm" scripting engine from Valkey.
///
/// Called during module unload.
pub fn unregister(ctx: &Context) -> Status {
    let unregister_fn = match unsafe { ValkeyModule_UnregisterScriptingEngine } {
        Some(f) => f,
        None => return Status::Ok,
    };

    let result = unsafe {
        unregister_fn(
            ctx.ctx.cast::<ValkeyModuleCtx>(),
            ENGINE_NAME.as_ptr().cast::<c_char>(),
        )
    };

    if result != 0 {
        ctx.log_warning("failed to unregister wasm scripting engine");
        return Status::Err;
    }

    // Free the engine context.
    if let Some(ptr) = ENGINE_CTX.take() {
        let _ = unsafe { Box::from_raw(ptr) };
    }

    ctx.log_notice("wasm scripting engine unregistered");
    Status::Ok
}

/// Global storage for the engine context pointer, so we can free it on unload.
static ENGINE_CTX: EngineCtxCell = EngineCtxCell::new();

/// A simple cell to store and retrieve the engine context pointer.
/// Only accessed from Valkey's main thread during init/deinit.
struct EngineCtxCell {
    ptr: std::sync::OnceLock<*mut EngineContext>,
}

impl EngineCtxCell {
    const fn new() -> Self {
        Self {
            ptr: std::sync::OnceLock::new(),
        }
    }

    fn set(&self, ptr: *mut EngineContext) -> Result<(), *mut EngineContext> {
        self.ptr.set(ptr)
    }

    fn take(&self) -> Option<*mut EngineContext> {
        self.ptr.get().copied()
    }
}

// SAFETY: Only accessed from Valkey's main thread.
unsafe impl Send for EngineCtxCell {}
unsafe impl Sync for EngineCtxCell {}
