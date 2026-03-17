use crate::ecc::EccError;
use crate::tmr::TmrError;
use bitflip::BitFlipError;

/// エラーが復旧可能かどうかを表します。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Recoverable,
    Fatal,
}

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

    #[error("TMR error: {0}")]
    Tmr(#[from] TmrError),

    #[error("Serialization/deserialization error: {0}")]
    Bincode(#[from] Box<bincode::ErrorKind>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// エラーの復旧可能性を返します。
    pub fn class(&self) -> ErrorClass {
        match self {
            Error::InvalidProtectionLevel | Error::Bincode(_) => ErrorClass::Fatal,
            Error::FrameTooShort
            | Error::UnrecoverableHeader
            | Error::CrcMismatch
            | Error::InvalidFrameFormat
            | Error::Ecc(_)
            | Error::BitFlip(_)
            | Error::Tmr(_) => ErrorClass::Recoverable,
            Error::Io(error) => match error.kind() {
                std::io::ErrorKind::NotFound
                | std::io::ErrorKind::InvalidData
                | std::io::ErrorKind::InvalidInput
                | std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::WouldBlock
                | std::io::ErrorKind::Interrupted
                | std::io::ErrorKind::TimedOut => ErrorClass::Recoverable,
                _ => ErrorClass::Fatal,
            },
        }
    }

    pub fn is_recoverable(&self) -> bool {
        self.class() == ErrorClass::Recoverable
    }

    pub fn is_fatal(&self) -> bool {
        self.class() == ErrorClass::Fatal
    }
}
