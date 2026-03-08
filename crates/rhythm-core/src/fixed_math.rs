//! リズム計算用固定小数点演算プリミティブ。
//!
//! | 型           | 内部型  | エンコーディング       | 有効範囲        |
//! |--------------|---------|-----------------------|----------------|
//! | [`BpmQ8`]    | `u16`   | Q8.8 (1.0 BPM = 256) | 60 – 120 BPM   |
//! | [`PhaseU16`] | `u16`   | 線形 0–65535 ≡ 2π    | ラップ算術      |
//! | [`SinQ15`]   | `i16`   | Q1.15 (1.0 ≈ 32767)  | [-1.0, 1.0]    |

use serde::{Deserialize, Serialize};

use crate::consts::{BPM_Q8_ONE, BPM_Q8_SHIFT, MS_PER_MINUTE, PHASE_MODULUS, SIN_LUT};

// ─────────────────────────────────────────
//  BpmQ8
// ─────────────────────────────────────────

/// Q8.8 固定小数点形式の BPM。
///
/// ```text
/// raw = BPM × 256
///  60 BPM →  15360
///  90 BPM →  23040
/// 120 BPM →  30720
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct BpmQ8(pub u16);

impl BpmQ8 {
    /// 60.0 BPM（Q8.8）— 有効範囲の下限。
    pub const MIN: Self = Self(60 * BPM_Q8_ONE);
    /// 120.0 BPM（Q8.8）— 有効範囲の上限。
    pub const MAX: Self = Self(120 * BPM_Q8_ONE);

    /// 整数 BPM 値から生成する。
    #[inline]
    pub const fn from_int(bpm: u16) -> Self {
        Self(bpm.saturating_mul(BPM_Q8_ONE))
    }

    /// BPM の整数部（切り捨て）。
    #[inline]
    pub const fn to_int_floor(self) -> u16 {
        self.0 >> BPM_Q8_SHIFT
    }

    /// BPM の整数部（四捨五入）。
    #[inline]
    pub fn to_int_round(self) -> u16 {
        ((self.0 as u32 + BPM_Q8_ONE as u32 / 2) >> BPM_Q8_SHIFT) as u16
    }

    /// ビート間隔（ミリ秒）から Q8.8 BPM を算出する。
    ///
    /// `interval_ms` が 0 の場合は飽和最大値を返す。
    pub fn from_interval_ms(interval_ms: u32) -> Self {
        if interval_ms == 0 {
            return Self(u16::MAX);
        }
        let numerator = MS_PER_MINUTE as u64 * BPM_Q8_ONE as u64;
        let raw = ((numerator + interval_ms as u64 / 2) / interval_ms as u64)
            .clamp(BPM_Q8_ONE as u64, u16::MAX as u64) as u16;
        Self(raw)
    }

    /// 有効 BPM 範囲 [`MIN`..`MAX`] にクランプする。
    #[inline]
    pub fn clamp_to_range(self) -> Self {
        Self(self.0.clamp(Self::MIN.0, Self::MAX.0))
    }

    /// 呼び出しごとに `self` から `target` へ `1/divisor` ずつ滑らかに近づける。
    ///
    /// 結果は [`MIN`..`MAX`] にクランプされる。
    /// `divisor ≤ 1` の場合は `target` を即時返す。
    pub fn blend_toward(self, target: Self, divisor: i32) -> Self {
        if divisor <= 1 {
            return target;
        }
        let diff = target.0 as i32 - self.0 as i32;
        if diff == 0 {
            return self;
        }
        let mut step = diff / divisor;
        if step == 0 {
            step = diff.signum();
        }
        let raw = (self.0 as i32 + step).clamp(Self::MIN.0 as i32, Self::MAX.0 as i32) as u16;
        Self(raw)
    }

    /// Q8.8 生値を返す。
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

// ─────────────────────────────────────────
//  PhaseU16
// ─────────────────────────────────────────

/// u16 ラップ算術で [0, 2π) を表す位相。範囲は [0, 65535]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct PhaseU16(pub u16);

impl PhaseU16 {
    /// 1/4 周期 — π/2 (90°)。
    pub const QUARTER: Self = Self(16384);
    /// 半周期 — π (180°)。
    pub const HALF: Self = Self(32768);
    /// 3/4 周期 — 3π/2 (270°)。
    pub const THREE_QUARTER: Self = Self(49152);

