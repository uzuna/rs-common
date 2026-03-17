use crate::ecc::EccError;
use bitflip::BitFlipError;

/// ライブラリ全体で使用されるエラー型。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid protection level value")]
    InvalidProtectionLevel,

    #[error("Frame data is too short to be valid")]
    FrameTooShort,

    #[error("Failed to recover header from both primary and secondary copies")]
    UnrecoverableHeader,

    #[error("Frame CRC check failed")]
    CrcMismatch,

    #[error("Invalid frame payload format")]
    InvalidFrameFormat,

    #[error("ECC error: {0}")]
    Ecc(#[from] EccError),

    #[error("Bit flip error: {0}")]
    BitFlip(#[from] BitFlipError),

    #[error("Serialization/deserialization error: {0}")]
    Bincode(#[from] Box<bincode::ErrorKind>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
