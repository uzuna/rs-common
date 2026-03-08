#![no_std]

use core::time::Duration;

use serde::{Deserialize, Serialize};

/// 1分あたりのミリ秒。
pub const MS_PER_MINUTE: u32 = 60_000;
/// サインLUTの振幅スケール（i16最大付近）。
pub const LUT_SCALE: i32 = 32_767;

/// 共有用バイナリレイアウトの全体サイズ。
pub const RHYTHM_MESSAGE_WIRE_SIZE: usize = 24;
/// 送信タイムスタンプ(u64)のオフセット。
pub const RHYTHM_MESSAGE_WIRE_TIMESTAMP_OFFSET: usize = 0;
/// cycle_count(=beat_count, u64)のオフセット。
pub const RHYTHM_MESSAGE_WIRE_CYCLE_COUNT_OFFSET: usize = 8;
/// phase(u16)のオフセット。
pub const RHYTHM_MESSAGE_WIRE_PHASE_OFFSET: usize = 16;
/// bpm(u16)のオフセット。
pub const RHYTHM_MESSAGE_WIRE_BPM_OFFSET: usize = 18;
/// 予約領域(4byte)のオフセット。
pub const RHYTHM_MESSAGE_WIRE_RESERVED_OFFSET: usize = 20;
/// 予約領域サイズ。
pub const RHYTHM_MESSAGE_WIRE_RESERVED_SIZE: usize = 4;

/// 0..2π を 256 分割した整数サインテーブル。
#[rustfmt::skip]
pub const SIN_LUT: [i16; 256] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787,
    21403, 22005, 22594, 23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683,
    28105, 28510, 28898, 29268, 29621, 29956, 30273, 30571, 30852, 31113, 31356, 31580, 31785,
    31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757, 32767, 32757, 32728, 32678,
    32609, 32521, 32412, 32285, 32137, 31971, 31785, 31580, 31356, 31113, 30852, 30571, 30273,
    29956, 29621, 29268, 28898, 28510, 28105, 27683, 27245, 26790, 26319, 25832, 25329, 24811,
    24279, 23731, 23170, 22594, 22005, 21403, 20787, 20159, 19519, 18868, 18204, 17530, 16846,
    16151, 15446, 14732, 14010, 13279, 12539, 11793, 11039, 10278, 9512, 8739, 7962, 7179, 6393,
    5602, 4808, 4011, 3212, 2410, 1608, 804, 0, -804, -1608, -2410, -3212, -4011, -4808, -5602,
    -6393, -7179, -7962, -8739, -9512, -10278, -11039, -11793, -12539, -13279, -14010, -14732,
    -15446, -16151, -16846, -17530, -18204, -18868, -19519, -20159, -20787, -21403, -22005,
    -22594, -23170, -23731, -24279, -24811, -25329, -25832, -26319, -26790, -27245, -27683,
    -28105, -28510, -28898, -29268, -29621, -29956, -30273, -30571, -30852, -31113, -31356,
    -31580, -31785, -31971, -32137, -32285, -32412, -32521, -32609, -32678, -32728, -32757,
    -32767, -32757, -32728, -32678, -32609, -32521, -32412, -32285, -32137, -31971, -31785,
    -31580, -31356, -31113, -30852, -30571, -30273, -29956, -29621, -29268, -28898, -28510,
    -28105, -27683, -27245, -26790, -26319, -25832, -25329, -24811, -24279, -23731, -23170,
    -22594, -22005, -21403, -20787, -20159, -19519, -18868, -18204, -17530, -16846, -16151,
    -15446, -14732, -14010, -13279, -12539, -11793, -11039, -10278, -9512, -8739, -7962, -7179,
    -6393, -5602, -4808, -4011, -3212, -2410, -1608, -804,
];

/// ネットワーク共有用の最小メッセージ。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RhythmMessage {
    /// メッセージ生成時のタイムスタンプ（ミリ秒）。
    pub timestamp_ms: u64,
    /// 累積ビート数（位相ラップした回数）。
    pub beat_count: u64,
    /// 位相値。
    pub phase: u16,
    /// BPM 値。
    pub bpm: u16,
    /// 予約領域。
    #[serde(default)]
    pub reserved: [u8; RHYTHM_MESSAGE_WIRE_RESERVED_SIZE],
}

