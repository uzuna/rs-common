use std::{cell::RefCell, collections::VecDeque, vec};

use crate::bindings::{
    exports::local::dsp::single_channel::{
        Guest as SingleChannelGuest, GuestProcessor, Parameter, Single, Value,
    },
    Guest,
};

#[allow(warnings)]
mod bindings;

enum Param {
    BufferSize(usize),
    Gain(f32),
}

impl Param {
    fn from_parameter(param: &Parameter) -> Result<Self, String> {
        match param.name.as_str() {
            "buffer_size" => match param.value.parse::<usize>() {
                Ok(size) => Ok(Param::BufferSize(size)),
                Err(_) => Err(format!("buffer_size failed to parse: {}", param.value)),
            },
            "gain" => match param.value.parse::<f32>() {
                Ok(gain) => Ok(Param::Gain(gain)),
                Err(_) => Err(format!("gain: failed to parse: {}", param.value)),
            },
            _ => Err(format!("Unknown parameter: {}", param.name)),
        }
    }

    fn as_parameter(&self) -> Parameter {
        let name = self.as_name().to_string();
        match self {
            Param::BufferSize(size) => Parameter {
                name,
                value: size.to_string(),
            },
            Param::Gain(gain) => Parameter {
                name,
                value: gain.to_string(),
            },
        }
    }

    const fn as_name(&self) -> &'static str {
        match self {
            Param::BufferSize(_) => "buffer_size",
            Param::Gain(_) => "gain",
        }
    }
}

struct Config {
    buffer_size: usize,
    gain: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            buffer_size: 10, // デフォルト値
            gain: 1.0,       // デフォルト値
        }
    }
}

impl Config {
    fn new(params: Vec<Parameter>) -> Result<Self, String> {
        let mut config = Config::default();

        for param in params {
            match Param::from_parameter(&param)? {
                Param::BufferSize(size) => config.buffer_size = size,
                Param::Gain(g) => config.gain = g,
            }
        }
        Ok(config)
    }

    fn get_param(&self, name: &str) -> Result<Parameter, String> {
        match name {
            "buffer_size" => Ok(Param::BufferSize(self.buffer_size).as_parameter()),
            "gain" => Ok(Param::Gain(self.gain).as_parameter()),
            _ => Err(format!("Unknown parameter: {}", name)),
        }
    }

    fn as_parameters(&self) -> Vec<Parameter> {
        vec![
            Param::BufferSize(self.buffer_size).as_parameter(),
            Param::Gain(self.gain).as_parameter(),
        ]
    }
}

struct ProcessorInner {
    buffer: VecDeque<i16>,
    config: Config,
}

impl ProcessorInner {
    fn new(config: Config) -> Self {
        ProcessorInner {
            buffer: VecDeque::from(vec![0; config.buffer_size]),
            config,
        }
    }

    fn process(&mut self, input: Value) -> Value {
        let res = self.buffer.pop_front().unwrap_or(0);
        self.buffer.push_back(input);
        // bufferした後に処理するという仕様
        (res as f32 * self.config.gain) as i16
    }

    fn set(&mut self, param: Param) -> Result<Param, String> {
        let res = match param {
            Param::BufferSize(size) => {
                if size == 0 {
                    return Err("Buffer size cannot be zero".to_string());
                }
                self.config.buffer_size = size;
                self.buffer = VecDeque::from(vec![0; size]);
                Param::BufferSize(size)
            }
            Param::Gain(gain) => {
                self.config.gain = gain;
                Param::Gain(gain)
            }
        };
        Ok(res)
    }
}

struct GuestProcessorImpl {
    inner: RefCell<ProcessorInner>,
}

impl GuestProcessor for GuestProcessorImpl {
    fn new(init: Vec<Parameter>) -> Self {
        let config = Config::new(init).unwrap_or_default();
        GuestProcessorImpl {
            inner: RefCell::new(ProcessorInner::new(config)), // 例として10sample Delayを実装
        }
    }

    fn process(&self, input: Single) -> Value {
        let mut inner = self.inner.borrow_mut();
        inner.process(input.data)
    }

    fn parameters() -> Vec<Parameter> {
        Config::default().as_parameters()
    }

    fn get(&self, name: String) -> Result<Parameter, String> {
        let inner = self.inner.borrow();
        inner.config.get_param(&name)
    }

    fn set(&self, param: Parameter) -> Result<Parameter, String> {
        let mut inner = self.inner.borrow_mut();
        let param = Param::from_parameter(&param)?;
        inner.set(param).map(|p| p.as_parameter())
    }
}

struct Component;

impl SingleChannelGuest for Component {
    type Processor = GuestProcessorImpl;
}

impl Guest for Component {
    fn plugin_name() -> String {
        "delay".to_string()
    }

    fn plugin_version() -> String {
        "0.1.0".to_string()
    }
}

bindings::export!(Component with_types_in bindings);
