use crate::ecc::{self, EncodedBlock};
use crate::error::Error;
use crate::metadata::{MetaData, MetaDataHeader};
use crate::ProtectionLevel;
use crc::{Crc, CRC_32_ISCSI};

const PRIMARY_HEADER_OFFSET: usize = 0;
const PRIMARY_HEADER_SIZE: usize = 16;
const SECONDARY_HEADER_OFFSET: usize = PRIMARY_HEADER_OFFSET + PRIMARY_HEADER_SIZE;
const SECONDARY_HEADER_SIZE: usize = 16;
const HEADERS_SIZE: usize = PRIMARY_HEADER_SIZE + SECONDARY_HEADER_SIZE;
const FOOTER_SIZE: usize = 8;
const MIN_FRAME_SIZE: usize = HEADERS_SIZE + FOOTER_SIZE;

pub const CRC_ISCSI: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// 8バイトのフッター。CRC32とシーケンス番号のチェックサムを含む。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Footer {
    crc32: u32,
    sequence_check: u32,
}

impl Footer {
    fn new(crc32: u32, sequence: u64) -> Self {
        Self {
            crc32,
            // u64のシーケンス番号をu32にキャストして格納。
            // 完全な一致性検証ではないが、破損検出の一助となる。
            sequence_check: sequence as u32,
        }
    }

    fn to_bytes(self) -> [u8; FOOTER_SIZE] {
        let mut bytes = [0u8; FOOTER_SIZE];
        bytes[0..4].copy_from_slice(&self.crc32.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.sequence_check.to_le_bytes());
        bytes
    }

    fn from_bytes(bytes: &[u8; FOOTER_SIZE]) -> Self {
        let crc32 = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let sequence_check = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        Self {
            crc32,
            sequence_check,
        }
    }
}

/// 永続化されるデータの完全なフレーム。
/// ヘッダー、ペイロード、フッターで構成される。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageFrame {
    pub meta: MetaData,
    /// 元の（デコードされた）ペイロードデータ。
    pub payload: Vec<u8>,
}

impl StorageFrame {
    fn checksum_content(meta: &MetaData, payload: &[u8]) -> u32 {
        let mut checksum_data = Vec::with_capacity(13 + payload.len());
        checksum_data.extend_from_slice(&meta.sequence.to_le_bytes());
        checksum_data.push(meta.level as u8);
        checksum_data.extend_from_slice(&meta.payload_len.to_le_bytes());
        checksum_data.extend_from_slice(payload);
        CRC_ISCSI.checksum(&checksum_data)
    }

    fn payload_block_size(level: ProtectionLevel) -> usize {
        match level {
            ProtectionLevel::High => 1,
            ProtectionLevel::Medium => 4,
            ProtectionLevel::Low => 8,
        }
    }

    fn encoded_block_size(level: ProtectionLevel) -> usize {
        match level {
            ProtectionLevel::High => 2,
            ProtectionLevel::Medium => 8,
            ProtectionLevel::Low => 16,
        }
    }

    fn padded_len(payload_len: usize, level: ProtectionLevel) -> usize {
        let block_size = Self::payload_block_size(level);
        let remainder = payload_len % block_size;
        if remainder == 0 {
            payload_len
        } else {
            payload_len + (block_size - remainder)
        }
    }

    fn encode_blocks_to_bytes(
        blocks: &[EncodedBlock],
        level: ProtectionLevel,
    ) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::with_capacity(blocks.len() * Self::encoded_block_size(level));

        for block in blocks {
            match (level, block) {
                (ProtectionLevel::High, EncodedBlock::High(v)) => {
                    bytes.extend_from_slice(&v.to_le_bytes())
                }
                (ProtectionLevel::Medium, EncodedBlock::Medium(v)) => {
                    bytes.extend_from_slice(&v.to_le_bytes())
                }
                (ProtectionLevel::Low, EncodedBlock::Low(v)) => {
                    bytes.extend_from_slice(&v.to_le_bytes())
                }
                _ => return Err(Error::InvalidFrameFormat),
            }
        }

