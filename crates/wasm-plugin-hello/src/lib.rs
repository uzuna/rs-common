#[allow(warnings)]
mod bindings;

use std::cell::RefCell;

use bindings::{
    exports::component::wasm_plugin_hello::types::{
        Guest as SetterTrait, GuestSetter, Pos2, Setter,
    },
    Guest,
};

struct HostSetter {
    value: Pos2,
}

impl HostSetter {
    fn new() -> Self {
        HostSetter {
            value: Pos2 { x: 0.0, y: 0.0 },
        }
    }

    fn set(&mut self, p: Pos2) {
        self.value = p;
    }

    fn get(&self) -> Pos2 {
        self.value
    }
}

struct GuestSetterImpl {
    inner: RefCell<HostSetter>,
}

impl GuestSetter for GuestSetterImpl {
    fn new() -> Setter {
        let inner = HostSetter::new();
        let inner = RefCell::new(inner);
        Setter::new(GuestSetterImpl { inner })
    }

    fn set(&self, p: Pos2) {
        self.inner.borrow_mut().set(p);
    }

    fn get(&self) -> Pos2 {
        self.inner.borrow().get()
    }
}
struct Component;

impl Guest for Component {
    /// Say hello!
    fn hello_world() -> String {
        "Hello, World!".to_string()
    }

    fn add(a: u32, b: u32) -> u32 {
        a + b
    }

    fn sum(l: Vec<u32>) -> u32 {
        l.iter().sum()
    }

    fn loop_sum(len: u32) -> u32 {
        (0..len).sum()
    }

    fn generate_string(len: u32) -> String {
        let mut s = String::with_capacity(len as usize);
        for i in 0..len {
            s.push_str(&i.to_string());
        }
        s
    }
}

impl SetterTrait for Component {
    type Setter = GuestSetterImpl;
}

bindings::export!(Component with_types_in bindings);
