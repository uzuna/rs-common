#[allow(warnings)]
mod bindings;

use bindings::Guest;

struct Component;

impl Guest for Component {
    /// Say hello!
    fn hello_world() -> String {
        "Hello, World!".to_string()
    }

    fn add(a: u32, b: u32) -> u32 {
        a + b
    }
}

bindings::export!(Component with_types_in bindings);
