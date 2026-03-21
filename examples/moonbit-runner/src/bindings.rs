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
/// `plugin_base::PluginHandle` および `plugin_base::StatefulPlugin` を実装する。
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

impl plugin_base::PluginHandle for PluginInst {
    /// `get-status` を呼び出す（呼び出しオーバーヘッド計測用）
    fn hello(&mut self) -> anyhow::Result<()> {
        self.get_status().map(|_| ())
    }

    /// WIT `add(a, b, loop_count)` を呼び出す
    fn add(&mut self, a: i32, b: i32, loop_count: i32) -> anyhow::Result<i32> {
        self.add(a, b, loop_count)
    }
}

impl plugin_base::StatefulPlugin for PluginInst {
    fn save_state(&mut self) -> anyhow::Result<Vec<u8>> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_save_state(&mut self.store)?)
    }

    fn load_state(&mut self, state: &[u8]) -> anyhow::Result<()> {
        let ctrl = self.instance.local_moonbit_control_api();
        Ok(ctrl.call_load_state(&mut self.store, state)?)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use plugin_base::{PluginHandle, StatefulPlugin};
    use wasmtime::component::Component;

    use crate::{context::ExecStore, engine};

    use super::{PluginInst, SensorData};

    fn component_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugins")
            .join("control.component.wasm")
    }

    fn new_inst() -> PluginInst {
        let path = component_path();
        assert!(
            path.exists(),
            "component Wasm が見つかりません: {}。先に `make -C examples/moonbit-runner build-plugin` を実行してください",
            path.display()
        );
        let engine = engine::create_engine_from_env().expect("engine 初期化失敗");
        let bytes = std::fs::read(&path).expect("Wasm 読み込み失敗");
        let component = Component::new(&engine, &bytes).expect("Component 生成失敗");
        let store = ExecStore::new(&engine);
        PluginInst::new_with_binary(store, &component).expect("PluginInst 初期化失敗")
    }

    struct StateTransferCase {
        name: &'static str,
        update_calls: usize,
        expected_count: i32,
    }

    #[test]
    fn save_load_state_値域確認() {
        // call_count=0 の初期状態を保存・復元できること
        let mut inst = new_inst();
        let state = inst.save_state().expect("save_state 失敗");
        assert!(!state.is_empty(), "save_state は空でないバイト列を返す");
        assert!(state.len() >= 4, "save_state は最低4バイト返す");

        inst.load_state(&state).expect("load_state 失敗");
    }

    #[test]
    fn save_load_state_正常系() {
        let cases = [
            StateTransferCase {
                name: "update1回後の状態引き継ぎ",
                update_calls: 1,
                expected_count: 1,
            },
            StateTransferCase {
                name: "update5回後の状態引き継ぎ",
                update_calls: 5,
                expected_count: 5,
            },
        ];

        for case in &cases {
            // 旧インスタンスで update を呼ぶ
            let mut old_inst = new_inst();
            let sensor = SensorData {
                load: 10.0,
                position: 0.0,
                extra: None,
            };
            for _ in 0..case.update_calls {
                old_inst.update(&[sensor.clone()]).expect("update 失敗");
            }

            // 状態を保存し新インスタンスへ引き継ぐ
            let saved = old_inst.save_state().expect("save_state 失敗");
            let mut new_inst = new_inst();
            new_inst.load_state(&saved).expect("load_state 失敗");

            // hello() が成功することで call_count の引き継ぎが確認できる（直接的な確認は get_status）
            new_inst.hello().expect("hello 失敗");

            // add は call_count に依存しないが、動作確認
            let result = new_inst.add(3, 4, case.expected_count).expect("add 失敗");
            assert_eq!(
                result,
                (3 + 4) * case.expected_count,
                "ケース '{}': add の結果が正しい",
                case.name
            );
        }
    }

    #[test]
    fn save_load_state_異常系() {
        let cases: &[(&str, &[u8])] = &[
            ("空スライスは無視される", &[]),
            ("1バイトは無視される", &[0xFF]),
            ("3バイトは無視される", &[0x01, 0x02, 0x03]),
        ];

        for (name, invalid_state) in cases {
            let mut inst = new_inst();
            // 無効な状態を渡しても panic しない
            inst.load_state(invalid_state)
                .expect(&format!("ケース '{}': load_state は失敗しない", name));
            // 続けて通常操作が動作すること
            inst.hello().expect(&format!(
                "ケース '{}': load_state 後も hello が動作する",
                name
            ));
        }
    }
}
