use crc::{Crc, CRC_32_ISCSI};
use reed_solomon_erasure::galois_8::ReedSolomon;
use thiserror::Error;

const RECORD_MAGIC: [u8; 4] = *b"RSR1";
const RECORD_VERSION: u32 = 1;
const SEGMENT_MAGIC: [u8; 4] = *b"RSG1";
const SEGMENT_VERSION: u32 = 1;
const SEGMENT_FOOTER_MAGIC: [u8; 4] = *b"RSE1";

pub const RS_DATA_SHARDS: usize = 8;
pub const RS_PARITY_SHARDS: usize = 2;
pub const RS_TOTAL_SHARDS: usize = RS_DATA_SHARDS + RS_PARITY_SHARDS;
pub const RS_SHARD_BYTES: usize = 4 * 1024;
pub const RS_DATA_BYTES_PER_SEGMENT: usize = RS_DATA_SHARDS * RS_SHARD_BYTES;

pub const RS_RECORD_HEADER_BYTES: usize = 32;
const RS_SEGMENT_HEADER_BYTES: usize = 24;
const RS_SHARD_CRC_TABLE_BYTES: usize = RS_TOTAL_SHARDS * 4;
const RS_SEGMENT_FOOTER_BYTES: usize = 8;
const RS_ENCODED_SEGMENT_BYTES: usize = RS_SEGMENT_HEADER_BYTES
    + RS_SHARD_CRC_TABLE_BYTES
    + (RS_TOTAL_SHARDS * RS_SHARD_BYTES)
    + RS_SEGMENT_FOOTER_BYTES;

