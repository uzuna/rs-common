use serde::{Deserialize, Serialize};

pub mod comm;
pub mod consts;
pub mod fixed_math;
pub mod util;

use consts::RHYTHM_MESSAGE_WIRE_SIZE;

// 下流クレートや結合テストで頻繁に使うユーティリティを再エクスポートする。
pub use util::bpm_from_int;

use crate::fixed_math::BpmQ8;

/// ネットワーク共有用メッセージ
///
/// 16バイトの固定レイアウトで、UDPマルチキャストなどで送受信することを想定する。
/// シリアライズはリトルエンディアンで行い、受信側で環境に応じて変換する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[repr(C)]
pub struct RhythmMessage {
    pub timestamp_ms: u64,
    pub beat_count: u32,
    pub phase: u16,
    pub bpm: BpmQ8,
}

impl RhythmMessage {
    /// 送信タイムスタンプ (u64) のオフセット。
    pub const WIRE_TIMESTAMP_OFFSET: usize = 0;
    /// beat_count (u32) のオフセット。
    pub const WIRE_BEAT_COUNT_OFFSET: usize = 8;
    /// phase (u16) のオフセット。
    pub const WIRE_PHASE_OFFSET: usize = 12;
    /// bpm (u16) のオフセット。
    pub const WIRE_BPM_OFFSET: usize = 14;

    pub const fn new(timestamp_ms: u64, beat_count: u32, phase: u16, bpm: BpmQ8) -> Self {
        Self {
            timestamp_ms,
            beat_count,
            phase,
            bpm,
        }
    }

    /// BigEndian環境の場合は Little Endian に変換してからシリアライズする。
    pub fn to_wire_bytes(self) -> [u8; RHYTHM_MESSAGE_WIRE_SIZE] {
        let mut buf = [0u8; RHYTHM_MESSAGE_WIRE_SIZE];
        buf[Self::WIRE_TIMESTAMP_OFFSET..Self::WIRE_TIMESTAMP_OFFSET + 8]
            .copy_from_slice(&self.timestamp_ms.to_le_bytes());
        buf[Self::WIRE_BEAT_COUNT_OFFSET..Self::WIRE_BEAT_COUNT_OFFSET + 4]
            .copy_from_slice(&self.beat_count.to_le_bytes());
        buf[Self::WIRE_PHASE_OFFSET..Self::WIRE_PHASE_OFFSET + 2]
            .copy_from_slice(&self.phase.to_le_bytes());
        buf[Self::WIRE_BPM_OFFSET..Self::WIRE_BPM_OFFSET + 2]
            .copy_from_slice(&self.bpm.to_int_round().to_le_bytes());
        buf
    }

    /// リトルエンディアンのバイトスライスからデシリアライズする。
    pub fn from_wire_slice(buf: &[u8]) -> Option<Self> {
        if buf.len() < RHYTHM_MESSAGE_WIRE_SIZE {
            return None;
        }
        let timestamp_ms = u64::from_le_bytes(
            buf[Self::WIRE_TIMESTAMP_OFFSET..Self::WIRE_TIMESTAMP_OFFSET + 8]
                .try_into()
                .ok()?,
        );
        let beat_count = u32::from_le_bytes(
            buf[Self::WIRE_BEAT_COUNT_OFFSET..Self::WIRE_BEAT_COUNT_OFFSET + 4]
                .try_into()
                .ok()?,
        );
        let phase = u16::from_le_bytes(
            buf[Self::WIRE_PHASE_OFFSET..Self::WIRE_PHASE_OFFSET + 2]
                .try_into()
                .ok()?,
        );
        let bpm = u16::from_le_bytes(
            buf[Self::WIRE_BPM_OFFSET..Self::WIRE_BPM_OFFSET + 2]
                .try_into()
                .ok()?,
        );
        Some(Self {
            timestamp_ms,
            beat_count,
            phase,
            bpm: BpmQ8::from_int(bpm),
        })
    }
}

/// 位相生成と外部同期（蔵本モデル）を担当するコア。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhythmGenerator {
    pub phase: u16,
    pub base_bpm: BpmQ8,
    pub beat_count: u64,
}

