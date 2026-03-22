//! WASMプラグインのライフサイクルを管理するモジュール。
//!
//! [`PluginManager`] が単一プラグインのロード・アンロード・リロード・状態引き継ぎを担当する。
//!
//! # ライフタイム管理
//!
//! [`LoadedPlugin`] が `component`（コンパイル済みWasm）と `inst`（実行インスタンス）を
//! 一緒に保持する。`component` は `inst` より後に drop されないよう、常に同じ構造体に置く。
//!
//! # リロードシーケンス
//!
//! ```text
//! 1. current.inst.save_state()  → saved_state に保存
//! 2. LoadedPlugin を drop       → Store + Component を解放
//! 3. 新 .wasm をロード          → Component::new → PluginInst::new_with_binary
//! 4. inst.load_state(saved)     → 旧状態を復元
//! 5. 失敗時は旧パスで再ロード   → 旧インスタンスで継続（saved_state は保持）
//! ```

use std::path::{Path, PathBuf};

use anyhow::Context;
use plugin_base::StatefulPlugin;
use wasmtime::component::Component;

use crate::{
    bindings::{MotorOutput, PluginInst, PluginStatus, SensorData},
    context::ExecStore,
    engine,
};

/// ロード済みプラグインの情報。
struct LoadedPlugin {
    /// コンパイル済み Wasm コンポーネント。`inst` のライフタイム確保のため保持する。
    #[allow(dead_code)]
    component: Component,
    /// 実行インスタンス（Store + Plugin）。
    inst: PluginInst,
    /// ロード元のパス。リロード失敗時の旧バージョン復帰に使う。
    wasm_path: PathBuf,
}

/// 単一 Wasm プラグインのライフサイクルを管理する。
///
/// ルーティングは呼び出し元が担当する。このクラスはロード・リロード・
/// 状態引き継ぎのみを管理する。
pub struct PluginManager {
    /// Wasmtime エンジン（リロードをまたいで再利用する）。
    engine: wasmtime::Engine,
    /// 現在ロードされているプラグイン。未ロード時は `None`。
    current: Option<LoadedPlugin>,
    /// 前回の `save_state` が返した状態バイト列。次の `load_state` に渡す。
    saved_state: Option<Vec<u8>>,
    /// プラグイン未ロード状態でのリクエスト数（デバッグ・監視用）。
    pub fallback_count: u64,
}

impl PluginManager {
    /// 新しい `PluginManager` を生成する。
    pub fn new() -> anyhow::Result<Self> {
        let engine = engine::create_engine_from_env()?;
        Ok(Self {
            engine,
            current: None,
            saved_state: None,
            fallback_count: 0,
        })
    }

    /// 指定パスのプラグインをロードする。
    ///
    /// すでにプラグインがロードされている場合は先に `save_state` してからロードする。
    pub fn load(&mut self, path: &Path) -> anyhow::Result<()> {
        self.save_current_state();
        self.load_internal(path)
    }

    /// 動作中に新しいプラグインへ切り替える（ホットリロード）。
    ///
    /// 新バイナリのロードに失敗した場合は旧バイナリで再起動する。
    /// この場合でも `Err` を返すが、旧プラグインで動作を継続する。
    pub fn reload(&mut self, new_path: &Path) -> anyhow::Result<()> {
        let old_path = self.current.as_ref().map(|l| l.wasm_path.clone());
        self.save_current_state();

        match self.load_internal(new_path) {
            Ok(()) => {
                tracing::info!("リロード成功: {}", new_path.display());
                Ok(())
            }
            Err(e) => {
                tracing::error!("リロード失敗 ({}): {e:#}", new_path.display());
                if let Some(old) = old_path {
                    tracing::info!("旧バージョンで再起動: {}", old.display());
                    if let Err(e2) = self.load_internal(&old) {
                        tracing::error!("旧バージョンの再起動にも失敗: {e2:#}");
                    }
                }
                Err(e)
            }
        }
    }

    /// 現在のプラグインをアンロードする。`saved_state` もクリアする。
    pub fn unload(&mut self) {
        self.save_current_state();
        self.current = None;
        self.saved_state = None;
    }

    /// センサーデータを入力してモーター出力を取得する。
    ///
    /// プラグイン未ロード時は `fallback_count` を加算して `Err` を返す。
    pub fn update(&mut self, input: &[SensorData]) -> anyhow::Result<Vec<MotorOutput>> {
        match self.current.as_mut() {
            Some(loaded) => loaded.inst.update(input),
            None => {
                self.fallback_count += 1;
                anyhow::bail!("プラグイン未ロード")
            }
        }
    }

    /// プラグインの内部状態を取得する。
    ///
    /// プラグイン未ロード時は `fallback_count` を加算して `Err` を返す。
    pub fn get_status(&mut self) -> anyhow::Result<PluginStatus> {
        match self.current.as_mut() {
            Some(loaded) => loaded.inst.get_status(),
            None => {
                self.fallback_count += 1;
                anyhow::bail!("プラグイン未ロード")
            }
        }
    }

    /// 加算関数を呼び出す。
    ///
    /// プラグイン未ロード時は `fallback_count` を加算して `Err` を返す。
    pub fn add(&mut self, a: i32, b: i32, loop_count: i32) -> anyhow::Result<i32> {
        match self.current.as_mut() {
            Some(loaded) => loaded.inst.add(a, b, loop_count),
            None => {
                self.fallback_count += 1;
                anyhow::bail!("プラグイン未ロード")
            }
        }
    }

    /// プラグインがロード済みかどうかを返す。
    pub fn is_loaded(&self) -> bool {
        self.current.is_some()
    }

