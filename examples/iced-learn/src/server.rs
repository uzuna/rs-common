use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub struct ThreadHandles {
    pub process: CancellationToken,
    pub handle: std::thread::JoinHandle<anyhow::Result<()>>,
}

/// UI側が持つチャネル
///
/// UIが主でサーバーが従
pub struct UiChannel<Req, Res> {
    pub rx: mpsc::Receiver<Res>,
    pub tx: mpsc::Sender<Req>,
}

/// サーバー側が持つチャネル
pub struct ServerChannel<Req, Res> {
    pub rx: mpsc::Receiver<Req>,
    pub tx: mpsc::Sender<Res>,
}

pub enum Request {}
pub enum Response {
    Tick(Duration),
}

pub type UiCh = UiChannel<Request, Response>;

pub struct Server {
    process: CancellationToken,
    ch: ServerChannel<Request, Response>,
}

impl Server {
    const DEFAULT_CHANNEL_SIZE: usize = 16;
    pub fn new(process: CancellationToken) -> (Self, UiChannel<Request, Response>) {
        let (tx, rx) = mpsc::channel::<Request>(Self::DEFAULT_CHANNEL_SIZE);
        let (tx2, rx2) = mpsc::channel::<Response>(Self::DEFAULT_CHANNEL_SIZE);

        let ch = ServerChannel { rx, tx: tx2 };
        let ui_ch = UiChannel { rx: rx2, tx };

        (Self { ch, process }, ui_ch)
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        info!("Server is running");
        let start = tokio::time::Instant::now();
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(50));
        let mut stalled = false;
        loop {
            tokio::select! {
                _ = self.process.cancelled() => {
                    info!("Server is cancelled");
                    break;
                }
                x = ticker.tick() => {
                    // ここでサーバーの処理を行う
                    let elapsed = x - start;
                    // 送信できない場合でもエラーにしない
                    match (self.ch.tx.try_send(Response::Tick(elapsed)), stalled) {
                        (Ok(_), false) => {}
                        (Ok(_), true) => {
                            stalled = false;
                        }
                        (Err(mpsc::error::TrySendError::Full(_)), false) => {
                            // 送信できない場合は、スルーする
                            warn!("UI is busy at {}s", elapsed.as_secs());
                            stalled = true;
                        },
                        (Err(mpsc::error::TrySendError::Full(_)), true) => {}
                        (Err(mpsc::error::TrySendError::Closed(_)), _) => {
                            // 送信先が閉じている場合は、終了する
                            info!("Server is closed");
                            break;
                        }
                    }
                }
            }
        }
        info!("Server is closing");
        Ok(())
    }
}

/// 時間のかかる処理やサーバーとのやり取りを行うためのスレッドを生成
pub fn spawn() -> (ThreadHandles, UiChannel<Request, Response>) {
    let token = CancellationToken::new();
    let (s, u) = Server::new(token.clone());
    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(s.run())
    });
    let th = ThreadHandles {
        process: token,
        handle,
    };
    (th, u)
}
