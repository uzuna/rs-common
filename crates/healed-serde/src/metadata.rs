use crate::ProtectionLevel;
use secded::{SecDed128, SecDedCodec};

const HEADER_BYTES: usize = 16;
const METADATA_PAYLOAD_BITS: usize = 120;
const METADATA_CODE_BITS: u32 = 8;
const HEADER_MAGIC: u16 = 0xA55A;

fn metadata_codec() -> SecDed128 {
    SecDed128::new(METADATA_PAYLOAD_BITS)
}

/// ヘッダーに格納されるメタデータ。
///
/// 永続化されるデータのバージョン（シーケンス番号）、保護レベル、
/// およびペイロード長を保持します。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaData {
    /// データのシーケンス番号。ローリングアップデートに使用される。
    pub sequence: u64,
    /// ECC保護レベル。
    pub level: ProtectionLevel,
    /// シリアライズされたペイロードの元の長さ。
    pub payload_len: u32,
}

/// メタデータを格納する16バイトの固定長ヘッダー。
///
/// 内部データはSECDEDによって1ビットエラー訂正が可能です。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetaDataHeader {
    /// 16バイトのバイナリデータ。
    /// secdedで保護された固定長ヘッダー。
    data: [u8; HEADER_BYTES],
}

impl MetaData {
    /// 新しいMetaDataインスタンスを作成します。
    pub fn new(sequence: u64, level: ProtectionLevel, payload_len: u32) -> Self {
        Self {
            sequence,
            level,
            payload_len,
        }
    }

    fn pack_payload(&self) -> u128 {
        let sequence = self.sequence as u128;
        let payload_len = (self.payload_len as u128) << 64;
        let level = (self.level as u8 as u128) << 96;
        let magic = (HEADER_MAGIC as u128) << 104;
        sequence | payload_len | level | magic
    }

    /// MetaDataをSECDEDで保護された16バイトのMetaDataHeaderにエンコードします。
    pub fn encode(&self) -> MetaDataHeader {
        let payload = self.pack_payload();
        let mut data = (payload << METADATA_CODE_BITS).to_be_bytes();
        let secded = metadata_codec();
        secded.encode(&mut data);
        MetaDataHeader { data }
    }
}

impl MetaDataHeader {
    /// 16バイトのスライスからMetaDataHeaderを作成します。
    pub fn from_bytes(bytes: &[u8; HEADER_BYTES]) -> Self {
        Self { data: *bytes }
    }

    /// ヘッダーデータをデコードし、1ビットエラーを修正してMetaDataを復元します。
    ///
    /// # Returns
    /// 修正不可能なエラーがある場合は `None` を返します。
    pub fn decode(&self) -> Option<MetaData> {
        let secded = metadata_codec();
        let mut decoded = self.data;
        if secded.decode(&mut decoded).is_err() {
            return None;
        }

        let payload = u128::from_be_bytes(decoded) >> METADATA_CODE_BITS;
        let sequence = payload as u64;
        let payload_len = ((payload >> 64) & 0xFFFF_FFFF) as u32;
        let level_u8 = ((payload >> 96) & 0xFF) as u8;
        let magic = ((payload >> 104) & 0xFFFF) as u16;

        if magic != HEADER_MAGIC {
            return None;
        }

        let level = ProtectionLevel::try_from(level_u8).ok()?;

        Some(MetaData {
            sequence,
            level,
            payload_len,
        })
    }

    /// ヘッダーの生バイトデータを返します。
    pub fn as_bytes(&self) -> &[u8; HEADER_BYTES] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MetaDataのエンコード/デコード往復が無損失で成立することを検証。
    #[test]
    fn test_metadata_encode_decode_roundtrip() {
        let original_meta = MetaData::new(123456789, ProtectionLevel::Medium, 1024);
        let header = original_meta.encode();
        let decoded_meta = header.decode().expect("デコードは成功するはず");

        assert_eq!(original_meta, decoded_meta);
    }

    /// ヘッダー全ビットの1ビット反転に対して訂正復元できることを検証。
    #[test]
    fn test_metadata_decode_with_1bit_error_correction() {
        let original_meta = MetaData::new(987654321, ProtectionLevel::High, 4096);
        let header = original_meta.encode();

        for bit in 0..(HEADER_BYTES * 8) {
            let mut corrupted_bytes = *header.as_bytes();
            corrupted_bytes[bit / 8] ^= 1 << (bit % 8);

            let corrupted_header = MetaDataHeader::from_bytes(&corrupted_bytes);
            let decoded_meta = corrupted_header
                .decode()
                .expect("1ビットエラーは訂正されてデコードが成功するはず");

            assert_eq!(
                original_meta, decoded_meta,
                "bit {} の1ビット反転でメタデータが復元できませんでした",
                bit
            );
        }
    }

    /// 2ビット破損の組み合わせに対して復元不能ケースを検出できることを検証。
    #[test]
    fn test_metadata_decode_with_2bit_error_fails() {
        let original_meta = MetaData::new(1, ProtectionLevel::Low, 128);
        let header = original_meta.encode();

        let mut found_uncorrectable = false;
        for bit1 in 0..(HEADER_BYTES * 8) {
            for bit2 in (bit1 + 1)..(HEADER_BYTES * 8) {
                let mut corrupted_bytes = *header.as_bytes();
                corrupted_bytes[bit1 / 8] ^= 1 << (bit1 % 8);
                corrupted_bytes[bit2 / 8] ^= 1 << (bit2 % 8);

                let corrupted_header = MetaDataHeader::from_bytes(&corrupted_bytes);
                if corrupted_header.decode().is_none() {
                    found_uncorrectable = true;
                    break;
                }
            }

            if found_uncorrectable {
                break;
            }
        }

        assert!(found_uncorrectable, "2ビットエラーを検出できるべき");
    }
}
