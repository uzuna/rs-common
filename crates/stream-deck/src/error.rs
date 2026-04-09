//! エラー定義

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StreamDeckError {
    #[error("デバイスが見つかりません: {0}")]
    DeviceNotFound(String),

    #[error("未対応デバイス: {0}")]
    UnsupportedDevice(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
