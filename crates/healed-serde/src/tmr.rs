use crc::{Crc, CRC_32_ISCSI};
use thiserror::Error;

const TMR_MAGIC: [u8; 4] = *b"TMR1";
const TMR_VERSION: u32 = 1;
const TMR_HEADER_BYTES: usize = 24;
const TMR_REPLICA_COUNT: usize = 3;
pub const TMR_HEADER_GROUP_BYTES: usize = TMR_HEADER_BYTES * TMR_REPLICA_COUNT;

const CRC_ISCSI: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// TMRで復元したスロットデータです。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmrDecodedSlot {
    pub sequence: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TmrHeader {
    sequence: u64,
    payload_len: u32,
    crc32: u32,
}

impl TmrHeader {
    fn new(sequence: u64, payload: &[u8]) -> Result<Self, TmrError> {
        let payload_len = u32::try_from(payload.len()).map_err(|_| TmrError::PayloadTooLarge)?;
        Ok(Self {
            sequence,
            payload_len,
            crc32: CRC_ISCSI.checksum(payload),
        })
    }

    fn to_bytes(self) -> [u8; TMR_HEADER_BYTES] {
        let mut bytes = [0u8; TMR_HEADER_BYTES];
        bytes[0..4].copy_from_slice(&TMR_MAGIC);
        bytes[4..8].copy_from_slice(&TMR_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.crc32.to_le_bytes());
        bytes
    }

    fn from_bytes(bytes: &[u8; TMR_HEADER_BYTES]) -> Result<Self, TmrError> {
        if bytes[0..4] != TMR_MAGIC {
            return Err(TmrError::InvalidMagic);
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != TMR_VERSION {
            return Err(TmrError::UnsupportedVersion(version));
        }

        Ok(Self {
            sequence: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            payload_len: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            crc32: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        })
    }
}

/// TMR処理時のエラーです。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TmrError {
    #[error("TMR payload is too large to encode")]
    PayloadTooLarge,

    #[error("TMR slot is too short")]
    SlotTooShort,

    #[error("TMR slot payload replicas have invalid length")]
    InvalidReplicaLength,

    #[error("TMR header magic is invalid")]
    InvalidMagic,

    #[error("TMR header version {0} is not supported")]
    UnsupportedVersion(u32),

    #[error("TMR payload checksum mismatch")]
    ChecksumMismatch,
}

/// 小サイズデータ向けのTMR戦略です。
pub struct TmrStrategy;

impl TmrStrategy {
    /// 3重化されたスロットを生成します。
    pub fn encode_tmr(sequence: u64, payload: &[u8]) -> Result<Vec<u8>, TmrError> {
        let header = TmrHeader::new(sequence, payload)?;
        let mut encoded =
            Vec::with_capacity(TMR_HEADER_GROUP_BYTES + payload.len() * TMR_REPLICA_COUNT);

        for _ in 0..TMR_REPLICA_COUNT {
            encoded.extend_from_slice(&header.to_bytes());
        }
        for _ in 0..TMR_REPLICA_COUNT {
            encoded.extend_from_slice(payload);
        }

        Ok(encoded)
    }

    /// 3つのレプリカから多数決で元データを復元します。
    pub fn decode_tmr_with_vote(bytes: &[u8]) -> Result<TmrDecodedSlot, TmrError> {
        if bytes.len() < TMR_HEADER_GROUP_BYTES {
            return Err(TmrError::SlotTooShort);
        }

        let payload_replicas_bytes = bytes.len() - TMR_HEADER_GROUP_BYTES;
        if !payload_replicas_bytes.is_multiple_of(TMR_REPLICA_COUNT) {
            return Err(TmrError::InvalidReplicaLength);
        }

        let payload_len = payload_replicas_bytes / TMR_REPLICA_COUNT;
        let header = Self::decode_header_with_vote(&bytes[..TMR_HEADER_GROUP_BYTES])?;
        if header.payload_len as usize != payload_len {
            return Err(TmrError::InvalidReplicaLength);
        }

        let payload_start = TMR_HEADER_GROUP_BYTES;
        let replica0 = &bytes[payload_start..payload_start + payload_len];
        let replica1 = &bytes[payload_start + payload_len..payload_start + (payload_len * 2)];
        let replica2 = &bytes[payload_start + (payload_len * 2)..payload_start + (payload_len * 3)];
        let payload = majority_vote_bytes(replica0, replica1, replica2);

        if CRC_ISCSI.checksum(&payload) != header.crc32 {
            return Err(TmrError::ChecksumMismatch);
        }

        Ok(TmrDecodedSlot {
            sequence: header.sequence,
            payload,
        })
    }

    /// ヘッダーだけからシーケンス番号を取り出します。
    pub fn peek_sequence(header_bytes: &[u8]) -> Option<u64> {
        if header_bytes.len() < TMR_HEADER_GROUP_BYTES {
            return None;
        }

        let header = Self::decode_header_with_vote(&header_bytes[..TMR_HEADER_GROUP_BYTES]).ok()?;
        Some(header.sequence)
    }

