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

fn main() -> Result<(), Box<dyn Error>> {
    let Opt { name } = Opt::parse();
    // wasmtimeのエンジンを初期化
    let engine = Engine::default();
    // 実行情報を保持するストアを定義
    let mut store = Store::new(&engine, Preview2Host::default());
    // コンポーネントの読み込み
    let component = Component::from_file(&engine, &name)?;
    // リンカーを作成し、WASIのインターフェースを追加
    let mut linker = wasmtime::component::Linker::new(&engine);
    // 重複読み出しを許可
    linker.allow_shadowing(true);
    // WASIp2向けのインターフェースをリンカーに追加
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

    let instance = linker.instantiate(&mut store, &component)?;
    let func = instance
        .get_func(&mut store, "hello-world")
        .expect("`hello-world` was not an exported function");
    let mut result = [wasmtime::component::Val::String("".into())];
    func.call(&mut store, &[], &mut result)?;

    println!("Answer: {:?}", result);

    for i in 0..5 {
        // インスタンスは1ど束縛したら再代入できない。都度作成が必要
        let instance = linker.instantiate(&mut store, &component)?;
        let addfunc = instance
            .get_func(&mut store, "add")
            .expect("`add` was not an exported function");
        let mut result = [wasmtime::component::Val::U32(0)];
        addfunc.call(
            &mut store,
            &[
                wasmtime::component::Val::U32(i),
                wasmtime::component::Val::U32(i),
            ],
            &mut result,
        )?;
        println!("add({i}+{i}) = {:?}", result[0]);
    }

    Ok(())
}
