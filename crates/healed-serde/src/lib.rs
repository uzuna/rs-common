pub mod ecc;
pub mod error;
pub mod frame;
pub mod metadata;

use serde::{Deserialize, Serialize};

/// データ保護レベル。
///
/// オーバーヘッドと保護性能のトレードオフを調整します。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProtectionLevel {
    /// 8bitデータ + ECC (高密度保護)
    High = 0,
    /// 32bitデータ + ECC (バランス)
    Medium = 1,
    /// 64bitデータ + ECC (低オーバーヘッド)
    Low = 2,
}

impl TryFrom<u8> for ProtectionLevel {
    type Error = crate::error::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ProtectionLevel::High),
            1 => Ok(ProtectionLevel::Medium),
            2 => Ok(ProtectionLevel::Low),
            _ => Err(crate::error::Error::InvalidProtectionLevel),
        }
    }
}