const CRC_ISCSI: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsDecodedRecord {
    pub sequence: u64,
    pub payload: Vec<u8>,
    pub segment_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecordHeader {
    sequence: u64,
    payload_len: u64,
    segment_count: u32,
}

impl RecordHeader {
    fn to_bytes(self) -> [u8; RS_RECORD_HEADER_BYTES] {
        let mut bytes = [0u8; RS_RECORD_HEADER_BYTES];
        bytes[0..4].copy_from_slice(&RECORD_MAGIC);
        bytes[4..8].copy_from_slice(&RECORD_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.segment_count.to_le_bytes());
        bytes[28..32].fill(0);
        bytes
    }

    fn from_bytes(bytes: &[u8; RS_RECORD_HEADER_BYTES]) -> Result<Self, RsError> {
        if bytes[0..4] != RECORD_MAGIC {
            return Err(RsError::InvalidRecordMagic);
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != RECORD_VERSION {
            return Err(RsError::UnsupportedRecordVersion(version));
        }

        Ok(Self {
            sequence: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            payload_len: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            segment_count: u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentHeader {
    segment_index: u32,
    data_len: u32,
    segment_crc32: u32,
}

impl SegmentHeader {
    fn to_bytes(self) -> [u8; RS_SEGMENT_HEADER_BYTES] {
        let mut bytes = [0u8; RS_SEGMENT_HEADER_BYTES];
        bytes[0..4].copy_from_slice(&SEGMENT_MAGIC);
        bytes[4..8].copy_from_slice(&SEGMENT_VERSION.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.segment_index.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.data_len.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.segment_crc32.to_le_bytes());
        bytes[20..24].fill(0);
        bytes
    }

    fn from_bytes(
        bytes: &[u8; RS_SEGMENT_HEADER_BYTES],
        expected_index: u32,
    ) -> Result<Self, RsError> {
        if bytes[0..4] != SEGMENT_MAGIC {
            return Err(RsError::InvalidSegmentMagic {
                segment_index: expected_index,
            });
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != SEGMENT_VERSION {
            return Err(RsError::UnsupportedSegmentVersion {
                segment_index: expected_index,
                version,
            });
        }

        let actual_index = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if actual_index != expected_index {
            return Err(RsError::InvalidSegmentIndex {
                expected: expected_index,
                actual: actual_index,
            });
        }

        let data_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        if data_len as usize > RS_DATA_BYTES_PER_SEGMENT {
            return Err(RsError::InvalidSegmentLength {
                segment_index: expected_index,
                data_len,
            });
        }

        Ok(Self {
            segment_index: actual_index,
            data_len,
            segment_crc32: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentFooter {
    segment_crc32: u32,
}

impl SegmentFooter {
    fn to_bytes(self) -> [u8; RS_SEGMENT_FOOTER_BYTES] {
        let mut bytes = [0u8; RS_SEGMENT_FOOTER_BYTES];
        bytes[0..4].copy_from_slice(&SEGMENT_FOOTER_MAGIC);
        bytes[4..8].copy_from_slice(&self.segment_crc32.to_le_bytes());
        bytes
    }

    fn from_bytes(
        bytes: &[u8; RS_SEGMENT_FOOTER_BYTES],
        expected_crc32: u32,
        segment_index: u32,
    ) -> Result<Self, RsError> {
        if bytes[0..4] != SEGMENT_FOOTER_MAGIC {
            return Err(RsError::InvalidSegmentFooter { segment_index });
        }

        let segment_crc32 = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if segment_crc32 != expected_crc32 {
            return Err(RsError::InvalidSegmentFooter { segment_index });
        }

        Ok(Self { segment_crc32 })
    }
}

#[derive(Debug, Error)]
pub enum RsError {
    #[error("RS payload length cannot be represented")]
    PayloadTooLarge,

    #[error("RS slot is too short")]
    SlotTooShort,

    #[error("RS record header magic is invalid")]
    InvalidRecordMagic,

    #[error("RS record version {0} is not supported")]
    UnsupportedRecordVersion(u32),

    #[error("RS encoded length mismatch: expected {expected} bytes, got {actual} bytes")]
    InvalidRecordLength { expected: usize, actual: usize },

    #[error("RS decoded payload length mismatch: expected {expected} bytes, got {actual} bytes")]
    DecodedPayloadLengthMismatch { expected: u64, actual: usize },

    #[error("RS segment {segment_index} header magic is invalid")]
    InvalidSegmentMagic { segment_index: u32 },

    #[error("RS segment {segment_index} version {version} is not supported")]
    UnsupportedSegmentVersion { segment_index: u32, version: u32 },

    #[error("RS segment index mismatch: expected {expected}, got {actual}")]
    InvalidSegmentIndex { expected: u32, actual: u32 },

    #[error("RS segment {segment_index} has invalid data_len {data_len}")]
    InvalidSegmentLength { segment_index: u32, data_len: u32 },

    #[error("RS segment {segment_index} footer is invalid")]
    InvalidSegmentFooter { segment_index: u32 },

    #[error(
        "RS segment {segment_index} has too many erasures: {missing_shards} > {parity_shards}"
    )]
    TooManyShardErasures {
        segment_index: u32,
        missing_shards: usize,
        parity_shards: usize,
    },

    #[error("RS segment {segment_index} shard {shard_index} CRC mismatch after reconstruct")]
    ShardCrcMismatch {
        segment_index: u32,
        shard_index: usize,
    },

    #[error("RS segment {segment_index} payload checksum mismatch")]
    SegmentChecksumMismatch { segment_index: u32 },

    #[error("RS reconstruct error: {0}")]
    Reconstruct(#[from] reed_solomon_erasure::Error),
}

pub struct RsStrategy;

impl RsStrategy {
    pub fn encode_record(sequence: u64, payload: &[u8]) -> Result<Vec<u8>, RsError> {
        let payload_len = u64::try_from(payload.len()).map_err(|_| RsError::PayloadTooLarge)?;
        let segment_count = Self::segment_count_for_payload_len(payload.len());
        let segment_count_u32 =
            u32::try_from(segment_count).map_err(|_| RsError::PayloadTooLarge)?;

        let mut encoded =
            Vec::with_capacity(RS_RECORD_HEADER_BYTES + (segment_count * RS_ENCODED_SEGMENT_BYTES));
        let header = RecordHeader {
            sequence,
            payload_len,
            segment_count: segment_count_u32,
        };
        encoded.extend_from_slice(&header.to_bytes());

        if segment_count == 0 {
            return Ok(encoded);
        }

        let codec = ReedSolomon::new(RS_DATA_SHARDS, RS_PARITY_SHARDS)?;
        for (segment_index, chunk) in payload.chunks(RS_DATA_BYTES_PER_SEGMENT).enumerate() {
            Self::encode_segment(segment_index as u32, chunk, &codec, &mut encoded)?;
        }

        Ok(encoded)
    }

    pub fn decode_record(bytes: &[u8]) -> Result<RsDecodedRecord, RsError> {
        if bytes.len() < RS_RECORD_HEADER_BYTES {
            return Err(RsError::SlotTooShort);
        }

        let header_bytes: &[u8; RS_RECORD_HEADER_BYTES] =
            bytes[0..RS_RECORD_HEADER_BYTES].try_into().unwrap();
        let header = RecordHeader::from_bytes(header_bytes)?;

        let expected_len =
            RS_RECORD_HEADER_BYTES + (header.segment_count as usize * RS_ENCODED_SEGMENT_BYTES);
        if bytes.len() != expected_len {
            return Err(RsError::InvalidRecordLength {
                expected: expected_len,
                actual: bytes.len(),
            });
        }

        let mut payload = Vec::with_capacity(header.payload_len as usize);
        if header.segment_count == 0 {
            if header.payload_len != 0 {
                return Err(RsError::DecodedPayloadLengthMismatch {
                    expected: header.payload_len,
                    actual: 0,
                });
            }
            return Ok(RsDecodedRecord {
                sequence: header.sequence,
                payload,
                segment_count: 0,
            });
        }

        let codec = ReedSolomon::new(RS_DATA_SHARDS, RS_PARITY_SHARDS)?;
        for segment_index in 0..header.segment_count {
            let segment_offset =
                RS_RECORD_HEADER_BYTES + segment_index as usize * RS_ENCODED_SEGMENT_BYTES;
            let segment_bytes = &bytes[segment_offset..segment_offset + RS_ENCODED_SEGMENT_BYTES];
            let chunk = Self::decode_segment(segment_index, segment_bytes, &codec)?;
            payload.extend_from_slice(&chunk);
        }

        if payload.len() != header.payload_len as usize {
            return Err(RsError::DecodedPayloadLengthMismatch {
                expected: header.payload_len,
                actual: payload.len(),
            });
        }

        Ok(RsDecodedRecord {
            sequence: header.sequence,
            payload,
            segment_count: header.segment_count,
        })
    }

    pub fn peek_sequence(header_bytes: &[u8]) -> Option<u64> {
        if header_bytes.len() < RS_RECORD_HEADER_BYTES {
            return None;
        }

        let record_bytes: &[u8; RS_RECORD_HEADER_BYTES] =
            header_bytes[0..RS_RECORD_HEADER_BYTES].try_into().ok()?;
        RecordHeader::from_bytes(record_bytes)
            .ok()
            .map(|header| header.sequence)
    }

    pub const fn segment_count_for_payload_len(payload_len: usize) -> usize {
        if payload_len == 0 {
            0
        } else {
            payload_len.div_ceil(RS_DATA_BYTES_PER_SEGMENT)
        }
    }

    fn encode_segment(
        segment_index: u32,
        chunk: &[u8],
        codec: &ReedSolomon,
        encoded: &mut Vec<u8>,
    ) -> Result<(), RsError> {
        debug_assert!(chunk.len() <= RS_DATA_BYTES_PER_SEGMENT);

        let mut data_block = vec![0u8; RS_DATA_BYTES_PER_SEGMENT];
        data_block[0..chunk.len()].copy_from_slice(chunk);

        let mut shards = vec![vec![0u8; RS_SHARD_BYTES]; RS_TOTAL_SHARDS];
        for shard_index in 0..RS_DATA_SHARDS {
            let start = shard_index * RS_SHARD_BYTES;
            let end = start + RS_SHARD_BYTES;
            shards[shard_index].copy_from_slice(&data_block[start..end]);
        }

        codec.encode(&mut shards)?;

        let segment_crc32 = CRC_ISCSI.checksum(chunk);
        let header = SegmentHeader {
            segment_index,
            data_len: chunk.len() as u32,
            segment_crc32,
        };
        encoded.extend_from_slice(&header.to_bytes());

        for shard in &shards {
            encoded.extend_from_slice(&CRC_ISCSI.checksum(shard).to_le_bytes());
        }
        for shard in &shards {
            encoded.extend_from_slice(shard);
        }

        let footer = SegmentFooter { segment_crc32 };
        encoded.extend_from_slice(&footer.to_bytes());

        Ok(())
    }

    fn decode_segment(
        expected_segment_index: u32,
        segment_bytes: &[u8],
        codec: &ReedSolomon,
    ) -> Result<Vec<u8>, RsError> {
        debug_assert_eq!(segment_bytes.len(), RS_ENCODED_SEGMENT_BYTES);

        let header_bytes: &[u8; RS_SEGMENT_HEADER_BYTES] = segment_bytes
            [0..RS_SEGMENT_HEADER_BYTES]
            .try_into()
            .unwrap();
        let header = SegmentHeader::from_bytes(header_bytes, expected_segment_index)?;

        let crc_table_start = RS_SEGMENT_HEADER_BYTES;
        let crc_table_end = crc_table_start + RS_SHARD_CRC_TABLE_BYTES;
        let shard_data_start = crc_table_end;
        let shard_data_end = shard_data_start + (RS_TOTAL_SHARDS * RS_SHARD_BYTES);
        let footer_bytes: &[u8; RS_SEGMENT_FOOTER_BYTES] = segment_bytes
            [shard_data_end..shard_data_end + RS_SEGMENT_FOOTER_BYTES]
            .try_into()
            .unwrap();
        SegmentFooter::from_bytes(footer_bytes, header.segment_crc32, expected_segment_index)?;

        let mut shard_crcs = [0u32; RS_TOTAL_SHARDS];
        for (shard_index, crc) in shard_crcs.iter_mut().enumerate() {
            let crc_start = crc_table_start + shard_index * 4;
            *crc = u32::from_le_bytes(segment_bytes[crc_start..crc_start + 4].try_into().unwrap());
        }

        let mut shards: Vec<Option<Vec<u8>>> = Vec::with_capacity(RS_TOTAL_SHARDS);
        let mut missing_shards = 0usize;
        for (shard_index, expected_crc) in shard_crcs.iter().enumerate() {
            let shard_start = shard_data_start + shard_index * RS_SHARD_BYTES;
            let shard_end = shard_start + RS_SHARD_BYTES;
            let shard = segment_bytes[shard_start..shard_end].to_vec();

            if CRC_ISCSI.checksum(&shard) == *expected_crc {
                shards.push(Some(shard));
            } else {
                shards.push(None);
                missing_shards += 1;
            }
        }

        Self::reconstruct_missing_shards(
            codec,
            &mut shards,
            expected_segment_index,
            missing_shards,
        )?;

        let mut recovered_shards = Vec::with_capacity(RS_TOTAL_SHARDS);
        for (shard_index, shard) in shards.into_iter().enumerate() {
            let shard = shard.ok_or(RsError::TooManyShardErasures {
                segment_index: expected_segment_index,
                missing_shards: RS_PARITY_SHARDS + 1,
                parity_shards: RS_PARITY_SHARDS,
            })?;

            if CRC_ISCSI.checksum(&shard) != shard_crcs[shard_index] {
                return Err(RsError::ShardCrcMismatch {
                    segment_index: expected_segment_index,
                    shard_index,
                });
            }

            recovered_shards.push(shard);
        }

        let mut data_block = Vec::with_capacity(RS_DATA_BYTES_PER_SEGMENT);
        for shard in recovered_shards.iter().take(RS_DATA_SHARDS) {
            data_block.extend_from_slice(shard);
        }

        let data_len = header.data_len as usize;
        let chunk = data_block[0..data_len].to_vec();
        if CRC_ISCSI.checksum(&chunk) != header.segment_crc32 {
            return Err(RsError::SegmentChecksumMismatch {
                segment_index: expected_segment_index,
            });
        }

        Ok(chunk)
    }

    fn reconstruct_missing_shards(
        codec: &ReedSolomon,
        shards: &mut [Option<Vec<u8>>],
        segment_index: u32,
        missing_shards: usize,
    ) -> Result<(), RsError> {
        if missing_shards > RS_PARITY_SHARDS {
            return Err(RsError::TooManyShardErasures {
                segment_index,
                missing_shards,
                parity_shards: RS_PARITY_SHARDS,
            });
        }

        if missing_shards > 0 {
            codec.reconstruct(shards)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corrupt_shard_bit(
        bytes: &mut [u8],
        segment_index: usize,
        shard_index: usize,
        byte_offset: usize,
        bit: u8,
    ) {
        let segment_offset = RS_RECORD_HEADER_BYTES + segment_index * RS_ENCODED_SEGMENT_BYTES;
        let shard_data_start = segment_offset + RS_SEGMENT_HEADER_BYTES + RS_SHARD_CRC_TABLE_BYTES;
        let target = shard_data_start + shard_index * RS_SHARD_BYTES + byte_offset;
        bytes[target] ^= 1u8 << (bit % 8);
    }

    fn assert_roundtrip_case(name: &str, sequence: u64, payload: &[u8], expected_segments: u32) {
        let encoded = RsStrategy::encode_record(sequence, payload)
            .unwrap_or_else(|error| panic!("{name}: encode failed: {error}"));
        let decoded = RsStrategy::decode_record(&encoded)
            .unwrap_or_else(|error| panic!("{name}: decode failed: {error}"));

        assert_eq!(decoded.sequence, sequence, "{name}: sequence mismatch");
        assert_eq!(decoded.payload, payload, "{name}: payload mismatch");
        assert_eq!(
            decoded.segment_count, expected_segments,
            "{name}: segment count mismatch"
        );
    }

    /// RSレコード境界（空/境界/超過）での往復整合性を検証する。
    #[test]
    fn test_rs_value_range() {
        let cases = [
            ("empty_payload", 1u64, Vec::new(), 0u32),
            ("single_byte", 2u64, vec![0xAB], 1u32),
            (
                "segment_exact",
                3u64,
                vec![0x11; RS_DATA_BYTES_PER_SEGMENT],
                1u32,
            ),
            (
                "segment_plus_one",
                4u64,
                vec![0x22; RS_DATA_BYTES_PER_SEGMENT + 1],
                2u32,
            ),
        ];

        for (name, sequence, payload, expected_segments) in cases {
            assert_roundtrip_case(name, sequence, &payload, expected_segments);
        }
    }

    /// 1〜2シャード破損（パリティ範囲内）で復元できることを検証する。
    #[test]
    fn test_rs_ok_cases() {
        let payload = (0..(RS_DATA_BYTES_PER_SEGMENT + 777))
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();

        let cases = [
            ("single_shard_corruption", 0usize, 0usize, 9usize, 1u8),
            ("two_shard_corruption", 0usize, 1usize, 15usize, 2u8),
            ("two_shard_corruption_parity", 0usize, 8usize, 31usize, 3u8),
        ];

        for (name, segment_index, shard_index, byte_offset, bit) in cases {
            let mut encoded = RsStrategy::encode_record(31, &payload)
                .unwrap_or_else(|error| panic!("{name}: encode failed: {error}"));

            corrupt_shard_bit(&mut encoded, segment_index, shard_index, byte_offset, bit);
            if name.starts_with("two_shard") {
                corrupt_shard_bit(
                    &mut encoded,
                    segment_index,
                    shard_index + 1,
                    byte_offset + 1,
                    bit + 1,
                );
            }

            let decoded = RsStrategy::decode_record(&encoded)
                .unwrap_or_else(|error| panic!("{name}: decode failed: {error}"));
            assert_eq!(decoded.payload, payload, "{name}: payload mismatch");
        }
    }

    /// 破損過多や不正フォーマットで明示エラーを返すことを検証する。
    #[test]
    fn test_rs_error_cases() {
        let payload = vec![0x33; RS_DATA_BYTES_PER_SEGMENT + 256];
        let encoded = RsStrategy::encode_record(41, &payload).unwrap();

        let mut invalid_magic = encoded.clone();
        invalid_magic[0] ^= 0x01;

        let mut too_many_erasures = encoded.clone();
        corrupt_shard_bit(&mut too_many_erasures, 0, 0, 0, 1);
        corrupt_shard_bit(&mut too_many_erasures, 0, 1, 1, 2);
        corrupt_shard_bit(&mut too_many_erasures, 0, 2, 2, 3);

        let cases = [
            ("slot_too_short", encoded[..12].to_vec()),
            ("invalid_magic", invalid_magic),
            ("too_many_erasures", too_many_erasures),
        ];

        for (name, bytes) in cases {
            let error = match RsStrategy::decode_record(&bytes) {
                Ok(_) => panic!("{name}: expected decode error"),
                Err(error) => error,
            };

            match name {
                "slot_too_short" => {
                    assert!(matches!(error, RsError::SlotTooShort), "{name}: {error}");
                }
                "invalid_magic" => {
                    assert!(
                        matches!(error, RsError::InvalidRecordMagic),
                        "{name}: {error}"
                    );
                }
                "too_many_erasures" => {
                    assert!(
                        matches!(error, RsError::TooManyShardErasures { .. }),
                        "{name}: {error}"
                    );
                }
                _ => unreachable!(),
            }
        }
    }
}
