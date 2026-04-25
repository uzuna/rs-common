//! 信号データ生成モジュール。
//!
//! 6 モードの 2ch 波形（A チャンネル, B チャンネル）と未知モード信号を生成する。

use rand::distr::{Distribution, Uniform};
use rand::RngExt;

use crate::{FREQ, FS, N_MODES};

/// 波形種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    Sin,
    Sawtooth,
    Square,
}

/// 各モードの (A チャンネル波形, B チャンネル波形) ペア
pub const MODE_PAIRS: [(Wave, Wave); N_MODES] = [
    (Wave::Sin, Wave::Sin),
    (Wave::Sin, Wave::Sawtooth),
    (Wave::Sin, Wave::Square),
    (Wave::Sawtooth, Wave::Sawtooth),
    (Wave::Sawtooth, Wave::Square),
    (Wave::Square, Wave::Square),
];

/// モード名（表示用）
pub const MODE_NAMES: [&str; N_MODES] = [
    "sin-sin", "sin-saw", "sin-squ", "saw-saw", "saw-squ", "squ-squ",
];

/// 1 サンプルの波形値を計算する。
/// t: サンプルインデックス（0 始まり）
fn sample_wave(wave: Wave, t: usize, amp: f64) -> f64 {
    let phase = FREQ * t as f64 / FS; // 周期 [cycle]
    match wave {
        Wave::Sin => amp * (2.0 * std::f64::consts::PI * phase).sin(),
        Wave::Sawtooth => amp * (2.0 * phase.rem_euclid(1.0) - 1.0),
        Wave::Square => amp * if phase.rem_euclid(1.0) < 0.5 { 1.0 } else { -1.0 },
    }
}

/// Box-Muller 変換によるガウシアンサンプル生成。
/// noise_std=0 の場合は 0 を返す。
fn sample_gaussian(noise_std: f64, rng: &mut impl RngExt) -> f64 {
    if noise_std <= 0.0 {
        return 0.0;
    }
    // rand 0.10 の random::<f64>() は [0, 1) の一様分布
    let u1: f64 = (rng.random::<f64>()).max(f64::MIN_POSITIVE);
    let u2: f64 = rng.random::<f64>();
    let r: f64 = (-2.0_f64 * u1.ln()).sqrt();
    let theta = 2.0 * std::f64::consts::PI * u2;
    r * theta.cos() * noise_std
}

/// 1 モードの n ステップの 2ch 信号を生成する。
///
/// 戻り値: `Vec<f64>` len = `n * 2`、レイアウト `[A0, B0, A1, B1, ...]`
pub fn generate_segment(
    mode: usize,
    n: usize,
    noise_std: f64,
    rng: &mut impl RngExt,
) -> Vec<f64> {
    assert!(mode < N_MODES, "モードインデックス {mode} が範囲外 (0..{N_MODES})");
    let (wave_a, wave_b) = MODE_PAIRS[mode];
    let amp = 1.0_f64;
    let mut out = Vec::with_capacity(n * 2);
    for t in 0..n {
        out.push(sample_wave(wave_a, t, amp) + sample_gaussian(noise_std, rng));
        out.push(sample_wave(wave_b, t, amp) + sample_gaussian(noise_std, rng));
    }
    out
}

/// 未知モード: A=sin 正常、B=ノイズのみ（周期信号なし）
///
/// 戻り値: `Vec<f64>` len = `n * 2`、レイアウト `[A0, B0, A1, B1, ...]`
pub fn generate_unknown(n: usize, noise_std: f64, rng: &mut impl RngExt) -> Vec<f64> {
    let amp = 1.0_f64;
    let b_noise_std = noise_std.max(0.3); // 未知モードの B チャンネルは常に高ノイズ
    let mut out = Vec::with_capacity(n * 2);
    for t in 0..n {
        let a = sample_wave(Wave::Sin, t, amp) + sample_gaussian(noise_std, rng);
        let b = sample_gaussian(b_noise_std, rng); // 信号成分なし
        out.push(a);
        out.push(b);
    }
    out
}

