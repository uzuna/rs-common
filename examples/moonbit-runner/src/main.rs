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

    /// HTTP サーバーの待ち受けアドレス
    #[arg(long, default_value = "127.0.0.1:8080")]
    addr: std::net::SocketAddr,

    /// Wasm プラグインへ処理を委譲する URL プレフィックス
    #[arg(long, default_value = "/api")]
    prefix: String,

    /// バージョン管理用ストレージディレクトリ
    #[arg(long, default_value = "plugin-versions")]
    plugin_dir: std::path::PathBuf,

    /// 保持するバージョン数の上限
    #[arg(long, default_value_t = 10)]
    max_versions: usize,
}

fn try_main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    let config = runner::RunnerConfig {
        wasm: opt.wasm,
        wasi: opt.wasi,
        bind_addr: opt.addr,
        plugin_prefix: opt.prefix,
        plugin_dir: opt.plugin_dir,
        max_versions: opt.max_versions,
    };
    runner::serve_http(config)
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
