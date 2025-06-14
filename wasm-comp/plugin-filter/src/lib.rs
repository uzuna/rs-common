use core::f32;
use std::{cell::RefCell, vec};

use crate::bindings::{
    exports::local::dsp::single_channel::{
        Guest as SingleChannelGuest, GuestProcessor, Parameter, Single, Value,
    },
    Guest,
};

#[allow(warnings)]
mod bindings;

enum Param {
    SampleRate(f32),
    Tc(f32),
}

impl Param {
    fn parse<T>(v: &str) -> Result<T, String>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        v.parse::<T>()
            .map_err(|e| format!("Failed to parse '{}': {}", v, e))
    }

    fn from_parameter(param: &Parameter) -> Result<Self, String> {
        match param.name.as_str() {
            "sample_rate" => Ok(Param::SampleRate(Self::parse::<f32>(&param.value)?)),
            "t_c" => Ok(Param::Tc(Self::parse::<f32>(&param.value)?)),
            _ => Err(format!("Unknown parameter: {}", param.name)),
        }
    }

    fn as_parameter(&self) -> Parameter {
        let name = self.as_name().to_string();
        match self {
            Param::SampleRate(value) => Parameter {
                name,
                value: value.to_string(),
            },
            Param::Tc(value) => Parameter {
                name,
                value: value.to_string(),
            },
        }
    }

    const fn as_name(&self) -> &'static str {
        match self {
            Param::SampleRate(_) => "sample_rate",
            Param::Tc(_) => "t_c",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Config {
    sample_rate: f32,
    time_constant: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            sample_rate: 60.0,
            time_constant: 0.066,
        }
    }
}

impl Config {
    fn new(params: Vec<Parameter>) -> Result<Self, String> {
        let mut config = Config::default();

        for param in params {
            match Param::from_parameter(&param)? {
                Param::SampleRate(value) => {
                    if value <= 0.0 {
                        return Err("Sample rate must be positive".to_string());
                    }
                    config.sample_rate = value;
                }
                Param::Tc(value) => {
                    if value <= 0.0 {
                        return Err("Q factor must be positive".to_string());
                    }
                    config.time_constant = value;
                }
            }
        }
        Ok(config)
    }

    fn apply(&mut self, param: Param) -> Result<Param, String> {
        match param {
            Param::SampleRate(value) => {
                if value <= 0.0 {
                    return Err("Sample rate must be positive".to_string());
                }
                self.sample_rate = value;
            }
            Param::Tc(value) => {
                if value <= 0.0 {
                    return Err("time_constant must be positive".to_string());
                }
                self.time_constant = value;
            }
        }
        Ok(param)
    }

    fn get_param(&self, name: &str) -> Result<Parameter, String> {
        match name {
            "sample_rate" => Ok(Param::SampleRate(self.sample_rate).as_parameter()),
            "t_c" => Ok(Param::Tc(self.time_constant).as_parameter()),
            _ => Err(format!("Unknown parameter: {}", name)),
        }
    }

    fn as_parameters(&self) -> Vec<Parameter> {
        vec![
            Param::SampleRate(self.sample_rate).as_parameter(),
            Param::Tc(self.time_constant).as_parameter(),
        ]
    }
}

// 一次後退差分の実装
// reference: https://aisumegane.com/converting-lpfs-to-discrete-systems/
struct BackwardDifference {
    a: f32,
    b: f32,
}

impl BackwardDifference {
    fn low_pass(config: &Config) -> Self {
        let tc = config.time_constant;
        let t = 1.0 / config.sample_rate;
        let a = tc / (tc + t);
        let b = t / (tc + t);
        BackwardDifference { a, b }
    }

    fn process(&self, input: f32, o1: f32) -> f32 {
        self.a * o1 + self.b * input
    }
}

struct ProcessorInner {
    config: Config,
    param: BackwardDifference,
    out_prev: f32,
}

impl ProcessorInner {
    fn new(config: Config) -> Self {
        let param = BackwardDifference::low_pass(&config);
        ProcessorInner {
            config,
            param,
            out_prev: 0.0,
        }
    }

    fn process(&mut self, input: Value) -> Value {
        let input = input as f32;
        let out = self.param.process(input, self.out_prev);
        self.out_prev = out;
        out as i16
    }

    fn set(&mut self, param: Param) -> Result<Param, String> {
        let res = self.config.apply(param)?;
        self.param = BackwardDifference::low_pass(&self.config);
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
        "lowpass-bd".to_string()
    }

    fn plugin_version() -> String {
        "0.1.0".to_string()
    }
}

bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = Config::default();
        let mut p = ProcessorInner::new(config);

        let inbuf = vec![128.0_f32; 60];
        let mut outbuf = vec![0.0; 60];
        for (i, &input) in inbuf.iter().enumerate() {
            let output = p.process(input as i16);
            outbuf[i] = output as f32;
        }
        println!("Output: {:?}", outbuf);
    }
}
