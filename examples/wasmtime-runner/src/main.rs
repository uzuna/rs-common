use clap::Parser;
use context::{run_sequence_hello, WasmComponent};
use std::path::PathBuf;
use wasmtime::*;

pub mod bingings;
pub mod context;

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

// WASIなし = storeなし
fn run_on_wasi(engine: &Engine, byte: &[u8]) -> anyhow::Result<()> {
    let c = WasmComponent::new_unknown(engine, byte, ())?;
    run_sequence_hello(c)
}

// WASI Preview2の実装で実行する関数
fn run_on_wasi_preview2(engine: &Engine, byte: &[u8]) -> anyhow::Result<()> {
    let c = WasmComponent::new_p2(engine, byte)?;
    run_sequence_hello(c)
}

fn main() -> anyhow::Result<()> {
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