    /// Q8.8 形式の `bpm` で `dt_ms` ミリ秒分の位相増分を計算する。
    ///
    /// 結果は 65536 でラップするため、1 周回ると `PhaseU16(0)` になる。
    /// 丸め誤差なしに使うには、ビート周期の整数倍の `dt_ms`（例: 120 BPM なら 500 ms）を選ぶ。
    /// ジェネレータ用には幅広アキュムレータを持つ [`phase_advance_u64`] を使うこと。
    #[inline]
    pub fn advance(bpm: BpmQ8, dt_ms: u32) -> Self {
        Self(phase_advance_u16(bpm.0, dt_ms))
    }

    /// ラップ加算。
    #[inline]
    pub fn wrapping_add(self, delta: Self) -> Self {
        Self(self.0.wrapping_add(delta.0))
    }

    /// 生の `u16` 増分によるラップ加算。
    #[inline]
    pub fn wrapping_add_raw(self, delta: u16) -> Self {
        Self(self.0.wrapping_add(delta))
    }

    /// SIN_LUT から正弦値を引いて [`SinQ15`] で返す。
    ///
    /// 位相の上位 8 ビットをインデックスとして使用する。
    #[inline]
    pub fn sin(self) -> SinQ15 {
        SinQ15(SIN_LUT[(self.0 >> 8) as usize])
    }

    /// `self − other` の符号付き位相差（[−32768, 32767]）。
    /// 円周上の最短経路を選択する。
    #[inline]
    pub fn signed_diff(self, other: Self) -> i16 {
        self.0.wrapping_sub(other.0) as i16
    }

    /// 生の `u16` 値を返す。
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

// ─────────────────────────────────────────
//  SinQ15
// ─────────────────────────────────────────

/// Q1.15 固定小数点形式の正弦値。
///
/// ```text
///  1.0 ≈  32767
///  0.0 =      0
/// -1.0 = -32768  (理論値; LUT の最小値は -32767)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct SinQ15(pub i16);

impl SinQ15 {
    /// テスト比較用に `[-1.0, 1.0]` の `f32` へ変換する。
    #[inline]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / 32767.0
    }

