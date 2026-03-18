//! WITバインディングとプラグイン呼び出しラッパー

use wasmtime::component::Component;

use crate::context::ExecStore;

// WIT定義からRust型とインスタンス化コードを生成する
// 生成されるworld名 `plugin` → Plugin 構造体
// インターフェースアクセサ: instance.local_moonbit_control_api()
wasmtime::component::bindgen!(in "wit/world.wit");

pub use exports::local::moonbit_control::api::{MotorOutput, PluginStatus, SensorData};

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
}
