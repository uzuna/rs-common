//! MoonBitプラグインの実行ホスト

use clap::Parser;
use moonbit_runner::runner;
use std::process::ExitCode;

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

    /// plain core Wasm を使う線形メモリ benchmark 用ファイルのパス
    #[arg(long)]
    raw_wasm: Option<std::path::PathBuf>,

    /// `/status` エンドポイントを起動する
    #[arg(long, default_value_t = false)]
    serve_status: bool,

    /// `/status` エンドポイントの待ち受けアドレス
    #[arg(long, default_value = "127.0.0.1:8080")]
    status_addr: std::net::SocketAddr,
}

fn try_main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    let config = runner::RunnerConfig {
        wasm: opt.wasm,
        wasi: opt.wasi,
        benchmark_iterations: opt.iterations,
        raw_wasm: opt.raw_wasm,
    };
    let report = runner::run(&config)?;
    runner::print_report(&config, &report);

    if opt.serve_status {
        runner::serve_status_endpoint(&config, &report, opt.status_addr)?;
    }

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