impl RhythmMessage {
    /// 生成ヘルパー（予約領域は0埋め）。
    pub const fn new(phase: u16, bpm: u16, timestamp_ms: u64, beat_count: u64) -> Self {
        Self {
            timestamp_ms,
            beat_count,
            phase,
            bpm,
            reserved: [0_u8; RHYTHM_MESSAGE_WIRE_RESERVED_SIZE],
        }
    }

    /// 固定24byteレイアウトへ構造体のメモリをそのまま書き出す。
    pub fn to_wire_bytes(&self) -> [u8; RHYTHM_MESSAGE_WIRE_SIZE] {
        let mut out = [0_u8; RHYTHM_MESSAGE_WIRE_SIZE];

        debug_assert_eq!(core::mem::size_of::<Self>(), RHYTHM_MESSAGE_WIRE_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(
                (self as *const Self).cast::<u8>(),
                out.as_mut_ptr(),
                RHYTHM_MESSAGE_WIRE_SIZE,
            );
        }

        out
    }

    /// 固定24byteレイアウトを構造体としてそのまま復元する。
    pub fn from_wire_bytes(bytes: [u8; RHYTHM_MESSAGE_WIRE_SIZE]) -> Self {
        debug_assert_eq!(core::mem::size_of::<Self>(), RHYTHM_MESSAGE_WIRE_SIZE);
        unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<Self>()) }
    }

    /// 任意バイト列から固定24byteレイアウトを安全に復元する。
    pub fn from_wire_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != RHYTHM_MESSAGE_WIRE_SIZE {
            return None;
        }

        debug_assert_eq!(core::mem::size_of::<Self>(), RHYTHM_MESSAGE_WIRE_SIZE);
        Some(unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<Self>()) })
    }
}

/// 位相生成と外部同期（引き込み）を担当するコア。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhythmGenerator {
    /// 最新の位相
    pub phase: u16,
    /// これまでの積算周期カウント数。
    pub beat_count: u64,
    /// 外部シグナルがないときの基準テンポ。
    pub base_bpm: u16,
    /// 現在テンポ（同期で変化しうる）。
    pub current_bpm: u16,
    /// BPM 補正の強さを表す係数。大きいほど外部シグナルに引き込まれやすい。
    pub k: u16,
}

pub type Rhythm = RhythmGenerator;

impl RhythmGenerator {
    /// 初期位相・基準BPM・同期強度を指定して生成する。
    pub fn new(phase: u16, base_bpm: u16, k: u16) -> Self {
        Self {
            phase,
            beat_count: 0,
            base_bpm,
            current_bpm: base_bpm,
            k,
        }
    }

    /// 位相0で生成する簡易コンストラクタ。
    pub fn with_bpm(base_bpm: u16, k: u16) -> Self {
        Self::new(0, base_bpm, k)
    }

    /// 現在BPMに従って位相を進める（ラップアラウンド込み）。
    pub fn update(&mut self, dt: Duration) {
        let (step, wraps) = self.phase_step_and_wraps(self.current_bpm, dt);
        self.phase = self.phase.wrapping_add(step);
        self.beat_count = self.beat_count.saturating_add(wraps);
    }

    /// 外部位相との差から蔵本モデル風に BPM を補正する。
    pub fn sync(&mut self, target_phase: u16) {
        let diff = self.phase_error(target_phase);
        let sin = sin_from_phase_diff(diff, phase_modulus());

        let base_bpm = self.base_bpm as i64;
        let k = self.k as i64;
        let correction = (k * sin as i64) / LUT_SCALE as i64;

        let next_bpm = clamp_i64(base_bpm + correction, 0, u16::MAX as i64);
        self.current_bpm = next_bpm as u16;
    }

    /// 位相差を最短距離（±半周以内）で返す。
    pub fn phase_error(&self, target_phase: u16) -> i64 {
        let modulus = phase_modulus() as i64;
        let target = target_phase as i64;
        let current = self.phase as i64;

        let mut diff = (target - current).rem_euclid(modulus);
        if diff > modulus / 2 {
            diff -= modulus;
        }
        diff
    }

    /// 位相差の絶対値。
    pub fn phase_distance(&self, target_phase: u16) -> u64 {
        self.phase_error(target_phase).unsigned_abs()
    }

    /// 現在状態を送信用メッセージに変換する。
    pub fn to_message(&self, timestamp: Duration) -> RhythmMessage {
        let timestamp_ms = duration_to_millis_u64(timestamp);

        RhythmMessage::new(self.phase, self.current_bpm, timestamp_ms, self.beat_count)
    }

