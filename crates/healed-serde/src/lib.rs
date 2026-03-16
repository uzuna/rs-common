pub mod ecc;
pub mod error;
pub mod frame;
pub mod metadata;
pub mod vault;

use error::Error;
use frame::StorageFrame;
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

/// 生バイナリをECC保護付きフレームにエンコードします。
///
/// 既に別シリアライザ（`serde_json::to_vec` など）で得たバイト列を
/// 追加でECC保護したい場合に使用します。
///
/// # Arguments
///
/// * `payload` - 保護対象のバイト列。
/// * `level` - ECCの保護レベル。
pub fn encode(payload: &[u8], level: ProtectionLevel) -> Result<Vec<u8>, Error> {
    if payload.len() > u32::MAX as usize {
        // ペイロードがフレーム形式で表現可能な最大サイズ（u32::MAX）を超えています。
        // これ以上大きい場合は誤ったフレームになるためエラーとします。
        return Err(Error::InvalidProtectionLevel);
    }

    let frame = StorageFrame::new(payload.to_vec(), 0, level);
    frame.to_bytes()
}

/// ECC保護付きフレームをデコードし、生バイナリを復元します。
///
/// [`encode`] で得たバイト列に1ビットのビット反転などの破損があっても
/// 自動修復して元のバイト列を返します。
///
/// # Arguments
///
/// * `bytes` - [`encode`] で生成したバイト列。
pub fn decode(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let frame = StorageFrame::recover(bytes)?;
    Ok(frame.payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Sample {
        id: u32,
        value: String,
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let payload = b"hello, binary payload".to_vec();

        for level in [
            ProtectionLevel::High,
            ProtectionLevel::Medium,
            ProtectionLevel::Low,
        ] {
            let encoded = encode(&payload, level).unwrap();
            let decoded = decode(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    #[test]
    fn test_decode_recovers_bit_flip() {
        let payload: Vec<u8> = (0..128).collect();

        let mut bytes = encode(&payload, ProtectionLevel::High).unwrap();
        // ペイロード領域（ヘッダー32バイト以降）の1ビットを反転
        bytes[40] ^= 0b0000_0001;

        let decoded = decode(&bytes).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn test_external_serializer_workflow() {
        let original = Sample {
            id: 99,
            value: "ecc test".to_string(),
        };

        // 外部シリアライザでバイナリ化
        let serialized = serde_json::to_vec(&original).unwrap();

        // バイナリをECC保護
        let protected = encode(&serialized, ProtectionLevel::Medium).unwrap();

        // ECC復元後に外部シリアライザでデシリアライズ
        let recovered_binary = decode(&protected).unwrap();
        let recovered: Sample = serde_json::from_slice(&recovered_binary).unwrap();

        assert_eq!(original, recovered);
    }
}
