use secded::{secded7264, SecDed128, SecDed64, SecDedCodec};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// データ保護レベル。オーバーヘッドと保護性能のトレードオフを調整します。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtectionLevel {
    /// 8bitデータごとにECCを付与します (高密度保護)。
    High,
    /// 32bitデータごとにECCを付与します (バランス)。
    Medium,
    /// 64bitデータごとにECCを付与します (低オーバーヘッド)。
    Low,
}

/// ECCでエンコードされたデータブロック。
/// ProtectionLevelに応じて内部のデータサイズが変わります。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncodedBlock {
    High(u16),
    Medium(u64),
    Low(u128),
}

/// ECC処理におけるエラー。
#[derive(Error, Debug, PartialEq, Eq)]
pub enum EccError {
    /// デコード対象のブロック間でProtectionLevelが一致しません。
    #[error("Inconsistent protection level in encoded blocks")]
    InconsistentLevel,
    /// 修正不可能なデータ破損が検出されました (2ビット以上のエラー)。
    #[error("Uncorrectable data corruption detected")]
    Uncorrectable,
    /// 指定されたProtectionLevelに対して、入力データの長さが不正です。
    #[error("Invalid input data length for the protection level")]
    InvalidLength,
}

/// 生データを指定された保護レベルでエンコードし、`EncodedBlock`のベクタを返します。
///
/// # Arguments
///
/// * `data` - エンコードするバイトスライス。
/// * `level` - 使用する保護レベル。
///
/// # Errors
///
/// `EccError::InvalidLength` - `data`の長さが保護レベルの要求するブロックサイズの倍数でない場合。
pub fn encode(data: &[u8], level: ProtectionLevel) -> Result<Vec<EncodedBlock>, EccError> {
    match level {
        ProtectionLevel::High => {
            // 8bit (1 byte) chunks
            Ok(data
                .iter()
                .map(|&byte| {
                    let mut raw = [0u8; 8];
                    raw[0] = byte;
                    let parity = secded7264::encode(raw);
                    EncodedBlock::High(u16::from_le_bytes([byte, parity]))
                })
                .collect())
        }
        ProtectionLevel::Medium => {
            // 32bit (4 bytes) chunks
            if data.len() % 4 != 0 {
                return Err(EccError::InvalidLength);
            }
            let secded = SecDed64::new(57);
            Ok(data
                .chunks_exact(4)
                .map(|chunk| {
                    let mut raw = [0u8; 4];
                    raw.copy_from_slice(chunk);
                    let mut encoded = ((u32::from_be_bytes(raw) as u64) << 32).to_be_bytes();
                    secded.encode(&mut encoded);
                    EncodedBlock::Medium(u64::from_be_bytes(encoded))
                })
                .collect())
        }
        ProtectionLevel::Low => {
            // 64bit (8 bytes) chunks
            if data.len() % 8 != 0 {
                return Err(EccError::InvalidLength);
            }
            let secded = SecDed128::new(120);
            Ok(data
                .chunks_exact(8)
                .map(|chunk| {
                    let mut raw = [0u8; 8];
                    raw.copy_from_slice(chunk);
                    let mut encoded = ((u64::from_be_bytes(raw) as u128) << 64).to_be_bytes();
                    secded.encode(&mut encoded);
                    EncodedBlock::Low(u128::from_be_bytes(encoded))
                })
                .collect())
        }
    }
}

