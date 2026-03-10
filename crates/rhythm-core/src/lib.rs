use serde::{Deserialize, Serialize};

pub mod comm;
pub mod consts;
pub mod fixed_math;
pub mod util;

use consts::RHYTHM_MESSAGE_WIRE_SIZE;
use fixed_math::{phase_advance_sub16, BpmQ8, PhaseU16};

// 下流クレートが使う公開定数・ユーティリティを再エクスポートする。
pub use consts::{BPM_Q8_ONE, BPM_Q8_SHIFT, MS_PER_MINUTE, PHASE_MODULUS};
pub use fixed_math::BpmLimitParam;
pub use util::{
    bpm_from_int, bpm_from_interval_ms, bpm_to_int_floor, bpm_to_int_round, fast_sin_q1_15,
};

/// 既存サンプル互換のためのエイリアス。
pub type Rhythm = RhythmGenerator;

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
    pub current_bpm: BpmQ8,
    pub beat_count: u64,
    pub sync_state: SyncState,

    coupling_divisor: u16,
    phase_accum_sub16: u64,
    first_point_ts_ms: Option<u64>,
    last_sync_ts_ms: Option<u64>,
}

/// 低頻度入力でのジッタ耐性を持たせるための同期状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// 自律動作。
    Idle,
    /// 1点目受信後。2点目待ち。
    WaitSecondPoint,
    /// 2点観測済み。BPM確定済み。
    Locked,
}

impl RhythmGenerator {
    /// Q8.8 BPM 指定でジェネレータを生成する。
    pub fn new(phase: u16, bpm: BpmQ8, coupling_divisor: u16) -> Self {
        Self {
            phase,
            base_bpm: bpm,
            current_bpm: bpm,
            beat_count: 0,
            sync_state: SyncState::Idle,
            coupling_divisor: coupling_divisor.max(2),
            phase_accum_sub16: (phase as u64) << 16,
            first_point_ts_ms: None,
            last_sync_ts_ms: None,
        }
    }

    /// 整数 BPM 指定でジェネレータを生成する。
    pub fn from_int_bpm(phase: u16, bpm: u16, coupling_divisor: u16) -> Self {
        Self::new(phase, BpmQ8::from_int(bpm), coupling_divisor)
    }

    /// 自律更新。パケット不在時は `base_bpm` を目標に `current_bpm` を徐々に戻す。
    /// BPM 値は毎回 `bpm_limit_param` の範囲に制約される。
    pub fn update(&mut self, dt_ms: u32, bpm_limit_param: &BpmLimitParam) {
        if dt_ms == 0 {
            return;
        }

        let bpm_limit_param = bpm_limit_param.sanitize();
        self.base_bpm = self.base_bpm.clamp_with_limit(&bpm_limit_param);
        self.current_bpm = self.current_bpm.clamp_with_limit(&bpm_limit_param);

        self.current_bpm = self.current_bpm.blend_toward_with_limit(
            self.base_bpm,
            self.coupling_divisor as i32,
            &bpm_limit_param,
        );

        let before_cycles = self.phase_accum_sub16 >> 32;
        let delta_sub16 = phase_advance_sub16(self.current_bpm.raw(), dt_ms);
        self.phase_accum_sub16 = self.phase_accum_sub16.wrapping_add(delta_sub16);
        let after_cycles = self.phase_accum_sub16 >> 32;

        let wraps = after_cycles.saturating_sub(before_cycles);
        self.beat_count = self.beat_count.saturating_add(wraps);
        self.phase = ((self.phase_accum_sub16 >> 16) & u16::MAX as u64) as u16;
    }