    /// 受信時刻を使って、受信メッセージの位相を現在時刻へ外挿する。
    pub fn predict_phase_from_message(message: RhythmMessage, now: Duration) -> u16 {
        let base = Duration::from_millis(message.timestamp_ms);
        let elapsed_ms = now.saturating_sub(base).as_millis();
        let modulus = phase_modulus() as u128;
        let step = (message.bpm as u128 * modulus * elapsed_ms / MS_PER_MINUTE as u128) % modulus;
        message.phase.wrapping_add(step as u16)
    }

    /// 外部ビート入力由来の2メッセージから現在状態を推定し、強制的に同期する。
    pub fn force_sync_from_beat_messages(
        &mut self,
        older: RhythmMessage,
        newer: RhythmMessage,
        now: Duration,
    ) -> bool {
        let Some((estimated_phase, estimated_bpm)) =
            Self::estimate_bpm_phase_from_beat_messages(older, newer, now)
        else {
            return false;
        };

        let elapsed_from_newer_ms = duration_to_millis_u64(now).saturating_sub(newer.timestamp_ms);
        let additional_beats = ((estimated_bpm as u128 * elapsed_from_newer_ms as u128)
            / MS_PER_MINUTE as u128)
            .min(u64::MAX as u128) as u64;

        self.phase = estimated_phase;
        self.beat_count = newer.beat_count.saturating_add(additional_beats);
        self.base_bpm = estimated_bpm;
        self.current_bpm = estimated_bpm;
        true
    }

    /// 2点のビート観測メッセージ（1 beat 間隔）から、現在時刻の BPM と位相を推定する。
    pub fn estimate_bpm_phase_from_beat_messages(
        older: RhythmMessage,
        newer: RhythmMessage,
        now: Duration,
    ) -> Option<(u16, u16)> {
        let observation_ms = newer.timestamp_ms.saturating_sub(older.timestamp_ms);
        if observation_ms == 0 {
            return None;
        }

        let modulus = phase_modulus() as u128;
        let bpm_raw = (MS_PER_MINUTE as u128 + observation_ms as u128 / 2) / observation_ms as u128;
        let estimated_bpm = bpm_raw.min(u16::MAX as u128) as u16;

        let elapsed_from_newer_ms =
            duration_to_millis_u64(now).saturating_sub(newer.timestamp_ms) as u128;
        let phase_step = (estimated_bpm as u128 * modulus * elapsed_from_newer_ms
            / MS_PER_MINUTE as u128)
            % modulus;
        let estimated_phase = newer.phase.wrapping_add(phase_step as u16);

        Some((estimated_phase, estimated_bpm))
    }

    /// BPM と経過時間から位相ステップとラップ回数を算出する。
    fn phase_step_and_wraps(&self, bpm: u16, dt: Duration) -> (u16, u64) {
        let dt_ms = dt.as_millis();
        let modulus = phase_modulus() as u128;
        let total_advance = bpm as u128 * modulus * dt_ms / MS_PER_MINUTE as u128;

        let step = total_advance % modulus;
        let wraps_from_advance = total_advance / modulus;
        let wraps_from_carry = if self.phase as u128 + step >= modulus {
            1_u128
        } else {
            0_u128
        };
        let wraps = wraps_from_advance
            .saturating_add(wraps_from_carry)
            .min(u64::MAX as u128) as u64;

        (step as u16, wraps)
    }
}

#[inline]
/// 位相差を LUT 参照でサイン値へ変換する。
pub fn sin_from_phase_diff(phase_diff: i64, phase_modulus: u64) -> i16 {
    if phase_modulus == 0 {
        return 0;
    }

    let index = ((phase_diff.rem_euclid(phase_modulus as i64) as u128 * 256)
        / phase_modulus as u128) as usize;
    SIN_LUT[index & 0xFF]
}

#[inline]
fn phase_modulus() -> u64 {
    u16::MAX as u64 + 1
}

