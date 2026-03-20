//! WITバインディングとプラグイン呼び出しラッパー

use wasmtime::component::Component;

use crate::context::ExecStore;

// WIT定義からRust型とインスタンス化コードを生成する
// 生成されるworld名 `plugin` → Plugin 構造体
// インターフェースアクセサ: instance.local_moonbit_control_api()
wasmtime::component::bindgen!(in "wit/world.wit");

pub use exports::local::moonbit_control::api::{
    BenchmarkInput128, BenchmarkInput1k, BenchmarkInput4k, BenchmarkOutput128, BenchmarkOutput1k,
    BenchmarkOutput4k, MotorOutput, PluginStatus, SensorData,
};

/// MoonBitプラグインの呼び出しラッパー
///
/// `update` / `get-status` を型安全に呼び出す責務を持つ。
pub struct PluginInst {
    instance: Plugin,
    store: wasmtime::Store<()>,
}

impl PluginInst {
    /// Wasmコンポーネントからプラグインインスタンスを生成する
    pub fn new_with_binary(es: ExecStore, component: &Component) -> anyhow::Result<Self> {
        let ExecStore { mut store, linker } = es;
        let instance = Plugin::instantiate(&mut store, component, &linker)?;
        Ok(Self { instance, store })
    }

    /// センサーデータを入力してモーター出力を取得する
    pub fn update(&mut self, input: &[SensorData]) -> anyhow::Result<Vec<MotorOutput>> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_update(&mut self.store, input)?)
    }

    /// プラグインの内部状態を取得する
    pub fn get_status(&mut self) -> anyhow::Result<PluginStatus> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_get_status(&mut self.store)?)
    }

    /// 純粋な計算負荷を測る加算関数を呼び出す
    pub fn add(&mut self, a: i32, b: i32, loop_count: i32) -> anyhow::Result<i32> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_add(&mut self.store, a, b, loop_count)?)
    }

    /// 128バイトの benchmark 入出力関数を呼び出す
    #[allow(dead_code)]
    pub fn benchmark_128(
        &mut self,
        input: &BenchmarkInput128,
    ) -> anyhow::Result<BenchmarkOutput128> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_benchmark_128(&mut self.store, input)?)
    }

    /// 1KBの benchmark 入出力関数を呼び出す
    #[allow(dead_code)]
    pub fn benchmark_1k(&mut self, input: &BenchmarkInput1k) -> anyhow::Result<BenchmarkOutput1k> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_benchmark_1k(&mut self.store, input)?)
    }

    /// 4KBの benchmark 入出力関数を呼び出す
    #[allow(dead_code)]
    pub fn benchmark_4k(&mut self, input: &BenchmarkInput4k) -> anyhow::Result<BenchmarkOutput4k> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_benchmark_4k(&mut self.store, input)?)
    }
}
