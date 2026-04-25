//! 異常検知・スペクトル分析モジュール。
//!
//! - 因果的移動平均平滑化
//! - スライディング窓スペクトルピーク比（未知モード検出用）
//! - パーセンタイル閾値計算

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// 因果的移動平均による平滑化（累積和ベース、未来情報を使わない）。
///
/// 各時刻 t の値は `errors[max(0, t-window+1)..=t]` の平均。
pub fn smooth(errors: &[f64], window: usize) -> Vec<f64> {
    if window <= 1 || errors.is_empty() {
        return errors.to_vec();
    }
    let n = errors.len();
    let mut cs = Vec::with_capacity(n + 1);
    cs.push(0.0_f64);
    for &e in errors {
        cs.push(cs.last().unwrap() + e);
    }
    (0..n)
        .map(|i| {
            let end = i + 1;
            let start = end.saturating_sub(window);
            let count = (end - start) as f64;
            (cs[end] - cs[start]) / count
        })
        .collect()
}

/// 因果的スライディング窓 FFT によるスペクトルピーク比。
///
/// 各時刻 t に対して、直前 `window` サンプルの FFT を計算し、
/// `target_freq` ビンのパワー / 全パワー（rfft 範囲）の比を返す。
///
/// - 先頭 `window` 要素は FFT 窓が満杯でないため `0.0` を返す。
/// - real 入力のため conjugate 対称: `buf[0..=window/2]` の `window/2+1` ビンのみ集計。
pub fn spectral_peak_ratio(sig: &[f64], target_freq: f64, fs: f64, window: usize) -> Vec<f64> {
    let n = sig.len();
    let freq_bin = (target_freq * window as f64 / fs).round() as usize;
    let nyquist_bin = window / 2; // rfft の最大ビン (inclusive)

    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(window);

    let mut ratio = vec![0.0_f64; n];

    // t=window..n-1 に対して frames[t-window..t] を処理
    // sliding_window_view の Python 実装に対応:
    // Python では `frames[i] = sig[i..i+window]`, `ratio[t] = ratio_from_frames[t-window]`
    // → t >= window で `sig[t-window..t]` を使う
    let mut buf: Vec<Complex<f64>> = vec![Complex::new(0.0, 0.0); window];

    for t in window..n {
        // 窓: sig[t-window..t]
        for (j, &v) in sig[t - window..t].iter().enumerate() {
            buf[j] = Complex::new(v, 0.0);
        }
        fft.process(&mut buf);

        // rfft 相当: bins 0..=nyquist_bin のみ集計
        let total_power: f64 = buf[0..=nyquist_bin].iter().map(|c| c.norm_sqr()).sum();
        let peak_power = buf[freq_bin].norm_sqr();
        ratio[t] = peak_power / (total_power + 1e-12);
    }

    ratio
}

/// ソートによるパーセンタイル計算（in-place ソート）。
///
/// `p`: 0.0〜100.0
pub fn percentile(values: &mut [f64], p: f64) -> f64 {
    assert!(!values.is_empty(), "パーセンタイル計算に空のスライスが渡された");
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((p / 100.0) * (values.len() - 1) as f64).round() as usize;
    values[idx.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smooth_basic() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let s = smooth(&data, 3);
        assert_eq!(s.len(), 5);
        // t=0: [1]/1=1, t=1: [1,2]/2=1.5, t=2: [1,2,3]/3=2, t=3: [2,3,4]/3=3
        assert!((s[0] - 1.0).abs() < 1e-9);
        assert!((s[1] - 1.5).abs() < 1e-9);
        assert!((s[2] - 2.0).abs() < 1e-9);
        assert!((s[3] - 3.0).abs() < 1e-9);
        assert!((s[4] - 4.0).abs() < 1e-9);
    }

    #[test]
    fn test_smooth_causal() {
        // 将来情報を使っていないことの確認: t=0 の値は errors[0] のみに依存
        let data = vec![10.0, 0.0, 0.0, 0.0];
        let s = smooth(&data, 4);
        assert!((s[0] - 10.0).abs() < 1e-9, "t=0 は errors[0] のみ: {}", s[0]);
    }

    #[test]
    fn test_spectral_peak_ratio_sin() {
        // 2Hz sin, FS=30, window=60 → freq_bin=4
        let fs = 30.0_f64;
        let freq = 2.0_f64;
        let n = 200_usize;
        let sig: Vec<f64> = (0..n)
            .map(|t| (2.0 * std::f64::consts::PI * freq * t as f64 / fs).sin())
            .collect();
        let ratio = spectral_peak_ratio(&sig, freq, fs, 60);
        // 窓が満杯になる t=60 以降で比が高くなるはず
        let mean_ratio: f64 = ratio[60..].iter().sum::<f64>() / (n - 60) as f64;
        assert!(mean_ratio > 0.5, "sin 2Hz のピーク比が低すぎる: {mean_ratio:.4}");
    }

    #[test]
    fn test_spectral_peak_ratio_noise() {
        use rand::RngExt;
        use rand::SeedableRng;
        use rand::rngs::SmallRng;

        let mut rng = SmallRng::seed_from_u64(99);
        // Box-Muller でガウシアンノイズ生成
        let sig: Vec<f64> = (0..100)
            .map(|_| {
                let u1: f64 = (rng.random::<f64>()).max(f64::MIN_POSITIVE);
                let u2: f64 = rng.random::<f64>();
                (-2.0_f64 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
            })
            .collect();
        let ratio = spectral_peak_ratio(&sig, 2.0, 30.0, 60);
        let mean_ratio: f64 = ratio[60..].iter().sum::<f64>() / (100 - 60) as f64;
        // ノイズは全周波数に均等分布 → 1/(window/2+1) ≈ 0.032 程度
        assert!(mean_ratio < 0.15, "ノイズのピーク比が高すぎる: {mean_ratio:.4}");
    }

    #[test]
    fn test_spectral_peak_ratio_zeros_prefix() {
        let sig = vec![0.0_f64; 100];
        let ratio = spectral_peak_ratio(&sig, 2.0, 30.0, 60);
        for i in 0..60 {
            assert_eq!(ratio[i], 0.0, "先頭 window 要素が 0 でない: ratio[{i}]={}", ratio[i]);
        }
    }

    #[test]
    fn test_percentile() {
        let mut v = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        assert!((percentile(&mut v, 0.0) - 1.0).abs() < 1e-9);
        assert!((percentile(&mut v, 100.0) - 5.0).abs() < 1e-9);
        assert!((percentile(&mut v, 50.0) - 3.0).abs() < 1e-9);
    }
}
