#![no_std]

use core::{fmt::Debug, time::Duration};

use num_traits::{ops::wrapping::WrappingAdd, sign::Unsigned, AsPrimitive, Bounded, NumCast};
use serde::{Deserialize, Serialize};

/// 1分あたりのミリ秒。
pub const MS_PER_MINUTE: u32 = 60_000;
/// サインLUTの振幅スケール（i16最大付近）。
pub const LUT_SCALE: i32 = 32_767;

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

/// 位相値として扱える整数型の境界。
pub trait PhaseValue:
    Copy + Default + WrappingAdd + Bounded + NumCast + AsPrimitive<u64> + Unsigned + Debug
{
}

impl<T> PhaseValue for T where
    T: Copy + Default + WrappingAdd + Bounded + NumCast + AsPrimitive<u64> + Unsigned + Debug
{
}

/// BPM 値として扱える整数型の境界。
pub trait TempoValue:
    Copy + Default + Bounded + NumCast + AsPrimitive<u64> + Unsigned + Debug
{
}

impl<T> TempoValue for T where
    T: Copy + Default + Bounded + NumCast + AsPrimitive<u64> + Unsigned + Debug
{
}

/// ネットワーク共有用の最小メッセージ。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhythmMessage<P = u16, T = u16> {
    pub phase: P,
    pub bpm: T,
    pub timestamp_ms: u64,
}

/// 位相生成と外部同期（引き込み）を担当するコア。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhythmGenerator<P = u16, T = u16>
where
    P: PhaseValue,
    T: TempoValue,
{
    pub phase: P,
    pub base_bpm: T,
    pub current_bpm: T,
    pub k: T,
}

pub type Rhythm = RhythmGenerator<u16, u16>;

impl<P, T> RhythmGenerator<P, T>
where
    P: PhaseValue,
    T: TempoValue,
{
    /// 初期位相・基準BPM・同期強度を指定して生成する。
    pub fn new(phase: P, base_bpm: T, k: T) -> Self {
        Self {
            phase,
            base_bpm,
            current_bpm: base_bpm,
            k,
        }
    }

    /// 位相0で生成する簡易コンストラクタ。
    pub fn with_bpm(base_bpm: T, k: T) -> Self {
        Self::new(P::default(), base_bpm, k)
    }

    /// 現在BPMに従って位相を進める（ラップアラウンド込み）。
    pub fn update(&mut self, dt: Duration) {
        let step = self.phase_step_from(self.current_bpm, dt);
        self.phase = self.phase.wrapping_add(&step);
    }

    /// 外部位相との差から蔵本モデル風に BPM を補正する。
    pub fn sync(&mut self, target_phase: P) {
        let diff = self.phase_error(target_phase);
        let sin = sin_from_phase_diff(diff, phase_modulus::<P>());

        let base_bpm = self.base_bpm.as_() as i64;
        let k = self.k.as_() as i64;
        let correction = (k * sin as i64) / LUT_SCALE as i64;

        let next_bpm = clamp_i64(base_bpm + correction, 0, T::max_value().as_() as i64);
        self.current_bpm = cast_from_u64(next_bpm as u64);
    }

    /// 位相差を最短距離（±半周以内）で返す。
    pub fn phase_error(&self, target_phase: P) -> i64 {
        let modulus = phase_modulus::<P>() as i64;
        let target = target_phase.as_() as i64;
        let current = self.phase.as_() as i64;

        let mut diff = (target - current).rem_euclid(modulus);
        if diff > modulus / 2 {
            diff -= modulus;
        }
        diff
    }

    /// 位相差の絶対値。
    pub fn phase_distance(&self, target_phase: P) -> u64 {
        self.phase_error(target_phase).unsigned_abs()
    }

    /// 現在状態を送信用メッセージに変換する。
    pub fn to_message(&self, timestamp: Duration) -> RhythmMessage<P, T> {
        RhythmMessage {
            phase: self.phase,
            bpm: self.current_bpm,
            timestamp_ms: duration_to_millis_u64(timestamp),
        }
    }

    /// 受信時刻を使って、受信メッセージの位相を現在時刻へ外挿する。
    pub fn predict_phase_from_message(message: RhythmMessage<P, T>, now: Duration) -> P {
        let base = Duration::from_millis(message.timestamp_ms);
        let elapsed_ms = now.saturating_sub(base).as_millis();
        let modulus = phase_modulus::<P>() as u128;
        let step =
            (message.bpm.as_() as u128 * modulus * elapsed_ms / MS_PER_MINUTE as u128) % modulus;
        let step_phase: P = cast_from_u64(step as u64);
        message.phase.wrapping_add(&step_phase)
    }

    /// BPM と経過時間から位相ステップを整数演算で算出する。
    fn phase_step_from(&self, bpm: T, dt: Duration) -> P {
        let dt_ms = dt.as_millis();
        let modulus = phase_modulus::<P>() as u128;
        let step = (bpm.as_() as u128 * modulus * dt_ms / MS_PER_MINUTE as u128) % modulus;
        cast_from_u64(step as u64)
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
fn phase_modulus<P>() -> u64
where
    P: PhaseValue,
{
    P::max_value().as_() + 1
}

#[inline]
fn cast_from_u64<T>(value: u64) -> T
where
    T: NumCast,
{
    NumCast::from(value).expect("value is in range of target integer type")
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
        let mut rhythm = RhythmGenerator::<u16, u16>::with_bpm(120, 8);
        rhythm.phase = u16::MAX - 5;

        rhythm.update(Duration::from_millis(60_000));

        assert_eq!(rhythm.phase, u16::MAX - 5);
    }

    #[test]
    // 同期を繰り返すと位相差が縮み、BPMが外部テンポに近づくことを確認。
    fn sync_reduces_phase_distance_and_tracks_external_tempo() {
        let mut leader = RhythmGenerator::<u16, u16>::new(0, 125, 0);
        let mut follower = RhythmGenerator::<u16, u16>::new(28_000, 120, 24);

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
    // メッセージ往復で値が保たれ、遅延ぶんの位相推定が一致することを確認。
    fn message_roundtrip_and_phase_prediction() {
        let generator = RhythmGenerator::<u16, u16>::new(1234, 120, 10);
        let msg = generator.to_message(Duration::from_millis(10_000));

        let encoded = serde_json::to_vec(&msg).unwrap();
        let decoded: RhythmMessage<u16, u16> = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(msg, decoded);

        let predicted = RhythmGenerator::<u16, u16>::predict_phase_from_message(
            decoded,
            Duration::from_millis(10_500),
        );

        let mut expected = RhythmGenerator::<u16, u16>::new(msg.phase, msg.bpm, 0);
        expected.update(Duration::from_millis(500));

        assert_eq!(predicted, expected.phase);
    }

    #[test]
    // 同期入力が止まっても、直前状態で位相更新を継続できることを確認。
    fn rhythm_keeps_running_without_sync_input() {
        let mut rhythm = RhythmGenerator::<u16, u16>::new(0, 120, 16);
        rhythm.sync(16_384);
        let synced_bpm = rhythm.current_bpm;

        for _ in 0..1000 {
            rhythm.update(Duration::from_millis(5));
        }

        assert_eq!(rhythm.current_bpm, synced_bpm);
    }
}