        Ok(bytes)
    }

    fn decode_blocks_from_bytes(
        bytes: &[u8],
        level: ProtectionLevel,
        payload_len: u32,
    ) -> Result<Vec<EncodedBlock>, Error> {
        let padded_len = Self::padded_len(payload_len as usize, level);
        let block_count = padded_len / Self::payload_block_size(level);
        let encoded_block_size = Self::encoded_block_size(level);
        let expected_len = block_count * encoded_block_size;

        if bytes.len() != expected_len {
            return Err(Error::InvalidFrameFormat);
        }

        let mut blocks = Vec::with_capacity(block_count);
        for chunk in bytes.chunks_exact(encoded_block_size) {
            let block = match level {
                ProtectionLevel::High => {
                    EncodedBlock::High(u16::from_le_bytes(chunk.try_into().unwrap()))
                }
                ProtectionLevel::Medium => {
                    EncodedBlock::Medium(u64::from_le_bytes(chunk.try_into().unwrap()))
                }
                ProtectionLevel::Low => {
                    EncodedBlock::Low(u128::from_le_bytes(chunk.try_into().unwrap()))
                }
            };
            blocks.push(block);
        }

        Ok(blocks)
    }

    /// 新しい`StorageFrame`を作成します。
    pub fn new(payload: Vec<u8>, sequence: u64, level: ProtectionLevel) -> Self {
        let meta = MetaData::new(sequence, level, payload.len() as u32);
        Self { meta, payload }
    }

    /// フレームをバイト列にシリアライズします。
    /// この処理にはペイロードのパディング、ECCエンコード、bincodeシリアライズ、CRC計算が含まれます。
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        // 1. ペイロードを保護レベルに応じてパディング
        let padded_payload = Self::pad_payload(&self.payload, self.meta.level);

        // 2. ECCブロックにエンコード
        let encoded_blocks = ecc::encode(&padded_payload, self.meta.level)?;

        // 3. 固定長バイナリへシリアライズ
        let encoded_payload_bytes = Self::encode_blocks_to_bytes(&encoded_blocks, self.meta.level)?;

        // 4. ヘッダーを生成
        let primary_header = self.meta.encode();
        let secondary_header = primary_header; // ミラー

        // 5. CRCを計算し、フッターを生成
        let crc = Self::checksum_content(&self.meta, &self.payload);
        let footer = Footer::new(crc, self.meta.sequence);

        // 6. 全てを結合して最終的なバイト列を返す
        let mut final_bytes =
            Vec::with_capacity(HEADERS_SIZE + encoded_payload_bytes.len() + FOOTER_SIZE);
        final_bytes.extend_from_slice(primary_header.as_bytes());
        final_bytes.extend_from_slice(secondary_header.as_bytes());
        final_bytes.extend_from_slice(&encoded_payload_bytes);
        final_bytes.extend_from_slice(&footer.to_bytes());

        Ok(final_bytes)
    }

    /// バイト列から`StorageFrame`を復元します。
    /// ヘッダーの修復、CRCチェック、ペイロードのデコードと修復を試みます。
    pub fn recover(bytes: &[u8]) -> Result<Self, Error> {
        // 1. 最低限の長さをチェック
        if bytes.len() < MIN_FRAME_SIZE {
            return Err(Error::FrameTooShort);
        }

        // 2. ヘッダーを復元
        let primary_header_bytes: &[u8; PRIMARY_HEADER_SIZE] = &bytes
            [PRIMARY_HEADER_OFFSET..SECONDARY_HEADER_OFFSET]
            .try_into()
            .unwrap();
        let secondary_header_bytes: &[u8; SECONDARY_HEADER_SIZE] = &bytes
            [SECONDARY_HEADER_OFFSET..HEADERS_SIZE]
            .try_into()
            .unwrap();

        let meta = MetaDataHeader::from_bytes(primary_header_bytes)
            .decode()
            .or_else(|| MetaDataHeader::from_bytes(secondary_header_bytes).decode())
            .ok_or(Error::UnrecoverableHeader)?;

        // 3. フッターを取得
        let footer_bytes: &[u8; FOOTER_SIZE] =
            bytes[bytes.len() - FOOTER_SIZE..].try_into().unwrap();
        let footer = Footer::from_bytes(footer_bytes);

        // 4. ペイロードをデコード
        let encoded_payload_bytes = &bytes[HEADERS_SIZE..bytes.len() - FOOTER_SIZE];
        let blocks =
            Self::decode_blocks_from_bytes(encoded_payload_bytes, meta.level, meta.payload_len)?;
        let decoded_padded_payload = ecc::decode(&blocks)?;

        // 5. パディングを削除して元のペイロードを取得
        if decoded_padded_payload.len() < meta.payload_len as usize {
            return Err(Error::CrcMismatch);
        }
        let payload = decoded_padded_payload[..meta.payload_len as usize].to_vec();

        // 6. 復元済みデータで最終整合性を検証
        let calculated_crc = Self::checksum_content(&meta, &payload);
        if footer.crc32 != calculated_crc || footer.sequence_check != meta.sequence as u32 {
            return Err(Error::CrcMismatch);
        }

        Ok(StorageFrame { meta, payload })
    }

    /// ペイロードを保護レベルのブロックサイズに合わせてパディングします。
    fn pad_payload(payload: &[u8], level: ProtectionLevel) -> Vec<u8> {
        let block_size = Self::payload_block_size(level);

        let remainder = payload.len() % block_size;
        if remainder == 0 {
            return payload.to_vec();
        }

        let padding_len = block_size - remainder;
        let mut padded = Vec::with_capacity(payload.len() + padding_len);
        padded.extend_from_slice(payload);
        padded.resize(payload.len() + padding_len, 0); // 0でパディング
        padded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// StorageFrameのシリアライズ/復元が無損失で往復できることを検証。
    #[test]
    fn test_frame_roundtrip() {
        let payload = b"hello, robust world!".to_vec();
        let frame = StorageFrame::new(payload, 123, ProtectionLevel::Medium);
        let bytes = frame.to_bytes().unwrap();
        let recovered_frame = StorageFrame::recover(&bytes).unwrap();

        assert_eq!(frame, recovered_frame);
    }

    /// Primaryヘッダーが壊れてもSecondaryヘッダーから復元できることを検証。
    #[test]
    fn test_recover_with_primary_header_corruption() {
        let payload = b"some important data".to_vec();
        let frame = StorageFrame::new(payload, 456, ProtectionLevel::Low);
        let mut bytes = frame.to_bytes().unwrap();

        // プライマリヘッダーを破壊 (2ビット反転)
        bytes[5] ^= 0b11;

        let recovered_frame = StorageFrame::recover(&bytes).unwrap();
        assert_eq!(frame, recovered_frame);
    }

    /// ペイロード領域の1ビット破損がECC復元後に元データへ戻ることを検証。
    #[test]
    fn test_recover_with_payload_corruption() {
        let payload: Vec<u8> = (0..128).collect();
        let frame = StorageFrame::new(payload, 789, ProtectionLevel::High);
        let mut bytes = frame.to_bytes().unwrap();

        // ペイロード部分のどこか1ビットを反転
        // ヘッダー(32B)とフッター(8B)以外
        let payload_offset = HEADERS_SIZE + 10;
        bytes[payload_offset] ^= 0b0001_0000;

        let recovered_frame = StorageFrame::recover(&bytes).unwrap();
        assert_eq!(frame, recovered_frame);
    }

    /// フッターCRC改ざん時に CrcMismatch が返ることを検証。
    #[test]
    fn test_crc_mismatch_detection() {
        let payload = b"data that must be integral".to_vec();
        let frame = StorageFrame::new(payload, 1, ProtectionLevel::Medium);
        let mut bytes = frame.to_bytes().unwrap();

        // フッター内のCRC値を改ざんして不一致を確実に発生させる
        let crc_offset = bytes.len() - FOOTER_SIZE;
        let current_crc = u32::from_le_bytes(bytes[crc_offset..crc_offset + 4].try_into().unwrap());
        let tampered_crc = current_crc.wrapping_add(1);
        bytes[crc_offset..crc_offset + 4].copy_from_slice(&tampered_crc.to_le_bytes());

        let result = StorageFrame::recover(&bytes);
        assert!(matches!(result, Err(Error::CrcMismatch)));
    }

    /// Primary/Secondary両ヘッダー破損時に UnrecoverableHeader となることを検証。
    #[test]
    fn test_unrecoverable_header_error() {
        let payload = b"test".to_vec();
        let frame = StorageFrame::new(payload, 2, ProtectionLevel::High);
        let mut bytes = frame.to_bytes().unwrap();

        // 両方のヘッダーを破壊
        bytes[0] ^= 0xff;
        bytes[1] ^= 0xff;
        bytes[16] ^= 0xff;
        bytes[17] ^= 0xff;

        let result = StorageFrame::recover(&bytes);
        assert!(matches!(result, Err(Error::UnrecoverableHeader)));
    }

    /// 保護レベルごとのパディングサイズ規則が正しいことを検証。
    #[test]
    fn test_padding_logic() {
        // Medium (4-byte alignment)
        let payload1 = vec![1, 2, 3];
        let padded1 = StorageFrame::pad_payload(&payload1, ProtectionLevel::Medium);
        assert_eq!(padded1.len(), 4);
        assert_eq!(&padded1, &[1, 2, 3, 0]);

        // Low (8-byte alignment)
        let payload2 = vec![1, 2, 3, 4, 5];
        let padded2 = StorageFrame::pad_payload(&payload2, ProtectionLevel::Low);
        assert_eq!(padded2.len(), 8);
        assert_eq!(&padded2, &[1, 2, 3, 4, 5, 0, 0, 0]);

        // High (no padding needed)
        let payload3 = vec![1, 2, 3];
        let padded3 = StorageFrame::pad_payload(&payload3, ProtectionLevel::High);
        assert_eq!(padded3.len(), 3);
        assert_eq!(payload3, padded3);
    }
}
