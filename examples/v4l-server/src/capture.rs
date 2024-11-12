//! captureはサーバーに対して1つの実行フロー歯科持つことができない

use tokio::{
    select,
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;
use v4l::{prelude::UserptrStream, video::Capture};

use crate::{error::AppError, util::open_device};

pub enum Request {
    Capture {
        tx: oneshot::Sender<Result<CaptureResponse, anyhow::Error>>,
        device_index: usize,
        format: v4l::Format,
        buffer_count: u32,
    },
}

#[derive(Debug, serde::Deserialize)]
pub struct CaptureQuery {
    pub fourcc: String,
    pub width: u32,
    pub height: u32,
    /// カメラの安定を待つバッファ数
    #[serde(default = "CaptureQuery::buffer_count_default")]
    pub buffer_count: u32,
}

impl CaptureQuery {
    /// パラメータが有効な範囲内かどうかを検証する
    pub fn validate(&self) -> Result<(), AppError> {
        if self.fourcc.len() != 4 {
            return Err(anyhow::anyhow!("FourCC must be 4 characters. {}", self.fourcc).into());
        }
        if self.width == 0 || self.height == 0 {
            return Err(
                anyhow::anyhow!("Invalid width or height {}x{}", self.width, self.height).into(),
            );
        }
        Ok(())
    }

    pub fn format(&self) -> v4l::Format {
        let mut fourcc = [0; 4];
        self.fourcc
            .as_bytes()
            .iter()
            .take(4)
            .enumerate()
            .for_each(|(i, &b)| {
                fourcc[i] = b;
            });
        v4l::Format::new(self.width, self.height, v4l::FourCC::new(&fourcc))
    }

    // 一般的には4バッファぐらい待つと自動露光などの結果が安定する
    fn buffer_count_default() -> u32 {
        4
    }
}

/// 最終的なcapture実行時のformat
#[derive(Debug, serde::Serialize)]
pub struct CaptureFormat {
    pub fourcc: String,
    pub width: u32,
    pub height: u32,
}

pub struct CaptureResponse {
    pub format: CaptureFormat,
    pub buffer: Vec<u8>,
}

/// サーバーに対して1つだけのcaptureルーチンを持つ実装
///
/// TODO: 実際には1デバイスあたり1つのルーチンまで実行が許されるので、良き感じに構造化するのが望ましい
pub struct CaptureRoutine {
    rx: mpsc::Receiver<Request>,
}

impl CaptureRoutine {
    pub fn new() -> (Self, mpsc::Sender<Request>) {
        let (tx, rx) = mpsc::channel(10);
        (CaptureRoutine { rx }, tx)
    }

    pub async fn start(&mut self, token: CancellationToken) -> anyhow::Result<()> {
        loop {
            select! {
                _ = token.cancelled() => {
                    break;
                }
                Some(req) = self.rx.recv() => {
                    match req {
                        Request::Capture {
                            tx,
                            format,
                            device_index,
                            buffer_count,
                        } => {
                            let res = match capture_inner(format, device_index, buffer_count).await{
                                Ok(res) => res,
                                Err(e) => {
                                    tracing::error!("Failed to capture: {:?}", e);
                                    continue;
                                }
                            };
                            match tx.send(Ok(res)) {
                                Ok(_) => {}
                                Err(_e) => {
                                    tracing::error!("Failed to sendback to connection");
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// captureの内部実装
async fn capture_inner(
    format: v4l::format::Format,
    device_index: usize,
    buffer_count: u32,
) -> anyhow::Result<CaptureResponse> {
    use v4l::io::traits::{AsyncCaptureStream, Stream};
    let dev = open_device(device_index)?;
    dev.set_format(&format).inspect_err(|e| {
        tracing::error!("Failed to set format: {:?}", e);
    })?;
    let actual_format = dev.format().inspect_err(|e| {
        tracing::error!("Failed to get format: {:?}", e);
    })?;
    let mut stream =
        UserptrStream::with_buffers(&dev, v4l::buffer::Type::VideoCapture, buffer_count)?;
    stream.poll_next().await?;
    if buffer_count > 2 {
        for _ in 0..buffer_count - 1 {
            let (_buf, _meta) = stream.poll_next().await?;
        }
    }
    let (buf, _meta) = stream.poll_next().await?;
    let b = buf.to_owned();
    stream.stop()?;
    Ok(CaptureResponse {
        format: CaptureFormat {
            fourcc: actual_format.fourcc.to_string(),
            width: actual_format.width,
            height: actual_format.height,
        },
        buffer: b,
    })
}
