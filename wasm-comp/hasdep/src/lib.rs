#[allow(warnings)]
mod bindings;

use bindings::{local::calc::add::add, Guest};

struct Component;

impl Guest for Component {
    fn add(a: u32, b: u32) -> u32 {
        add(a, b)
    }
}

bindings::export!(Component with_types_in bindings);