/// `EncodedBlock`のベクタをデコードし、元のデータを復元します。
/// 1ビットのエラーは自動的に修復されます。
///
/// # Arguments
///
/// * `blocks` - デコードする`EncodedBlock`のスライス。
///
/// # Errors
///
/// * `EccError::InconsistentLevel` - スライス内に異なる保護レベルのブロックが混在している場合。
/// * `EccError::Uncorrectable` - 2ビット以上の修復不可能なエラーが検出された場合。
pub fn decode(blocks: &[EncodedBlock]) -> Result<Vec<u8>, EccError> {
    if blocks.is_empty() {
        return Ok(Vec::new());
    }

    let mut decoded_data = Vec::new();

    // 最初のブロックからレベルを判断し、全ブロックで一貫しているか検証
    let level = match blocks[0] {
        EncodedBlock::High(_) => ProtectionLevel::High,
        EncodedBlock::Medium(_) => ProtectionLevel::Medium,
        EncodedBlock::Low(_) => ProtectionLevel::Low,
    };

    match level {
        ProtectionLevel::High => {
            for block in blocks {
                let EncodedBlock::High(encoded) = *block else {
                    return Err(EccError::InconsistentLevel);
                };

                let [data, parity] = encoded.to_le_bytes();
                let mut decoded = [0u8; 8];
                decoded[0] = data;
                if secded7264::decode(&mut decoded, parity).is_err() {
                    return Err(EccError::Uncorrectable);
                }
                decoded_data.push(decoded[0]);
            }
        }
        ProtectionLevel::Medium => {
            let secded = SecDed64::new(57);
            for block in blocks {
                let EncodedBlock::Medium(encoded) = *block else {
                    return Err(EccError::InconsistentLevel);
                };

                let mut decoded = encoded.to_be_bytes();
                if secded.decode(&mut decoded).is_err() {
                    return Err(EccError::Uncorrectable);
                }
                decoded_data
                    .extend_from_slice(&((u64::from_be_bytes(decoded) >> 32) as u32).to_be_bytes());
            }
        }
        ProtectionLevel::Low => {
            let secded = SecDed128::new(120);
            for block in blocks {
                let EncodedBlock::Low(encoded) = *block else {
                    return Err(EccError::InconsistentLevel);
                };

                let mut decoded = encoded.to_be_bytes();
                if secded.decode(&mut decoded).is_err() {
                    return Err(EccError::Uncorrectable);
                }
                decoded_data.extend_from_slice(
                    &((u128::from_be_bytes(decoded) >> 64) as u64).to_be_bytes(),
                );
            }
        }
    }

    Ok(decoded_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_high() {
        let data = b"hello world";
        let level = ProtectionLevel::High;
        let encoded = encode(data, level).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(data, decoded.as_slice());
    }

    #[test]
    fn test_roundtrip_medium() {
        let data = b"hello world!"; // 12 bytes, multiple of 4
        let level = ProtectionLevel::Medium;
        let encoded = encode(data, level).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(data, decoded.as_slice());
    }

    #[test]
    fn test_roundtrip_low() {
        let data = b"16 bytes data..."; // 16 bytes, multiple of 8
        let level = ProtectionLevel::Low;
        let encoded = encode(data, level).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(data, decoded.as_slice());
    }

    #[test]
    fn test_1bit_flip_correction_high() {
        let data = b"A";
        let level = ProtectionLevel::High;
        let mut encoded = encode(data, level).unwrap();

        // Flip 1 bit in the encoded data
        if let EncodedBlock::High(ref mut val) = &mut encoded[0] {
            *val ^= 1 << 5; // Flip 5th bit
        }

        let decoded = decode(&encoded).unwrap();
        assert_eq!(data, decoded.as_slice(), "Data should be corrected");
    }

    #[test]
    fn test_2bit_flip_uncorrectable_high() {
        let data = b"B";
        let level = ProtectionLevel::High;
        let mut encoded = encode(data, level).unwrap();

        if let EncodedBlock::High(ref mut val) = &mut encoded[0] {
            *val ^= 1 << 3; // Flip bit 3
            *val ^= 1 << 6; // Flip bit 6
        }

        let result = decode(&encoded);
        assert!(matches!(result, Err(EccError::Uncorrectable)));
    }

    #[test]
    fn test_invalid_length_error() {
        let data = b"12345"; // 5 bytes
        let result_medium = encode(data, ProtectionLevel::Medium);
        assert_eq!(result_medium, Err(EccError::InvalidLength));

        let result_low = encode(data, ProtectionLevel::Low);
        assert_eq!(result_low, Err(EccError::InvalidLength));
    }
}
