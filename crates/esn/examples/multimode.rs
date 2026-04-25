//! Phase 10 実験: ESN 6 モード 2ch 波形分類 + SpectralPeak 未知モード検出
//!
//! 実行: `cargo run -p esn --example multimode`
//! 出力: output/confusion_matrix.csv, output/unknown_detection.csv

use std::fs;
use std::path::Path;

use esn::{EsnConfig, FREQ, FS, N_MODES, WARMUP};
use esn::data::{MODE_NAMES, generate_segment, generate_test_sequence, generate_unknown};
use esn::detector::{percentile, spectral_peak_ratio};
use esn::readout::RidgeReadout;
use esn::reservoir::Reservoir;
use rand::SeedableRng;
use rand::rngs::SmallRng;

const TRAIN_STEPS: usize = 2000;
const TEST_STEPS: usize = 1000;
const SEED: u64 = 42;
const RIDGE: f64 = 1e-4;
const NOISE_STD: f64 = 0.01;
const FFT_WINDOW: usize = 60;

fn main() {
    println!("{}", "=".repeat(60));
    println!("ESN 6 モード 2ch 波形 分類実験 (Rust)");
    println!("{}", "=".repeat(60));

    let config = EsnConfig {
        units: 200,
        sr: 0.9,
        lr: 0.3,
        ridge: RIDGE,
        input_dim: 2,
        n_modes: N_MODES,
        seed: SEED,
    };

    // ── [1] 訓練データ生成 ──────────────────────────────────────────────
    println!("\n[1] 訓練データ生成 (6 モード × {TRAIN_STEPS} step)");
    let mut rng_train = SmallRng::seed_from_u64(SEED);
    let train_segs: Vec<Vec<f64>> = (0..N_MODES)
        .map(|m| generate_segment(m, TRAIN_STEPS, NOISE_STD, &mut rng_train))
        .collect();

    // ── [2] リザーバ構築 ────────────────────────────────────────────────
    println!("[2] リザーバ構築 (units={}, sr={}, lr={})", config.units, config.sr, config.lr);
    let mut reservoir = Reservoir::new(&config);

    // ── [3] 状態収集（モードごとリセット、warmup 除外）──────────────────
    println!("[3] 状態収集 (warmup={WARMUP} 除外)");
    let units = config.units;
    let mut all_states: Vec<f64> = Vec::new();
    let mut all_labels: Vec<usize> = Vec::new();

    for (mode_idx, seg) in train_segs.iter().enumerate() {
        reservoir.reset();
        let states = reservoir.run(seg, TRAIN_STEPS);
        // warmup 後の状態のみ使用
        all_states.extend_from_slice(&states[WARMUP * units..]);
        let n_valid = TRAIN_STEPS - WARMUP;
        all_labels.extend(std::iter::repeat(mode_idx).take(n_valid));
    }

    let n_train = all_labels.len();
    println!("  学習サンプル数: {n_train}");

    // ── [4] Ridge fit（6 クラス one-hot）────────────────────────────────
    println!("[4] Ridge 学習 (ridge={RIDGE})");
    let n_out = N_MODES;
    let mut targets = vec![0.0_f64; n_train * n_out];
    for (i, &label) in all_labels.iter().enumerate() {
        targets[i * n_out + label] = 1.0;
    }

    let mut readout = RidgeReadout::new(RIDGE);
    readout.fit(&all_states, n_train, &targets, n_out, units).unwrap();

    // ── [5] テスト系列生成 ──────────────────────────────────────────────
    println!("[5] テスト系列生成 (6 モード × {TEST_STEPS} step, shuffle)");
    let mut rng_test = SmallRng::seed_from_u64(SEED + 1);
    let (test_data, true_labels, segments) =
        generate_test_sequence(TEST_STEPS, NOISE_STD, &mut rng_test);
    let n_test = true_labels.len();
    let order_str: Vec<&str> = segments.iter().map(|&(m, _, _)| MODE_NAMES[m]).collect();
    println!("  モード順: {}", order_str.join(" → "));

    // ── [6] 推論（連続系列、リザーバリセットなし）───────────────────────
    println!("[6] 推論");
    reservoir.reset();
    let test_states = reservoir.run(&test_data, n_test);
    let scores = readout.run(&test_states, n_test, units).unwrap();
    let pred_labels: Vec<usize> = (0..n_test)
        .map(|t| {
            let s = &scores[t * n_out..(t + 1) * n_out];
            s.iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                .map(|(i, _)| i)
                .unwrap()
        })
        .collect();

    // ── [7] 評価（各モードブロック先頭 warmup を除外）───────────────────
    println!("[7] 評価");
    let mut mask = vec![true; n_test];
    for &(_, start, _) in &segments {
        for i in start..(start + WARMUP).min(n_test) {
            mask[i] = false;
        }
    }

    let mut confusion = vec![0_u64; N_MODES * N_MODES];
    for t in 0..n_test {
        if mask[t] {
            confusion[true_labels[t] * N_MODES + pred_labels[t]] += 1;
        }
    }

    let per_mode_acc: Vec<f64> = (0..N_MODES)
        .map(|i| {
            let row_sum: u64 = (0..N_MODES).map(|j| confusion[i * N_MODES + j]).sum();
            if row_sum > 0 {
                confusion[i * N_MODES + i] as f64 / row_sum as f64
            } else {
                0.0
            }
        })
        .collect();

    let total_correct: u64 = (0..N_MODES).map(|i| confusion[i * N_MODES + i]).sum();
    let total: u64 = confusion.iter().sum();
    let overall_acc = total_correct as f64 / total as f64;

    println!("\n  全体精度: {:.1}%", overall_acc * 100.0);
    println!("  モード別精度:");
    for (i, (name, acc)) in MODE_NAMES.iter().zip(&per_mode_acc).enumerate() {
        let bar = "█".repeat((acc * 20.0) as usize);
        println!("    [{i}] {name:8}: {:6.1}%  {bar}", acc * 100.0);
    }

    println!("\n  混同行列（行=正解、列=予測）:");
    let header = "         ".to_string()
        + &MODE_NAMES
            .iter()
            .map(|n| format!("{n:>8}"))
            .collect::<Vec<_>>()
            .join("  ");
    println!("{header}");
    for i in 0..N_MODES {
        let vals: String = (0..N_MODES)
            .map(|j| format!("{:8}", confusion[i * N_MODES + j]))
            .collect::<Vec<_>>()
            .join("  ");
        println!("  [{}] {:8}  {vals}", i, MODE_NAMES[i]);
    }

    // ── CSV 出力（混同行列）─────────────────────────────────────────────
    let output_dir = Path::new("output");
    fs::create_dir_all(output_dir).unwrap();

    let cm_path = output_dir.join("confusion_matrix.csv");
    let mut cm_csv = String::from("true\\pred");
    for name in MODE_NAMES {
        cm_csv.push(',');
        cm_csv.push_str(name);
    }
    cm_csv.push('\n');
    for i in 0..N_MODES {
        cm_csv.push_str(MODE_NAMES[i]);
        for j in 0..N_MODES {
            cm_csv.push(',');
            cm_csv.push_str(&confusion[i * N_MODES + j].to_string());
        }
        cm_csv.push('\n');
    }
    fs::write(&cm_path, &cm_csv).unwrap();
    println!("\n  Saved: {}", cm_path.display());

    // ── [8] 未知モード検出テスト ─────────────────────────────────────────
    println!("\n[8] 未知モード検出テスト (A=sin, B=ノイズのみ)");
    let mut unk_csv = String::from("noise_std,segment,mode,metric,value\n");

    for (noise_label, noise_std_test) in [("0.01", NOISE_STD), ("0.30", 0.30_f64)] {
        println!("\n  --- noise_std = {noise_label} ---");

        // 訓練データと同じノイズレベルで閾値計算
        let mut rng_thr = SmallRng::seed_from_u64(SEED);
        let thr_segs: Vec<Vec<f64>> = (0..N_MODES)
            .map(|m| generate_segment(m, TRAIN_STEPS, noise_std_test, &mut rng_thr))
            .collect();

        let mut all_ratios: Vec<f64> = Vec::new();
        for seg in &thr_segs {
            let b_ch: Vec<f64> = seg.iter().skip(1).step_by(2).copied().collect();
            let ratio = spectral_peak_ratio(&b_ch, FREQ, FS, FFT_WINDOW);
            all_ratios.extend_from_slice(&ratio[WARMUP..]);
        }
        let thr = percentile(&mut all_ratios, 1.0);
        println!("  SpectralPeak 閾値 (1 パーセンタイル): {thr:.5}");

        // テスト系列: known × 3 + unknown × 3 交互
        let mut rng = SmallRng::seed_from_u64(SEED + 99);
        let known_ids = [0_usize, 1, 4]; // sin-sin, sin-saw, saw-squ

        let mut x_parts: Vec<Vec<f64>> = Vec::new();
        let mut seg_info: Vec<(&str, i32, usize, usize)> = Vec::new();
        let mut pos = 0;
        for &mode_idx in &known_ids {
            x_parts.push(generate_segment(mode_idx, TEST_STEPS, noise_std_test, &mut rng));
            seg_info.push(("known", mode_idx as i32, pos, pos + TEST_STEPS));
            pos += TEST_STEPS;
            x_parts.push(generate_unknown(TEST_STEPS, noise_std_test, &mut rng));
            seg_info.push(("unknown", -1, pos, pos + TEST_STEPS));
            pos += TEST_STEPS;
        }

        let x_seq: Vec<f64> = x_parts.into_iter().flatten().collect();
        let b_ch: Vec<f64> = x_seq.iter().skip(1).step_by(2).copied().collect();
        let peak_ratio = spectral_peak_ratio(&b_ch, FREQ, FS, FFT_WINDOW);
        let is_unknown: Vec<bool> = peak_ratio.iter().map(|&r| r < thr).collect();

        for &(kind, mode_idx, start, end) in &seg_info {
            let eval_start = (start + WARMUP).min(end);
            if eval_start >= end {
                continue;
            }
            let seg_flags = &is_unknown[eval_start..end];
            let flagged_rate =
                seg_flags.iter().filter(|&&b| b).count() as f64 / seg_flags.len() as f64;

            if kind == "unknown" {
                let ok = if flagged_rate >= 0.9 { "✓" } else { "✗" };
                println!("  [unknown] unknown      検出率={:.1}%  {ok}", flagged_rate * 100.0);
                unk_csv.push_str(&format!(
                    "{noise_label},unknown,-1,detection_rate,{flagged_rate:.4}\n"
                ));
            } else {
                let mode_name = MODE_NAMES[mode_idx as usize];
                let ok = if flagged_rate <= 0.05 { "✓" } else { "✗" };
                println!(
                    "  [known  ] {mode_name:12} 誤検知率={:.1}%  {ok}",
                    flagged_rate * 100.0
                );
                unk_csv.push_str(&format!(
                    "{noise_label},known,{mode_name},false_alarm_rate,{flagged_rate:.4}\n"
                ));
            }
        }
    }

    let unk_path = output_dir.join("unknown_detection.csv");
    fs::write(&unk_path, &unk_csv).unwrap();
    println!("\n  Saved: {}", unk_path.display());

    // ── 精度アサーション ──────────────────────────────────────────────────
    assert!(
        overall_acc >= 0.90,
        "全体精度 {:.1}% が 90% を下回った",
        overall_acc * 100.0
    );
    println!("\n{}", "=".repeat(60));
    println!("完了  全体精度: {:.1}%", overall_acc * 100.0);
}
