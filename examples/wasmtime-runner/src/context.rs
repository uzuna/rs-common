//! WASM実行環境の構成を行う

use wasmtime::{Engine, Store};
use wasmtime_wasi::{
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
    ResourceTable,
};

/// WASIリンクに必要なトレイと実装構造体
///
/// wasip2でコンパイルした場合、実行環境は[WASIp2 interface](https://docs.wasmtime.dev/api/wasmtime_wasi/p2/index.html)を実装している必要がある
/// この実装を保持して提供する役目がある。
pub struct Preview2Host {
    wasi_ctx: WasiCtx,
    resource_table: ResourceTable,
}

impl Default for Preview2Host {
    fn default() -> Self {
        let wasi_ctx = WasiCtxBuilder::new().inherit_stdio().build();
        let resource_table = ResourceTable::default();
        Self {
            wasi_ctx,
            resource_table,
        }
    }
}

/// [WasiView]は[IoView]トレイトを前提としている
impl IoView for Preview2Host {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.resource_table
    }
}

/// [wasmtime_wasi::p2::add_to_linker_sync]を実行するためには[WasiView]トレイトの実装が必要
impl WasiView for Preview2Host {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi_ctx
    }
}

pub struct ExecStore<T> {
    // StoreはWasmインスタンスとホスト定義の状態のコレクション
    // すべてのインsタンスとアイテムはstoreニアタッチされてそれを参照する。実態はここに有り。
    // プログラム内の短命なオブジェクトとして使う意図で設計されており、GCもないため明示的な削除まで開放されない
    // A内に無制限の数のインスタンスを作成することは想定していないので、実行したいメインインスタンスと同じ有効期限になるような使い方を推奨している
    // StoreはComponentのインスタンスごとに作る
    pub store: Store<T>,
    // Componentをインスタンス化するための型
    // コンポーネントの相互リンクやホスト機能のために使用される
    // 値はインポート名によって定義されて、名前解決を使用してインスタンス化される。
    // ここからLinkerInstanceを取得してfunc_wrapなどを通してホスト関数を定義する
    // 別のguest関数定義方法はよくわからない...
    // linker: wasmtime::component::Linker<T>,
    pub linker: wasmtime::component::Linker<T>,
}

impl<T> ExecStore<T> {
    // WASIなしで実行する場合のコンポーネントを生成
    pub fn new_core(engine: &Engine, data: T) -> Self {
        let store = Store::new(engine, data);
        let linker = wasmtime::component::Linker::new(engine);
        Self { store, linker }
    }
}

impl ExecStore<Preview2Host> {
    // WASI Preview2のインターフェースを持つ実行コンテキスト
    pub fn new_p2(engine: &Engine) -> anyhow::Result<Self> {
        let store = Store::new(engine, Preview2Host::default());
        let mut linker = wasmtime::component::Linker::new(engine);
        // 重複読み出しを許可
        linker.allow_shadowing(true);
        // WASIp2向けのインターフェースをリンカーに追加
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        Ok(Self { store, linker })
    }
}
