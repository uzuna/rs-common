use clap::Parser;
use std::{error::Error, path::PathBuf};
use wasmtime::{component::Component, *};
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
wasmtime::component::bindgen!(in "../../crates/wasm-plugin-hello/wit/world.wit");

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

    fn call_add(&mut self, a: u32, b: u32) -> Result<u32, Box<dyn Error>> {
        let e = Example::instantiate(&mut self.store, &self.component, &self.linker)?;
        let res = e.call_add(&mut self.store, a, b)?;
        Ok(res)
    }

    fn call_hello_world(&mut self) -> Result<String, Box<dyn Error>> {
        let e = Example::instantiate(&mut self.store, &self.component, &self.linker)?;
        let res = e.call_hello_world(&mut self.store)?;
        Ok(res)
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
    let mut c = WasmComponent::new_unknown(engine, byte, ())?;
    let res = c.call_hello_world()?;
    println!("Hello from WASI Preview1: {}", res);

    for i in 0..5 {
        let result = c.call_add(i, i)?;
        println!("add({i}+{i}) = {result}");
    }

    Ok(())
}

// WASI Preview2の実装で実行する関数
fn run_on_wasi_preview2(engine: &Engine, byte: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut c = WasmComponent::new_p2(engine, byte)?;
    let res = c.call_hello_world()?;
    println!("Hello from WASI Preview2: {}", res);

    for i in 0..5 {
        let result = c.call_add(i, i)?;
        println!("add({i}+{i}) = {result}");
    }

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