    /// 外部メッセージに同期する。
    ///
    /// - 遅延補償: `now_ms - msg.timestamp_ms` ぶん相手位相を進める
    /// - 2点観測: 1点目で待機、2点目で BPM 確定
    /// - BPM 範囲外: 2:1 分周で 60-120 BPM に折りたたむ
    /// - 位相: 90度刻みオフセットを候補に、現在位相へ最短の点へ吸着
    pub fn sync(&mut self, msg: RhythmMessage, now_ms: u64, bpm_limit_param: &BpmLimitParam) {
        let bpm_limit_param = bpm_limit_param.sanitize();
        let compensated_remote_phase = compensated_remote_phase(msg, now_ms);
        let phase_target = if is_bpm_in_primary_range(msg.bpm, &bpm_limit_param) {
            compensated_remote_phase
        } else {
            nearest_quarter_phase(self.phase, compensated_remote_phase)
        };
        let hinted_bpm = normalize_bpm_to_primary_range(msg.bpm, &bpm_limit_param);

        match self.sync_state {
            SyncState::Idle => {
                self.sync_state = SyncState::WaitSecondPoint;
                self.first_point_ts_ms = Some(msg.timestamp_ms);

                // 1点目でもリズム感を外さないよう、ヒントBPMへ強めに寄せる。
                self.base_bpm = hinted_bpm;
                self.current_bpm =
                    self.current_bpm
                        .blend_toward_with_limit(hinted_bpm, 2, &bpm_limit_param);
                self.force_phase(phase_target);
            }
            SyncState::WaitSecondPoint => {
                let maybe_interval_ms = self
                    .first_point_ts_ms
                    .filter(|first_ts| msg.timestamp_ms > *first_ts)
                    .map(|first_ts| (msg.timestamp_ms - first_ts).min(u32::MAX as u64) as u32);

                if let Some(interval_ms) = maybe_interval_ms {
                    let observed_bpm = normalize_bpm_to_primary_range(
                        BpmQ8::from_interval_ms(interval_ms),
                        &bpm_limit_param,
                    );
                    self.base_bpm = observed_bpm;
                    self.current_bpm = observed_bpm;
                    self.sync_state = SyncState::Locked;
                    self.first_point_ts_ms = None;
                } else {
                    self.first_point_ts_ms = Some(msg.timestamp_ms);
                    self.base_bpm = hinted_bpm;
                    self.current_bpm =
                        self.current_bpm
                            .blend_toward_with_limit(hinted_bpm, 2, &bpm_limit_param);
                }
                self.force_phase(phase_target);
            }
            SyncState::Locked => {
                if let Some(last_ts_ms) = self.last_sync_ts_ms {
                    if msg.timestamp_ms > last_ts_ms {
                        let interval_ms =
                            (msg.timestamp_ms - last_ts_ms).min(u32::MAX as u64) as u32;
                        let observed_bpm = normalize_bpm_to_primary_range(
                            BpmQ8::from_interval_ms(interval_ms),
                            &bpm_limit_param,
                        );
                        // ロック中はジッタを抑えつつ追従する。
                        self.base_bpm = self.base_bpm.blend_toward_with_limit(
                            observed_bpm,
                            4,
                            &bpm_limit_param,
                        );
                    }
                }

                self.base_bpm =
                    self.base_bpm
                        .blend_toward_with_limit(hinted_bpm, 4, &bpm_limit_param);
                self.current_bpm =
                    self.current_bpm
                        .blend_toward_with_limit(self.base_bpm, 2, &bpm_limit_param);
                self.force_phase(phase_target);
            }
        }

        self.last_sync_ts_ms = Some(msg.timestamp_ms);
    }

    pub fn to_message(&self, now_ms: u64) -> RhythmMessage {
        RhythmMessage {
            timestamp_ms: now_ms,
            beat_count: self.beat_count.min(u32::MAX as u64) as u32,
            phase: self.phase,
            bpm: self.current_bpm,
        }
    }

    #[inline]
    fn force_phase(&mut self, phase: u16) {
        self.phase = phase;
        let cycles = self.phase_accum_sub16 >> 32;
        self.phase_accum_sub16 = (cycles << 32) | ((phase as u64) << 16);
    }
}

#[inline]
fn is_bpm_in_primary_range(bpm: BpmQ8, bpm_limit_param: &BpmLimitParam) -> bool {
    bpm_limit_param.contains_q8(bpm)
}

#[inline]
fn compensated_remote_phase(msg: RhythmMessage, now_ms: u64) -> u16 {
    let delay_ms = now_ms.saturating_sub(msg.timestamp_ms).min(u32::MAX as u64) as u32;
    PhaseU16(msg.phase)
        .wrapping_add(PhaseU16::advance(msg.bpm, delay_ms))
        .raw()
}

#[inline]
fn nearest_quarter_phase(current_phase: u16, remote_phase: u16) -> u16 {
    const QUARTER_OFFSETS: [u16; 4] = [0, 16_384, 32_768, 49_152];

    let mut best = remote_phase;
    let mut best_abs = i32::MAX;
    for offset in QUARTER_OFFSETS {
        let candidate = remote_phase.wrapping_add(offset);
        let abs = (PhaseU16(candidate).signed_diff(PhaseU16(current_phase)) as i32).abs();
        if abs < best_abs {
            best_abs = abs;
            best = candidate;
        }
    }
    best
}

#[inline]
fn normalize_bpm_to_primary_range(bpm: BpmQ8, bpm_limit_param: &BpmLimitParam) -> BpmQ8 {
    let bpm_limit_param = bpm_limit_param.sanitize();
    let min = bpm_limit_param.min_q8().raw() as u32;
    let max = bpm_limit_param.max_q8().raw() as u32;

    let mut raw = bpm.raw() as u32;
    if raw == 0 {
        return bpm_limit_param.min_q8();
    }

    while raw > max {
        // 高すぎるテンポは 2:1 分周へ折りたたむ。
        raw = raw.div_ceil(2);
    }
    while raw < min {
        raw = raw.saturating_mul(2);
        if raw == 0 {
            return bpm_limit_param.min_q8();
        }
    }

    BpmQ8(raw as u16).clamp_with_limit(&bpm_limit_param)
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
