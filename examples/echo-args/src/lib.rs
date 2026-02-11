#![no_std]
extern crate alloc;

use alloc::format;
use dlmalloc::GlobalDlmalloc;

#[global_allocator]
static ALLOC: GlobalDlmalloc = GlobalDlmalloc;

wit_bindgen::generate!({
    world: "read-only",
    path: "../../wit",
});

struct Component;

impl exports::valkey::scripting::script::Guest for Component {
    fn run() -> valkey::scripting::types::Value {
        use valkey::scripting::server_read_only;
        use valkey::scripting::types::Value;

        let args = server_read_only::get_args();
        // Convert bytes to strings for readable test assertions.
        // Safe: test fixtures always use valid UTF-8.
        let keys: alloc::vec::Vec<&str> = args
            .keys
            .iter()
            .map(|k| core::str::from_utf8(k).unwrap_or("?"))
            .collect();
        let argv: alloc::vec::Vec<&str> = args
            .argv
            .iter()
            .map(|a| core::str::from_utf8(a).unwrap_or("?"))
            .collect();
        Value::Ok(format!("keys={keys:?} argv={argv:?}"))
    }
}

export!(Component);

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
