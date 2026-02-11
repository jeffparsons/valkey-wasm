#![no_std]
extern crate alloc;

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

        let results = server_read_only::mget(&[b"test".to_vec()]);
        match results.first() {
            Some(Some(bytes)) => Value::Bytes(bytes.clone()),
            _ => Value::Nil,
        }
    }
}

export!(Component);

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
