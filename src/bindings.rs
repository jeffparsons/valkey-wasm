wasmtime::component::bindgen!({
    world: "read-only",
    path: "wit",
});

// Re-export the types other modules need, so WIT path/world name
// changes don't ripple across the codebase.
pub use self::valkey::scripting::server_read_only::Args;
pub use self::valkey::scripting::server_read_only::Host as ServerReadOnlyHost;
pub use self::valkey::scripting::types::Value;
