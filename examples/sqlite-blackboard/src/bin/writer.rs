use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;

use sqlite_blackboard::writer::WriterConfig;

#[derive(Parser)]
#[command(about = "SQLite ブラックボード PoC — Writer")]
struct Args {
    /// DBファイルパス
    #[arg(long, default_value = "/dev/shm/sqlite_poc/blackboard.db")]
    db: PathBuf,

    /// 生成するチャネル数
    #[arg(long, default_value = "1")]
    channels: usize,

    /// 1 メッセージのデータサイズ (bytes)
    #[arg(long, default_value = "4096")]
    data_size: usize,

    /// 書き込み周波数 (Hz)
    #[arg(long, default_value = "1000")]
    hz: u32,

    /// コミット間隔 (ms)
    #[arg(long, default_value = "100")]
    commit_ms: u64,

    /// 実行時間（秒）。省略時は無限
    #[arg(long)]
    duration: Option<u64>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = Args::parse();
    let config = WriterConfig {
        channel_count: args.channels,
        data_size: args.data_size,
        hz: args.hz,
        commit_ms: args.commit_ms,
    };
    let duration = args.duration.map(Duration::from_secs);
    sqlite_blackboard::writer::run(&args.db, &config, duration)
}
