//! MoonBitプラグインの実行ホスト

use clap::Parser;
use std::process::ExitCode;

mod bindings;
mod context;
mod runner;

/// MoonBitプラグインランナー
#[derive(Debug, Parser)]
struct Opt {
    /// 実行するWasmコンポーネントファイルのパス
    #[arg(short, long)]
    wasm: std::path::PathBuf,

    /// WASIサポートの種類
    #[arg(long, value_enum, default_value_t = runner::WasiSupport::None)]
    wasi: runner::WasiSupport,

    /// PPS計測で `update` を呼び出す回数
    #[arg(long, default_value_t = 10_000)]
    iterations: usize,
}

fn try_main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    let config = runner::RunnerConfig {
        wasm: opt.wasm,
        wasi: opt.wasi,
        benchmark_iterations: opt.iterations,
    };
    let report = runner::run(&config)?;
    runner::print_report(&config, &report);
    Ok(())
}

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("実行エラー: {err:#}");
            ExitCode::FAILURE
        }
    }
}
