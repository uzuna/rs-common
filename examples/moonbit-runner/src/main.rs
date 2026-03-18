//! MoonBitプラグインの実行ホスト

use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;
use wasmtime::component::Component;

mod bindings;
mod context;

/// MoonBitプラグインランナー
#[derive(Debug, clap::Parser)]
struct Opt {
    /// 実行するWasmコンポーネントファイルのパス
    #[arg(short, long)]
    wasm: PathBuf,
}

fn run(path: &std::path::Path) -> anyhow::Result<()> {
    let engine = wasmtime::Engine::default();

    let buffer = std::fs::read(path)
        .with_context(|| format!("Wasmファイルの読み込み失敗: {}", path.display()))?;

    let component = Component::new(&engine, &buffer)
        .map_err(|e| anyhow::anyhow!("Componentの生成失敗: {}: {}", path.display(), e))?;

    let es = context::ExecStore::new(&engine);
    let mut inst = bindings::PluginInst::new_with_binary(es, &component)?;

    // 初期ステータスを確認する
    let status = inst.get_status()?;
    println!(
        "初期ステータス: running={}, error_code={}, temperature={}",
        status.running, status.error_code, status.temperature
    );

    // 仮入力データでupdateを呼び出す
    let input = vec![bindings::SensorData {
        load: 1.0,
        position: 0.0,
        extra: None,
    }];
    let output = inst.update(&input)?;
    for out in &output {
        println!("output: position={}, torque={}", out.position, out.torque);
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    run(&opt.wasm)
}
