use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(about = "SQLite ブラックボード PoC — Reader (30Hz 読み出し)")]
struct Args {
    /// DBファイルパス
    #[arg(long, default_value = "/dev/shm/sqlite_poc/blackboard.db")]
    db: PathBuf,

    /// 監視対象のチャネル ID
    #[arg(long, default_value = "1")]
    channel_id: i64,

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
    let duration = args.duration.map(Duration::from_secs);
    sqlite_blackboard::reader::run(&args.db, args.channel_id, duration)
}
