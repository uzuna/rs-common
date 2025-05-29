#[allow(warnings)]
mod bindings;

use std::cell::RefCell;

use bindings::{
    exports::component::wasm_plugin_hello::{
        filter::{self, Guest as FilterTrait, GuestFir},
        types::{Guest as SetterTrait, GuestSetter, GuestSummer, Pos2, Setter, Summer},
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

struct HostSummer {
    k: String,
    v: Vec<u32>,
}

impl HostSummer {
    fn new() -> Self {
        HostSummer {
            k: String::new(),
            v: Vec::new(),
        }
    }

    fn set_val(&mut self, a: Vec<u32>) {
        self.v = a;
    }

    fn set_key(&mut self, k: String) {
        self.k = k;
    }

    fn sum(&self) -> u32 {
        self.v.iter().sum()
    }

    fn get_key(&self) -> &str {
        &self.k
    }
}

struct GuestSummerImpl {
    inner: RefCell<HostSummer>,
}

impl GuestSummer for GuestSummerImpl {
    fn new() -> Summer {
        let inner = HostSummer::new();
        let s = GuestSummerImpl {
            inner: RefCell::new(inner),
        };
        Summer::new(s)
    }

    fn set_val(&self, l: Vec<u32>) {
        self.inner.borrow_mut().set_val(l);
    }

    fn set_key(&self, k: String) {
        self.inner.borrow_mut().set_key(k);
    }

    fn sum(&self) -> u32 {
        self.inner.borrow().sum()
    }

    fn get_key(&self) -> String {
        self.inner.borrow().get_key().to_string()
    }
}

struct GuestFirImpl {
    inner: RefCell<dsp::Fir>,
}

impl GuestFir for GuestFirImpl {
    fn new(tap: Vec<f32>) -> filter::Fir {
        let inner = dsp::Fir::new(tap);
        filter::Fir::new(GuestFirImpl {
            inner: RefCell::new(inner),
        })
    }

    fn new_moving(n: u32) -> filter::Fir {
        let inner = dsp::Fir::new_moving(n as usize); // Example: 3-tap moving average
        filter::Fir::new(GuestFirImpl {
            inner: RefCell::new(inner),
        })
    }

    fn filter(&self, input: f32) -> f32 {
        self.inner.borrow_mut().filter(input)
    }

    fn filter_vec(&self, input: Vec<f32>) -> Vec<f32> {
        self.inner.borrow_mut().filter_vec(&input)
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
    type Summer = GuestSummerImpl;
}

impl FilterTrait for Component {
    type Fir = GuestFirImpl;
}

bindings::export!(Component with_types_in bindings);
