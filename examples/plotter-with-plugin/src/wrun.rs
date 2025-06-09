use wasmtime::{Engine, Store};

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
