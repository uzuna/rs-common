use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;

use sqlite_blackboard::reader::ChannelSpec;

#[derive(Parser)]
#[command(about = "SQLite ブラックボード PoC — Reader (30Hz 読み出し)")]
#[group(required = false)]
struct Args {
    /// DBファイルパス
    #[arg(long, default_value = "/dev/shm/sqlite_poc/blackboard.db")]
    db: PathBuf,

    /// 監視対象のチャネル ID（--topic と排他）
    #[arg(long, default_value = "1", conflicts_with = "topic")]
    channel_id: i64,

    /// 監視対象のトピック名（--channel-id と排他、Writer 再起動後も自動追従）
    #[arg(long, conflicts_with = "channel_id")]
    topic: Option<String>,

    /// ログ出力用のリーダー識別子
    #[arg(long, default_value = "r0")]
    reader_id: String,

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

    let channel_spec = if let Some(topic) = args.topic {
        ChannelSpec::Topic(topic)
    } else {
        ChannelSpec::Id(args.channel_id)
    };

    sqlite_blackboard::reader::run(&args.db, channel_spec, &args.reader_id, duration)
}