    /// 生の `i16` 値を返す。
    #[inline]
    pub const fn raw(self) -> i16 {
        self.0
    }
}

// ─────────────────────────────────────────
//  内部ヘルパー
// ─────────────────────────────────────────

/// 1 ステップ分の位相増分を `u16` に切り捨てて返す。
///
/// [`PhaseU16::advance`] から呼ばれる。整数切り捨てがあるため、
/// 長時間動かすアキュムレータには [`phase_advance_u64`] を使うこと。
#[inline]
pub(crate) fn phase_advance_u16(bpm_q8: u16, dt_ms: u32) -> u16 {
    let numerator = bpm_q8 as u128 * PHASE_MODULUS as u128 * dt_ms as u128;
    let denominator = MS_PER_MINUTE as u128 * BPM_Q8_ONE as u128;
    (numerator / denominator) as u16
}

/// 幅広アキュムレータ用の位相増分を `u64` で返す。
///
/// 呼び出し側は `u64` の累積値を保持し、現在の位相を
/// `(total % PHASE_MODULUS as u64) as u16` で取得する。
/// これにより長時間動作での切り捨てドリフトを回避できる。
///
/// **注意**: `dt_ms` がビート周期より短い場合でも 1 ステップあたり
/// 最大 1 単位の切り捨てが発生する。細かいステップ（< 100 ms）では
/// 下位 16 ビットに小数残差を保持する [`phase_advance_sub16`] を使うこと。
#[inline]
pub(crate) fn phase_advance_u64(bpm_q8: u16, dt_ms: u32) -> u64 {
    let numerator = bpm_q8 as u128 * PHASE_MODULUS as u128 * dt_ms as u128;
    let denominator = MS_PER_MINUTE as u128 * BPM_Q8_ONE as u128;
    (numerator / denominator).min(u64::MAX as u128) as u64
}

/// 高精度 `u64` アキュムレータ向けのサブ位相増分を返す。
///
/// 1 周回 = 2³² サブ位相単位。現在の `u16` 位相は
/// `(accum >> 16) as u16` で取得する。
/// アキュムレータの下位 16 ビットが小数残差を保持するため、
/// 非常に小さな `dt_ms` でも長期ドリフトをほぼゼロにできる。
///
/// # 使用例
/// ```ignore
/// let mut accum: u64 = 0;
/// let delta = phase_advance_sub16(bpm.raw(), dt_ms);
/// accum = accum.wrapping_add(delta);
/// let phase_u16: u16 = (accum >> 16) as u16;
/// ```
#[inline]
pub(crate) fn phase_advance_sub16(bpm_q8: u16, dt_ms: u32) -> u64 {
    // PHASE_MODULUS を 2^16 倍して下位 16 ビットを小数残差として使う。
    let pm_sub: u128 = (PHASE_MODULUS as u128) << 16;
    let numerator = bpm_q8 as u128 * pm_sub * dt_ms as u128;
    let denominator = MS_PER_MINUTE as u128 * BPM_Q8_ONE as u128;
    (numerator / denominator).min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    // ── BpmQ8: 正常系 ─ 整数 BPM ↔ Q8.8 変換 ──────────────────────────────

    /// 整数 BPM から生成した Q8.8 値が正しい raw 値になること、
    /// および floor / round で元の整数へ戻ること。
    #[test]
    fn bpm_q8_int_conversion() {
        struct Case {
            bpm: u16,
            expected_raw: u16,
        }
        let cases = [
            Case {
                bpm: 60,
                expected_raw: 15360,
            },
            Case {
                bpm: 90,
                expected_raw: 23040,
            },
            Case {
                bpm: 120,
                expected_raw: 30720,
            },
        ];
        for c in &cases {
            let q8 = BpmQ8::from_int(c.bpm);
            assert_eq!(q8.raw(), c.expected_raw, "BPM={} raw", c.bpm);
            assert_eq!(q8.to_int_floor(), c.bpm, "BPM={} floor", c.bpm);
            assert_eq!(q8.to_int_round(), c.bpm, "BPM={} round", c.bpm);
        }
    }

    // ── BpmQ8: 正常系 ─ ビート間隔(ms) → BPM 変換 ──────────────────────────

    /// ビート間隔ミリ秒から BPM を逆算したとき、丸め込みを含めて正しい整数 BPM になること。
    #[test]
    fn bpm_q8_from_interval_ms() {
        struct Case {
            label: &'static str,
            interval_ms: u32,
            expected_bpm: u16,
        }
        let cases = [
            Case {
                label: "120 BPM = 500 ms",
                interval_ms: 500,
                expected_bpm: 120,
            },
            Case {
                label: "60 BPM = 1000 ms",
                interval_ms: 1000,
                expected_bpm: 60,
            },
            Case {
                label: "90 BPM ≈ 667 ms",
                interval_ms: 667,
                expected_bpm: 90,
            },
        ];
        for c in &cases {
            let q8 = BpmQ8::from_interval_ms(c.interval_ms);
            assert_eq!(
                q8.to_int_round(),
                c.expected_bpm,
                "[{}] interval_ms={} → BPM",
                c.label,
                c.interval_ms,
            );
        }
    }

    // ── BpmQ8: 値域確認 ─ clamp_to_range ───────────────────────────────────

    /// clamp_to_range が MIN/MAX の外側および内側の値を正しく扱うこと。
    #[test]
    fn bpm_q8_clamp() {
        struct Case {
            label: &'static str,
            input: BpmQ8,
            expected: BpmQ8,
        }
        let cases = [
            Case {
                label: "0 → MIN",
                input: BpmQ8(0),
                expected: BpmQ8::MIN,
            },
            Case {
                label: "MAX_U16 → MAX",
                input: BpmQ8(u16::MAX),
                expected: BpmQ8::MAX,
            },
            Case {
                label: "90 BPM → 変化なし",
                input: BpmQ8::from_int(90),
                expected: BpmQ8::from_int(90),
            },
            Case {
                label: "MIN そのまま",
                input: BpmQ8::MIN,
                expected: BpmQ8::MIN,
            },
            Case {
                label: "MAX そのまま",
                input: BpmQ8::MAX,
                expected: BpmQ8::MAX,
            },
        ];
        for c in &cases {
            assert_eq!(c.input.clamp_to_range(), c.expected, "[{}]", c.label,);
        }
    }

    // ── BpmQ8: 正常系 / 値域確認 ─ blend_toward ─────────────────────────────

    /// blend_toward が 1/divisor ずつ目標へ近づくこと、
    /// および結果が MIN/MAX を超えないこと。
    #[test]
    fn bpm_q8_blend_toward() {
        struct Case {
            label: &'static str,
            start: BpmQ8,
            target: BpmQ8,
            divisor: i32,
            expected: BpmQ8,
        }
        let cases = [
            Case {
                label: "60→120 by 1/10",
                start: BpmQ8::from_int(60),
                target: BpmQ8::from_int(120),
                divisor: 10,
                expected: BpmQ8(
                    BpmQ8::from_int(60).0 + (BpmQ8::from_int(120).0 - BpmQ8::from_int(60).0) / 10,
                ),
            },
            Case {
                label: "divisor=1 → target を即時返す",
                start: BpmQ8::from_int(60),
                target: BpmQ8::from_int(120),
                divisor: 1,
                expected: BpmQ8::from_int(120),
            },
            Case {
                label: "MAX を超える目標 → MAX に止まる",
                start: BpmQ8::MAX,
                target: BpmQ8(u16::MAX),
                divisor: 2,
                expected: BpmQ8::MAX,
            },
            Case {
                label: "MIN を下回る目標 → MIN に止まる",
                start: BpmQ8::MIN,
                target: BpmQ8(0),
                divisor: 2,
                expected: BpmQ8::MIN,
            },
        ];
        for c in &cases {
            assert_eq!(
                c.start.blend_toward(c.target, c.divisor),
                c.expected,
                "[{}]",
                c.label,
            );
        }
    }

    // ── PhaseU16: 正常系 ─ advance（位相増分） ──────────────────────────────

    /// 指定 BPM / 経過時間から計算した位相増分が期待値と一致すること。
    #[test]
    fn phase_u16_advance() {
        struct Case {
            label: &'static str,
            bpm: u16,
            dt_ms: u32,
            expected: u16,
        }
        let cases = [
            Case {
                label: "120 BPM 500 ms = 1 周",
                bpm: 120,
                dt_ms: 500,
                expected: 0,
            },
            Case {
                label: "90 BPM 2000 ms = 3 周",
                bpm: 90,
                dt_ms: 2000,
                expected: 0,
            },
            Case {
                label: "60 BPM 500 ms = 半周",
                bpm: 60,
                dt_ms: 500,
                expected: 32768,
            },
        ];
        for c in &cases {
            let delta = PhaseU16::advance(BpmQ8::from_int(c.bpm), c.dt_ms);
            assert_eq!(delta.0, c.expected, "[{}]", c.label);
        }
    }

    // ── PhaseU16: 正常系 ─ signed_diff（符号付き位相差） ────────────────────

    /// signed_diff が円周上の最短経路を正しく返すこと。
    #[test]
    fn phase_u16_signed_diff() {
        struct Case {
            label: &'static str,
            a: u16,
            b: u16,
            expected: i16,
        }
        let cases = [
            Case {
                label: "正方向",
                a: 1000,
                b: 500,
                expected: 500,
            },
            Case {
                label: "負方向",
                a: 500,
                b: 1000,
                expected: -500,
            },
            // a=100, b=65336: 前方 300 ステップが後方 65236 より短い
            Case {
                label: "ラップして最短経路（正）",
                a: 100,
                b: 65336,
                expected: 300,
            },
        ];
        for c in &cases {
            assert_eq!(
                PhaseU16(c.a).signed_diff(PhaseU16(c.b)),
                c.expected,
                "[{}]",
                c.label,
            );
        }
    }

    // ── PhaseU16: 異常系 ─ wrapping_add オーバーフロー ──────────────────────

    /// wrapping_add が u16 の上限を超えたとき正しくラップすること。
    #[test]
    fn phase_u16_wrapping_add_overflow() {
        struct Case {
            label: &'static str,
            base: u16,
            delta: u16,
            expected: u16,
        }
        let cases = [
            Case {
                label: "0xFFFE + 2 → 0",
                base: 0xFFFE,
                delta: 2,
                expected: 0,
            },
            Case {
                label: "0xFFFF + 1 → 0",
                base: 0xFFFF,
                delta: 1,
                expected: 0,
            },
            Case {
                label: "0 + 0 → 0",
                base: 0,
                delta: 0,
                expected: 0,
            },
            Case {
                label: "0 + 65535 → 65535",
                base: 0,
                delta: 65535,
                expected: 65535,
            },
        ];
        for c in &cases {
            assert_eq!(
                PhaseU16(c.base).wrapping_add(PhaseU16(c.delta)).0,
                c.expected,
                "[{}]",
                c.label,
            );
        }
    }

    // ── SinQ15 / LUT: 正常系 ─ 主要角度のサイン値 ────────────────────────────

    /// sin LUT の主要サンプル点（0, π/2, π, 3π/2）が正しい raw 値を返すこと。
    #[test]
    fn sin_lut_key_values() {
        struct Case {
            label: &'static str,
            phase: u16,
            expected_raw: i16,
        }
        let cases = [
            Case {
                label: "sin 0 = 0",
                phase: 0,
                expected_raw: 0,
            },
            Case {
                label: "sin π/2 ≈ 1.0",
                phase: 16384,
                expected_raw: 32767,
            },
            Case {
                label: "sin π = 0",
                phase: 32768,
                expected_raw: 0,
            },
            Case {
                label: "sin 3π/2 ≈ -1.0",
                phase: 49152,
                expected_raw: -32767,
            },
        ];
        for c in &cases {
            assert_eq!(
                PhaseU16(c.phase).sin().raw(),
                c.expected_raw,
                "[{}]",
                c.label,
            );
        }
    }

    // ── SinQ15 / LUT: 精度確認 ─ 全エントリ誤差 ─────────────────────────────

    /// 全 256 エントリで f64 sin との相対誤差が 0.2% 未満であること。
    #[test]
    fn sin_lut_accuracy_full_range() {
        for i in 0u16..256 {
            let phase = PhaseU16(i << 8);
            let expected = ((i as f64 / 256.0) * 2.0 * PI).sin();
            let got = phase.sin().to_f32() as f64;
            assert!(
                (got - expected).abs() < 0.002,
                "LUT[{i}]: got={got:.5}, expected={expected:.5}, err={:.6}",
                (got - expected).abs(),
            );
        }
    }

    // ── SinQ15 / LUT: 値域確認 ─ f32 変換後の範囲 ───────────────────────────

    /// to_f32 の返り値が常に [-1.0, 1.0] の範囲に収まること。
    #[test]
    fn sin_to_f32_stays_in_range() {
        for i in 0u16..256 {
            let v = PhaseU16(i << 8).sin().to_f32();
            assert!(
                (-1.0001..=1.0001).contains(&v),
                "SinQ15 out of range at i={i}: {v}"
            );
        }
    }

    // ── phase_advance_sub16: 長期安定性 ─ ドリフト確認 ──────────────────────

    /// サブ位相アキュムレータで長時間動かしてもドリフトが 10 単位以内に収まること。
    #[test]
    fn phase_advance_sub16_no_drift() {
        struct Case {
            label: &'static str,
            bpm: u16,
            dt_ms: u32,
            steps: u32,
        }
        let cases = [
            Case {
                label: "120 BPM 10ms×6000 = 60s",
                bpm: 120,
                dt_ms: 10,
                steps: 6000,
            },
            Case {
                label: "90 BPM 10ms×6000 = 60s",
                bpm: 90,
                dt_ms: 10,
                steps: 6000,
            },
        ];
        for c in &cases {
            let bpm_q8 = BpmQ8::from_int(c.bpm).raw();
            let mut accum: u64 = 0;
            for _ in 0..c.steps {
                accum = accum.wrapping_add(phase_advance_sub16(bpm_q8, c.dt_ms));
            }
            let phase_u16 = (accum >> 16) as u16;
            assert!(
                phase_u16 < 10 || phase_u16 > 65526,
                "[{}] ドリフト発生: phase_u16={phase_u16}",
                c.label,
            );
        }
    }
}
