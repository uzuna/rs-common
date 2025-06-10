// crate rootからの相対パスで指定
wasmtime::component::bindgen!(in "../../wits/dsp/wit/dsp.wit");

use crate::wrun::ExecStore;
use exports::local::dsp::single_channel::Parameter;
pub use exports::local::dsp::single_channel::Single;
use wasmtime::{
    component::{Component, ResourceAny},
    Store,
};

pub struct SingleInst<T> {
    instance: Dsp,
    store: Store<T>,
    r: ResourceAny,
}

impl<T> SingleInst<T> {
    pub fn new_with_binary(es: ExecStore<T>, component: &Component) -> anyhow::Result<Self> {
        let ExecStore { mut store, linker } = es;
        let e = Dsp::instantiate(&mut store, component, &linker)?;
        let g = e.local_dsp_single_channel();
        let r = g.processor();
        let r = r.call_constructor(&mut store, &[])?;
        Ok(Self::new(e, r, store))
    }

    pub fn new(instance: Dsp, r: ResourceAny, store: Store<T>) -> Self {
        Self { instance, r, store }
    }

    pub fn name(&mut self) -> anyhow::Result<String> {
        self.instance.call_plugin_name(&mut self.store)
    }

    pub fn process(&mut self, input: Single) -> anyhow::Result<i16> {
        let res = self.instance.local_dsp_single_channel();
        let caller = res.processor();
        let res = caller.call_process(&mut self.store, self.r, input)?;
        Ok(res)
    }

    pub fn single(elapsed: u64, data: i16) -> Single {
        Single { elapsed, data }
    }

    pub fn set_parameter(&mut self, key: &str, value: &str) -> anyhow::Result<()> {
        let res = self.instance.local_dsp_single_channel();
        let caller = res.processor();
        let param = Parameter {
            name: key.to_string(),
            value: value.to_string(),
        };
        let c = caller.call_set(&mut self.store, self.r, &param)?;
        match c {
            Ok(_next) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Failed to set parameter: {}", e)),
        }
    }

    pub fn get_parameter(&mut self, name: &str) -> anyhow::Result<String> {
        let res = self.instance.local_dsp_single_channel();
        let caller = res.processor();
        let res = caller.call_get(&mut self.store, self.r, name)?;
        match res {
            Ok(param) => Ok(param.value),
            Err(e) => Err(anyhow::anyhow!("Failed to set parameter: {}", e)),
        }
    }
}

pub struct PluginLoader {
    engine: wasmtime::Engine,
}

impl PluginLoader {
    pub fn load_plugin(&self, buffer: &[u8]) -> anyhow::Result<SingleInst<()>> {
        // プラグインのコンポーネントを読み込む
        let component = wasmtime::component::Component::from_binary(&self.engine, buffer)?;
        let es = ExecStore::new_core(&self.engine, ());
        SingleInst::new_with_binary(es, &component)
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        let engine = wasmtime::Engine::default();
        Self { engine }
    }
}