    fn decode_header_with_vote(bytes: &[u8]) -> Result<TmrHeader, TmrError> {
        if bytes.len() < TMR_HEADER_GROUP_BYTES {
            return Err(TmrError::SlotTooShort);
        }

        let replica0: &[u8; TMR_HEADER_BYTES] = bytes[0..TMR_HEADER_BYTES].try_into().unwrap();
        let replica1: &[u8; TMR_HEADER_BYTES] = bytes[TMR_HEADER_BYTES..TMR_HEADER_BYTES * 2]
            .try_into()
            .unwrap();
        let replica2: &[u8; TMR_HEADER_BYTES] = bytes[TMR_HEADER_BYTES * 2..TMR_HEADER_GROUP_BYTES]
            .try_into()
            .unwrap();
        let voted = majority_vote_bytes(replica0, replica1, replica2);
        let voted_header: [u8; TMR_HEADER_BYTES] = voted.try_into().unwrap();
        TmrHeader::from_bytes(&voted_header)
    }
}

fn majority_vote_bytes(a: &[u8], b: &[u8], c: &[u8]) -> Vec<u8> {
    debug_assert_eq!(a.len(), b.len());
    debug_assert_eq!(b.len(), c.len());

    a.iter()
        .zip(b.iter())
        .zip(c.iter())
        .map(|((&lhs, &mid), &rhs)| (lhs & mid) | (lhs & rhs) | (mid & rhs))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_bits(lhs: &[u8], rhs: &[u8]) -> usize {
        assert_eq!(lhs.len(), rhs.len(), "length mismatch");
        lhs.iter()
            .zip(rhs.iter())
            .map(|(l, r)| (l ^ r).count_ones() as usize)
            .sum()
    }

    fn corrupt_bit(bytes: &mut [u8], offset: usize, bit: u8) {
        bytes[offset] ^= 1u8 << bit;
    }

    #[test]
    fn test_tmr_value_range() {
        let cases = [
            ("empty_payload", 7u64, Vec::new()),
            ("single_byte", 8u64, vec![0x5A]),
            ("small_payload", 9u64, vec![0xAB; 128]),
        ];

        for (name, sequence, payload) in cases {
            let encoded = TmrStrategy::encode_tmr(sequence, &payload)
                .unwrap_or_else(|e| panic!("{name}: encode failed: {e}"));
            let decoded = TmrStrategy::decode_tmr_with_vote(&encoded)
                .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));

            assert_eq!(decoded.sequence, sequence, "{name}: sequence mismatch");
            assert_eq!(decoded.payload, payload, "{name}: payload mismatch");
        }
    }

    #[test]
    fn test_tmr_ok_cases() {
        let cases = [
            (
                "header_one_bit_corruption",
                21u64,
                vec![0x11; 64],
                3usize,
                0u8,
            ),
            (
                "payload_one_bit_corruption",
                22u64,
                vec![0x22; 96],
                TMR_HEADER_GROUP_BYTES + 8,
                2u8,
            ),
            (
                "payload_mbu_single_replica",
                23u64,
                vec![0x33; 128],
                TMR_HEADER_GROUP_BYTES + 40,
                4u8,
            ),
        ];

        for (name, sequence, payload, offset, bit) in cases {
            let mut encoded = TmrStrategy::encode_tmr(sequence, &payload)
                .unwrap_or_else(|e| panic!("{name}: encode failed: {e}"));
            corrupt_bit(&mut encoded, offset, bit % 8);

            let decoded = TmrStrategy::decode_tmr_with_vote(&encoded)
                .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
            assert_eq!(decoded.sequence, sequence, "{name}: sequence mismatch");
            assert_eq!(decoded.payload, payload, "{name}: payload mismatch");
        }
    }

    #[test]
    fn test_tmr_error_cases() {
        let payload = vec![0x44; 48];
        let encoded = TmrStrategy::encode_tmr(31, &payload).unwrap();

        let mut invalid_length = encoded.clone();
        invalid_length.pop();

        let mut checksum_break = encoded.clone();
        let payload_start = TMR_HEADER_GROUP_BYTES;
        let second_replica_payload_start = payload_start + payload.len();
        corrupt_bit(&mut checksum_break, payload_start, 0);
        corrupt_bit(&mut checksum_break, second_replica_payload_start, 0);

        let cases = [
            (
                "slot_too_short",
                encoded[..16].to_vec(),
                TmrError::SlotTooShort,
            ),
            (
                "invalid_replica_length",
                invalid_length,
                TmrError::InvalidReplicaLength,
            ),
            (
                "checksum_mismatch",
                checksum_break,
                TmrError::ChecksumMismatch,
            ),
        ];

        for (name, bytes, expected) in cases {
            let error = match TmrStrategy::decode_tmr_with_vote(&bytes) {
                Ok(_) => panic!("{name}: expected decode error"),
                Err(error) => error,
            };
            assert_eq!(error, expected, "{name}: unexpected error");
        }
    }

    #[test]
    fn test_tmr_majority_vote_diff_bits() {
        let payload = vec![0x77; 32];
        let encoded = TmrStrategy::encode_tmr(41, &payload).unwrap();
        let decoded = TmrStrategy::decode_tmr_with_vote(&encoded).unwrap();

        assert_eq!(diff_bits(&decoded.payload, &payload), 0);
    }
}