/// テスト用: 全 6 モードをシャッフルして連結した系列を生成する。
///
/// 戻り値:
/// - `data`: `Vec<f64>` len = `steps_per_mode * N_MODES * 2`（インターリーブ 2ch）
/// - `true_labels`: `Vec<usize>` len = `steps_per_mode * N_MODES`（各ステップのモードラベル）
/// - `segments`: `Vec<(usize, usize, usize)>`（mode_idx, start_step, end_step）
pub fn generate_test_sequence(
    steps_per_mode: usize,
    noise_std: f64,
    rng: &mut impl RngExt,
) -> (Vec<f64>, Vec<usize>, Vec<(usize, usize, usize)>) {
    // Fisher-Yates シャッフル
    let mut order: Vec<usize> = (0..N_MODES).collect();
    for i in (1..N_MODES).rev() {
        let j = Uniform::new(0_usize, i + 1).unwrap().sample(rng);
        order.swap(i, j);
    }

    let total_steps = steps_per_mode * N_MODES;
    let mut data = Vec::with_capacity(total_steps * 2);
    let mut labels = Vec::with_capacity(total_steps);
    let mut segments = Vec::with_capacity(N_MODES);

    let mut pos = 0;
    for &mode_idx in &order {
        let seg = generate_segment(mode_idx, steps_per_mode, noise_std, rng);
        data.extend_from_slice(&seg);
        labels.extend(std::iter::repeat(mode_idx).take(steps_per_mode));
        segments.push((mode_idx, pos, pos + steps_per_mode));
        pos += steps_per_mode;
    }

    (data, labels, segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    #[test]
    fn test_segment_length() {
        let mut rng = SmallRng::seed_from_u64(0);
        for mode in 0..N_MODES {
            let seg = generate_segment(mode, 100, 0.0, &mut rng);
            assert_eq!(seg.len(), 200, "モード {mode} の len が 200 でない");
        }
    }

    #[test]
    fn test_sin_range() {
        let mut rng = SmallRng::seed_from_u64(0);
        // noise なしの sin-sin は [-1, 1] に収まる
        let seg = generate_segment(0, 1000, 0.0, &mut rng);
        for &v in &seg {
            assert!(v.abs() <= 1.0 + 1e-9, "sin-sin 値 {v} が範囲外");
        }
    }

    #[test]
    fn test_square_values() {
        let mut rng = SmallRng::seed_from_u64(0);
        // noise なしの squ-squ は ±1 のみ
        let seg = generate_segment(5, 300, 0.0, &mut rng);
        for &v in &seg {
            assert!((v.abs() - 1.0).abs() < 1e-9, "squ-squ 値 {v} が ±1 でない");
        }
    }

    #[test]
    fn test_unknown_b_has_no_signal() {
        let mut rng = SmallRng::seed_from_u64(0);
        let seg = generate_unknown(1000, 0.01, &mut rng);
        // B チャンネルの振幅が信号振幅 1.0 より遙かに小さいことを確認
        let b_vals: Vec<f64> = seg.iter().skip(1).step_by(2).copied().collect();
        let b_max = b_vals.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        assert!(b_max < 1.0, "未知モードの B チャンネルが振幅 1.0 を超えた: {b_max}");
    }

    #[test]
    fn test_test_sequence() {
        let mut rng = SmallRng::seed_from_u64(1);
        let (data, labels, segs) = generate_test_sequence(100, 0.01, &mut rng);
        assert_eq!(data.len(), N_MODES * 100 * 2);
        assert_eq!(labels.len(), N_MODES * 100);
        assert_eq!(segs.len(), N_MODES);
        // 全モードが 1 回ずつ現れる
        let mut seen = [false; N_MODES];
        for &(m, _, _) in &segs {
            seen[m] = true;
        }
        assert!(seen.iter().all(|&b| b), "一部のモードがテスト系列に含まれていない");
    }
}