    /// 現在ロード中のプラグインパスを返す。
    pub fn current_path(&self) -> Option<&Path> {
        self.current.as_ref().map(|l| l.wasm_path.as_path())
    }

    /// 前回の `save_state` が返した状態バイト列を参照する（テスト・デバッグ用）。
    pub fn saved_state(&self) -> Option<&[u8]> {
        self.saved_state.as_deref()
    }

    /// 現在のインスタンスの状態をバイト列として取り出す（テスト・デバッグ用）。
    ///
    /// `saved_state` は変更しない。プラグイン未ロード時は `None` を返す。
    pub fn snapshot_current_state(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        match self.current.as_mut() {
            Some(loaded) => Ok(Some(loaded.inst.save_state()?)),
            None => Ok(None),
        }
    }

    /// 現在のプラグインの状態を保存して drop する。
    ///
    /// `save_state` に失敗した場合はエラーをログに記録し `saved_state = None` とする。
    /// 次の `load_state` にはフレッシュな状態（空スライス）が渡される。
    fn save_current_state(&mut self) {
        if let Some(mut loaded) = self.current.take() {
            match loaded.inst.save_state() {
                Ok(bytes) if !bytes.is_empty() => {
                    tracing::debug!("プラグイン状態を保存: {} バイト", bytes.len());
                    self.saved_state = Some(bytes);
                }
                Ok(_) => {
                    self.saved_state = None;
                }
                Err(e) => {
                    tracing::error!(
                        "プラグイン状態のシリアライズ失敗: {e:#}。次回はフレッシュ状態で起動します"
                    );
                    self.saved_state = None;
                }
            }
        }
    }

    /// プラグインをロードして `load_state` を呼び出す内部実装。
    fn load_internal(&mut self, path: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("Wasm ファイルの読み込み失敗: {}", path.display()))?;

        let component = Component::new(&self.engine, &bytes).map_err(|e| {
            anyhow::anyhow!("Wasm コンポーネントの生成失敗 ({}): {e:#}", path.display())
        })?;

        let store = ExecStore::new(&self.engine);
        let mut inst = PluginInst::new_with_binary(store, &component)
            .with_context(|| format!("プラグインインスタンスの初期化失敗: {}", path.display()))?;

        // 前回の状態を復元する（初回起動時は空スライスを渡す）
        let state = self.saved_state.as_deref().unwrap_or(&[]);
        inst.load_state(state)
            .with_context(|| format!("プラグイン状態の復元失敗: {}", path.display()))?;

        self.current = Some(LoadedPlugin {
            component,
            inst,
            wasm_path: path.to_owned(),
        });
        self.saved_state = None;

        tracing::info!("プラグインロード完了: {}", path.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::PluginManager;
    use crate::bindings::SensorData;

    fn component_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugins")
            .join("control.component.wasm")
    }

    fn skip_if_no_plugin() -> Option<std::path::PathBuf> {
        let path = component_path();
        if !path.exists() {
            eprintln!(
                "スキップ: Wasm が見つかりません（{}）。先に `make -C examples/moonbit-runner build-plugin` を実行してください",
                path.display()
            );
            None
        } else {
            Some(path)
        }
    }

    struct LoadCase {
        name: &'static str,
        update_count: usize,
    }

    #[test]
    fn load_値域確認() {
        let Some(path) = skip_if_no_plugin() else {
            return;
        };
        let mut mgr = PluginManager::new().expect("PluginManager 初期化失敗");
        mgr.load(&path).expect("ロード失敗");
        assert!(mgr.is_loaded(), "ロード後は is_loaded()=true");
        assert_eq!(mgr.current_path(), Some(path.as_path()), "ロードパスが一致");
        assert_eq!(mgr.fallback_count, 0, "fallback_count は 0");
    }

    #[test]
    fn load_正常系() {
        let Some(path) = skip_if_no_plugin() else {
            return;
        };

        let cases = [
            LoadCase {
                name: "update 1回",
                update_count: 1,
            },
            LoadCase {
                name: "update 3回",
                update_count: 3,
            },
        ];

        for case in &cases {
            let mut mgr = PluginManager::new().expect("PluginManager 初期化失敗");
            mgr.load(&path).expect("ロード失敗");

            let sensor = SensorData {
                load: 10.0,
                position: 0.0,
                extra: None,
            };
            for _ in 0..case.update_count {
                mgr.update(&[sensor.clone()])
                    .expect(&format!("ケース '{}': update 失敗", case.name));
            }

            let status = mgr
                .get_status()
                .expect(&format!("ケース '{}': get_status 失敗", case.name));
            assert!(status.running, "ケース '{}': running=true", case.name);
        }
    }

    #[test]
    fn load_異常系() {
        let mut mgr = PluginManager::new().expect("PluginManager 初期化失敗");

        // 存在しないパス
        let result = mgr.load(Path::new("/nonexistent/plugin.wasm"));
        assert!(result.is_err(), "存在しないパスはエラー");
        assert!(!mgr.is_loaded(), "ロード失敗後は is_loaded()=false");

        // 未ロード時の操作
        let result = mgr.update(&[]);
        assert!(result.is_err(), "未ロード時の update は Err");
        assert_eq!(
            mgr.fallback_count, 1,
            "未ロード時の update は fallback_count を加算"
        );

        let result = mgr.get_status();
        assert!(result.is_err(), "未ロード時の get_status は Err");
        assert_eq!(
            mgr.fallback_count, 2,
            "未ロード時の get_status は fallback_count を加算"
        );
    }
}
