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
    fn get_func(&mut self, name: &str) -> Result<wasmtime::component::Func, wasmtime::Error> {
        let instance = self.linker.instantiate(&mut self.store, &self.component)?;
        instance
            .get_func(&mut self.store, name)
            .ok_or(wasmtime::Error::msg(format!(
                "Function `{}` not found in component",
                name
            )))
    }

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
    let res = call_hello_world(&mut c)?;
    println!("Hello from WASI Preview1: {}", res);

    for i in 0..5 {
        let result = call_add_func(&mut c, i, i)?;
        println!("add({i}+{i}) = {result}");
    }

    Ok(())
}

// WASI Preview2の実装で実行する関数
fn run_on_wasi_preview2(engine: &Engine, byte: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut c = WasmComponent::new_p2(engine, byte)?;
    let res = call_hello_world(&mut c)?;
    println!("Hello from WASI Preview2: {}", res);

    for i in 0..5 {
        let result = call_add_func(&mut c, i, i)?;
        println!("add({i}+{i}) = {result}");
    }

    Ok(())
}

// hello-world関数を呼び出す
fn call_hello_world<T>(wc: &mut WasmComponent<T>) -> Result<String, Box<dyn Error>> {
    let func = wc.get_func("hello-world")?;
    let mut result = [wasmtime::component::Val::String("".into())];
    func.call(&mut wc.store, &[], &mut result)?;
    match &result[0] {
        wasmtime::component::Val::String(s) => Ok(s.to_owned()),
        _ => unreachable!("Expected a string result"),
    }
}

// add関数を呼び出す
fn call_add_func<T>(wc: &mut WasmComponent<T>, a: u32, b: u32) -> Result<u32, Box<dyn Error>> {
    let func = wc.get_func("add")?;
    let mut result = [wasmtime::component::Val::U32(0)];
    func.call(
        &mut wc.store,
        &[
            wasmtime::component::Val::U32(a),
            wasmtime::component::Val::U32(b),
        ],
        &mut result,
    )?;
    match result[0] {
        wasmtime::component::Val::U32(res) => Ok(res),
        _ => unreachable!("Expected a u32 result"),
    }
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
