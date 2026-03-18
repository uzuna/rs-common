//! Wasmtime実行環境の構成を行う

use wasmtime::{Engine, Store};

/// WASIなしの実行コンテキスト
///
/// MoonBitプラグインはwasm32-unknown-unknownターゲットでビルドするためWASI不要。
pub struct ExecStore {
    /// Wasmインスタンスの状態を保持するストア
    pub store: Store<()>,
    /// コンポーネントのリンクと名前解決を担うリンカー
    pub linker: wasmtime::component::Linker<()>,
}

impl ExecStore {
    /// WASIなしの実行コンテキストを生成する
    pub fn new(engine: &Engine) -> Self {
        let store = Store::new(engine, ());
        let linker = wasmtime::component::Linker::new(engine);
        Self { store, linker }
    }
}