#[inline]
fn duration_to_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[inline]
fn clamp_i64(value: i64, min: i64, max: i64) -> i64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use core::time::Duration;

    use super::*;

    #[test]
    // 指定時間更新で位相が正しくラップすることを確認。
    fn update_wraps_phase_after_full_cycles() {
        let mut rhythm = RhythmGenerator::with_bpm(120, 8);
        rhythm.phase = u16::MAX - 5;

        rhythm.update(Duration::from_millis(60_000));

        assert_eq!(rhythm.phase, u16::MAX - 5);
        assert_eq!(rhythm.beat_count, 120);
    }

    #[test]
    // 位相の境界をまたぐ更新で beatcount が増えることを確認。
    fn update_increments_beatcount_on_wrap() {
        let mut rhythm = RhythmGenerator::with_bpm(120, 8);
        rhythm.phase = u16::MAX - 10;

        rhythm.update(Duration::from_millis(100));

        assert_eq!(rhythm.beat_count, 1);
    }

    #[test]
    // メッセージへはジェネレーター保持の beatcount が出力されることを確認。
    fn message_exports_generator_beatcount() {
        let mut rhythm = RhythmGenerator::with_bpm(120, 8);
        rhythm.update(Duration::from_millis(1_500));
        let msg = rhythm.to_message(Duration::from_millis(10_000));

        assert_eq!(msg.beat_count, rhythm.beat_count);
        assert_eq!(msg.beat_count, 3);
    }

    #[test]
    // 同期を繰り返すと位相差が縮み、BPMが外部テンポに近づくことを確認。
    fn sync_reduces_phase_distance_and_tracks_external_tempo() {
        let mut leader = RhythmGenerator::new(0, 125, 0);
        let mut follower = RhythmGenerator::new(28_000, 120, 24);

        let initial_distance = follower.phase_distance(leader.phase);

        for _ in 0..2000 {
            leader.update(Duration::from_millis(10));
            follower.sync(leader.phase);
            follower.update(Duration::from_millis(10));
        }

        let final_distance = follower.phase_distance(leader.phase);
        let bpm_error = follower.current_bpm.abs_diff(125);

        assert!(
            final_distance < initial_distance,
            "phase distance did not shrink"
        );
        assert!(bpm_error <= 2, "follower bpm did not converge near 125");
    }

    #[test]
    // 2メッセージの時間差から推定した値で強制同期できることを確認。
    fn force_sync_from_two_messages_estimates_bpm_and_phase() {
        let msg0 = RhythmMessage {
            phase: 4_000,
            bpm: 130,
            timestamp_ms: 1_000,
            beat_count: 100,
            ..RhythmMessage::default()
        };
        let msg1 = RhythmMessage {
            phase: 4_000,
            bpm: 130,
            timestamp_ms: 1_500,
            beat_count: 101,
            ..RhythmMessage::default()
        };
        let now = Duration::from_millis(1_750);

        let mut follower = RhythmGenerator::new(20_000, 90, 16);
        let synced = follower.force_sync_from_beat_messages(msg0, msg1, now);

        assert!(synced);
        assert_eq!(follower.current_bpm, 120);
        assert_eq!(follower.base_bpm, follower.current_bpm);
        assert_eq!(follower.phase, 36_768);
        assert_eq!(follower.beat_count, 101);
    }

    #[test]
    // 観測時刻差が0なら推定不能として同期を拒否することを確認。
    fn force_sync_rejects_invalid_message_pair() {
        let mut follower = RhythmGenerator::new(10_000, 100, 16);
        let before = follower;

        let msg0 = RhythmMessage {
            phase: 1_000,
            bpm: 120,
            timestamp_ms: 2_000,
            beat_count: 10,
            ..RhythmMessage::default()
        };
        let msg1 = RhythmMessage {
            phase: 2_000,
            bpm: 120,
            timestamp_ms: 2_000,
            beat_count: 10,
            ..RhythmMessage::default()
        };

        let synced =
            follower.force_sync_from_beat_messages(msg0, msg1, Duration::from_millis(2_050));
        assert!(!synced);
        assert_eq!(follower, before);
    }

    #[test]
    fn estimate_from_messages_prefers_observed_40bpm_over_local_120bpm() {
        // 1,500ms で1beat進むサンプル（40bpm相当）。
        let older = RhythmMessage {
            phase: 1_000,
            bpm: 120,
            timestamp_ms: 1_000,
            beat_count: 20,
            ..RhythmMessage::default()
        };
        let newer = RhythmMessage {
            phase: 33_768,
            bpm: 120,
            timestamp_ms: 2_500,
            beat_count: 21,
            ..RhythmMessage::default()
        };

        let now = Duration::from_millis(3_000);
        let (estimated_phase, estimated_bpm) =
            RhythmGenerator::estimate_bpm_phase_from_beat_messages(older, newer, now)
                .expect("valid pair should be estimable");

        assert_eq!(estimated_bpm, 40);
        assert_eq!(estimated_phase, 55_613);

        let mut local = RhythmGenerator::new(10_000, 120, 16);
        assert!(local.force_sync_from_beat_messages(older, newer, now));
        assert_eq!(local.base_bpm, 40);
        assert_eq!(local.current_bpm, 40);
        assert_eq!(local.phase, estimated_phase);
        assert_eq!(local.beat_count, 21);

        // 40bpm では 1,500ms がちょうど1周期なので、位相は同じ値に戻る。
        local.update(Duration::from_millis(1_500));
        assert_eq!(local.phase, estimated_phase);
    }

    #[test]
    // メッセージ往復で値が保たれ、遅延ぶんの位相推定が一致することを確認。
    fn message_roundtrip_and_phase_prediction() {
        let generator = RhythmGenerator::new(1234, 120, 10);
        let msg = generator.to_message(Duration::from_millis(10_000));

        let encoded = serde_json::to_vec(&msg).unwrap();
        let decoded: RhythmMessage = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(msg, decoded);

        let predicted =
            RhythmGenerator::predict_phase_from_message(decoded, Duration::from_millis(10_500));

        let mut expected = RhythmGenerator::new(msg.phase, msg.bpm, 0);
        expected.update(Duration::from_millis(500));

        assert_eq!(predicted, expected.phase);
    }

    #[test]
    // 固定レイアウトのバイト配置と往復変換を確認。
    fn wire_layout_roundtrip() {
        let msg = RhythmMessage {
            phase: 0x1234,
            bpm: 0x5678,
            timestamp_ms: 0x0102_0304_0506_0708,
            beat_count: 0x1112_1314_1516_1718,
            ..RhythmMessage::default()
        };

        let bytes = msg.to_wire_bytes();

        assert_eq!(
            &bytes[RHYTHM_MESSAGE_WIRE_TIMESTAMP_OFFSET..RHYTHM_MESSAGE_WIRE_TIMESTAMP_OFFSET + 8],
            &msg.timestamp_ms.to_le_bytes(),
        );
        assert_eq!(
            &bytes[RHYTHM_MESSAGE_WIRE_CYCLE_COUNT_OFFSET
                ..RHYTHM_MESSAGE_WIRE_CYCLE_COUNT_OFFSET + 8],
            &msg.beat_count.to_le_bytes(),
        );
        assert_eq!(
            &bytes[RHYTHM_MESSAGE_WIRE_PHASE_OFFSET..RHYTHM_MESSAGE_WIRE_PHASE_OFFSET + 2],
            &msg.phase.to_le_bytes(),
        );
        assert_eq!(
            &bytes[RHYTHM_MESSAGE_WIRE_BPM_OFFSET..RHYTHM_MESSAGE_WIRE_BPM_OFFSET + 2],
            &msg.bpm.to_le_bytes(),
        );
        assert_eq!(
            &bytes[RHYTHM_MESSAGE_WIRE_RESERVED_OFFSET
                ..RHYTHM_MESSAGE_WIRE_RESERVED_OFFSET + RHYTHM_MESSAGE_WIRE_RESERVED_SIZE],
            &[0_u8; RHYTHM_MESSAGE_WIRE_RESERVED_SIZE],
        );

        let decoded = RhythmMessage::from_wire_bytes(bytes);
        assert_eq!(decoded, msg);
    }

    #[test]
    // 固定サイズ以外の入力は復元に失敗することを確認。
    fn wire_slice_rejects_invalid_size() {
        assert!(RhythmMessage::from_wire_slice(&[0_u8; 23]).is_none());
        assert!(RhythmMessage::from_wire_slice(&[0_u8; 25]).is_none());
    }

    #[test]
    // 同期入力が止まっても、直前状態で位相更新を継続できることを確認。
    fn rhythm_keeps_running_without_sync_input() {
        let mut rhythm = RhythmGenerator::new(0, 120, 16);
        rhythm.sync(16_384);
        let synced_bpm = rhythm.current_bpm;

        for _ in 0..1000 {
            rhythm.update(Duration::from_millis(5));
        }

        assert_eq!(rhythm.current_bpm, synced_bpm);
    }
}