impl RhythmGenerator {
    pub fn to_message(&self, now_ms: u64) -> RhythmMessage {
        RhythmMessage {
            timestamp_ms: now_ms,
            beat_count: self.beat_count.min(u32::MAX as u64) as u32,
            phase: self.phase,
            bpm: self.base_bpm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use consts::RHYTHM_MESSAGE_WIRE_SIZE;

    // ── 正常系: シリアライズ→デシリアライズの往復 ─────────────────────────────────

    /// to_wire_bytes → from_wire_slice の往復で元の値に戻ること。
    #[test]
    fn wire_bytes_roundtrip() {
        let cases: &[(&str, RhythmMessage)] = &[
            ("全ゼロ（デフォルト）", RhythmMessage::default()),
            (
                "120 BPM / 標準",
                RhythmMessage::new(1_000, 1, 0, BpmQ8::from_int(120)),
            ),
            (
                "90 BPM / 標準",
                RhythmMessage::new(2_000, 10, 32768, BpmQ8::from_int(90)),
            ),
            (
                "60 BPM / 標準",
                RhythmMessage::new(3_000, 100, 65535, BpmQ8::from_int(60)),
            ),
            (
                "タイムスタンプ最大値",
                RhythmMessage::new(u64::MAX, 0, 0, BpmQ8::from_int(120)),
            ),
            (
                "beat_count 最大値",
                RhythmMessage::new(0, u32::MAX, 0, BpmQ8::from_int(90)),
            ),
            (
                "phase 最大値",
                RhythmMessage::new(0, 0, u16::MAX, BpmQ8::from_int(60)),
            ),
            (
                "全フィールド境界値",
                RhythmMessage::new(u64::MAX, u32::MAX, u16::MAX, BpmQ8::from_int(120)),
            ),
        ];
        for (label, msg) in cases {
            let buf = msg.to_wire_bytes();
            let restored = RhythmMessage::from_wire_slice(&buf)
                .unwrap_or_else(|| panic!("[{label}] from_wire_slice が None を返した"));
            assert_eq!(restored, *msg, "[{label}] 往復後の値が一致しない");
        }
    }

    // ── 値域確認: ワイヤーバイトの各フィールドのバイト位置 ─────────────────────────

    /// 各フィールドが仕様のオフセット位置に正しく書き込まれていること。
    #[test]
    fn wire_bytes_field_layout() {
        // (label, msg, timestamp_ms, beat_count, phase, bpm_int)
        let cases: &[(&str, RhythmMessage, u64, u32, u16, u16)] = &[
            (
                "通常値",
                RhythmMessage::new(
                    0x0102_0304_0506_0708,
                    0xDEAD_BEEF,
                    0xABCD,
                    BpmQ8::from_int(120),
                ),
                0x0102_0304_0506_0708,
                0xDEAD_BEEF,
                0xABCD,
                120,
            ),
            ("全ゼロ", RhythmMessage::default(), 0, 0, 0, 0),
        ];
        for (label, msg, ts, bc, ph, bpm_int) in cases {
            let buf = msg.to_wire_bytes();
            assert_eq!(
                u64::from_le_bytes(
                    buf[RhythmMessage::WIRE_TIMESTAMP_OFFSET..][..8]
                        .try_into()
                        .unwrap()
                ),
                *ts,
                "[{label}] timestamp_ms",
            );
            assert_eq!(
                u32::from_le_bytes(
                    buf[RhythmMessage::WIRE_BEAT_COUNT_OFFSET..][..4]
                        .try_into()
                        .unwrap()
                ),
                *bc,
                "[{label}] beat_count",
            );
            assert_eq!(
                u16::from_le_bytes(
                    buf[RhythmMessage::WIRE_PHASE_OFFSET..][..2]
                        .try_into()
                        .unwrap()
                ),
                *ph,
                "[{label}] phase",
            );
            assert_eq!(
                u16::from_le_bytes(
                    buf[RhythmMessage::WIRE_BPM_OFFSET..][..2]
                        .try_into()
                        .unwrap()
                ),
                *bpm_int,
                "[{label}] bpm",
            );
        }
    }

    // ── 異常系: バッファ不足で None を返す ─────────────────────────────────────────

    /// バッファ長が WIRE_SIZE 未満の場合は from_wire_slice が None を返すこと。
    #[test]
    fn wire_bytes_short_buffer_returns_none() {
        for len in 0..RHYTHM_MESSAGE_WIRE_SIZE {
            let short = vec![0u8; len];
            assert!(
                RhythmMessage::from_wire_slice(&short).is_none(),
                "長さ {len} のバッファで None が返らなかった"
            );
        }
    }
}
