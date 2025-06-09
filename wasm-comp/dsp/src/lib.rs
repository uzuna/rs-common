use crate::bindings::exports::local::dsp::single_channel::{Guest, GuestProcessor, Single, Value};

#[allow(warnings)]
mod bindings;

struct GuestProcessorImpl {}

impl GuestProcessor for GuestProcessorImpl {
    fn new() -> Self {
        GuestProcessorImpl {}
    }

    fn process(&self, input: Single) -> Value {
        input.data
    }
}

struct Component;

impl Guest for Component {
    type Processor = GuestProcessorImpl;
}

bindings::export!(Component with_types_in bindings);
