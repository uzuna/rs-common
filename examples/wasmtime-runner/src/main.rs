use clap::Parser;
use exports::local::hello::types::Pos2;
use std::{error::Error, path::PathBuf};
use wasmtime::{
    component::{Component, ResourceAny},
    *,
};
use wasmtime_wasi::{
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
    ResourceTable,
};

#[derive(Debug, Clone, clap::Parser)]
struct Opt {
    #[arg(short, long, default_value = "hello.wat")]
    pub name: PathBuf,

    #[arg(long, default_value = "preview2")]
    pub wasi: WasiSupport,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[clap(rename_all = "snake_case")]
enum WasiSupport {
    None,
    Preview2,
}

// crate rootからの相対パスで指定
wasmtime::component::bindgen!(in "../../wasm-comp/hello/wit/world.wit");

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

// インスタンスを作るタイプは、関数呼び出しと違ってインスタンスを使い回さなければならない
struct SetterWrap {
    instance: Example,
    setter: ResourceAny,
}

impl SetterWrap {
    fn get<T>(&self, store: &mut Store<T>) -> Result<Pos2, Box<dyn Error>> {
        let g = self.instance.local_hello_types();
        let caller = g.setter();
        let res = caller.call_get(store, self.setter)?;
        Ok(res)
    }

    fn set<T>(&self, store: &mut Store<T>, p: Pos2) -> Result<(), Box<dyn Error>> {
        let g = self.instance.local_hello_types();
        let caller = g.setter();
        caller.call_set(store, self.setter, p)?;
        Ok(())
    }

    // 自動でドロップされない
    fn drop<T>(self, store: &mut Store<T>) -> Result<(), Box<dyn Error>> {
        self.setter.resource_drop(store)?;
        Ok(())
    }
}

struct WasmComponent<T> {
    store: Store<T>,
    component: Component,
    linker: wasmtime::component::Linker<T>,
}

impl<T> WasmComponent<T> {
    // WASIなしで実行する場合のコンポーネントを生成
    fn new_unknown(engine: &Engine, byte: &[u8], data: T) -> Result<Self, Box<dyn Error>> {
        // WASIなしで実行する場合の設定
        let store = Store::new(engine, data);
        let component = Component::new(engine, byte)?;
        let linker = wasmtime::component::Linker::new(engine);
        Ok(Self {
            store,
            component,
            linker,
        })
    }

    fn instance(&mut self) -> Result<Example, Box<dyn Error>> {
        // コンポーネントをインスタンス化
        let res = Example::instantiate(&mut self.store, &self.component, &self.linker)?;
        Ok(res)
    }

    fn setter(&mut self) -> Result<SetterWrap, Box<dyn Error>> {
        let e = Example::instantiate(&mut self.store, &self.component, &self.linker)?;
        let g = e.local_hello_types();
        let caller = g.setter();
        let setter = caller.call_new(&mut self.store)?;
        let sw = SetterWrap {
            instance: e,
            setter,
        };

        Ok(sw)
    }
}

impl WasmComponent<Preview2Host> {
    fn new_p2(engine: &Engine, byte: &[u8]) -> Result<Self, Box<dyn Error>> {
        // WASI Preview2のインターフェースを追加
        let store = Store::new(engine, Preview2Host::default());
        let component = Component::new(engine, byte)?;
        let mut linker = wasmtime::component::Linker::new(engine);
        // 重複読み出しを許可
        linker.allow_shadowing(true);
        // WASIp2向けのインターフェースをリンカーに追加
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

        Ok(Self {
            store,
            component,
            linker,
        })
    }
}

// WASIなし = storeなし
fn run_on_wasi(engine: &Engine, byte: &[u8]) -> Result<(), Box<dyn Error>> {
    let c = WasmComponent::new_unknown(engine, byte, ())?;
    run_sequence(c)
}

// WASI Preview2の実装で実行する関数
fn run_on_wasi_preview2(engine: &Engine, byte: &[u8]) -> Result<(), Box<dyn Error>> {
    let c = WasmComponent::new_p2(engine, byte)?;
    run_sequence(c)
}

fn run_sequence<T>(mut c: WasmComponent<T>) -> Result<(), Box<dyn Error>> {
    let e = c.instance()?;
    let res = e.call_hello_world(&mut c.store)?;
    println!("Hello from WASI Preview1: {}", res);

    for i in 0..5 {
        let result = e.call_add(&mut c.store, i, i)?;
        println!("add({i}+{i}) = {result}");
    }

    let s = e.call_sum(&mut c.store, &[1, 2, 3, 4, 5])?;
    println!("sum([1, 2, 3, 4, 5]) = {}", s);

    let sw = c.setter()?;
    let res = sw.get(&mut c.store)?;
    println!("setter.get() = {:?}", res);
    sw.set(&mut c.store, Pos2 { x: 1.0, y: 2.0 })?;
    let get = sw.get(&mut c.store)?;
    println!("setter.get() = {:?}", get);
    sw.drop(&mut c.store)?;

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let Opt { name, wasi } = Opt::parse();

    let wasm_binaly = std::fs::read(name)?;
    // wasmtimeのエンジンを初期化
    let engine = Engine::default();

    match wasi {
        WasiSupport::None => {
            // WASIなしで実行
            println!("Running without WASI support");
            run_on_wasi(&engine, &wasm_binaly)?;
        }
        WasiSupport::Preview2 => {
            // WASI Preview2を使用する場合の設定
            println!("Running with WASI Preview2 support");
            run_on_wasi_preview2(&engine, &wasm_binaly)?;
        }
    }
    Ok(())
}
