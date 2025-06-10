use std::{cell::RefCell, collections::VecDeque, vec};

use crate::bindings::{
    exports::local::dsp::single_channel::{
        Guest as SingleChannelGuest, GuestProcessor, Single, Value,
    },
    Guest,
};

#[allow(warnings)]
mod bindings;

struct ProcessorInner {
    buffer: VecDeque<i16>,
}

impl ProcessorInner {
    fn new(len: usize) -> Self {
        ProcessorInner {
            buffer: VecDeque::from(vec![0; len]),
        }
    }

    fn process(&mut self, input: Value) -> Value {
        let res = self.buffer.pop_front().unwrap_or(0);
        self.buffer.push_back(input);
        res
    }
}

struct GuestProcessorImpl {
    inner: RefCell<ProcessorInner>,
}

impl GuestProcessor for GuestProcessorImpl {
    fn new() -> Self {
        GuestProcessorImpl {
            inner: RefCell::new(ProcessorInner::new(10)), // 例として10sample Delayを実装
        }
    }

    fn process(&self, input: Single) -> Value {
        let mut inner = self.inner.borrow_mut();
        inner.process(input.data)
    }
}

struct Component;

impl SingleChannelGuest for Component {
    type Processor = GuestProcessorImpl;
}

impl Guest for Component {
    fn plugin_name() -> String {
        "dsp-single-channel".to_string()
    }
}

bindings::export!(Component with_types_in bindings);
