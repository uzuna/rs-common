"""ESN による 2 チャンネル同期信号の異常検知実験。

A: 2Hz サイン波（常に正常）
B: 通常は A の 0.5 倍（同期）。異常セグメントでは位相シフトまたは振幅変化。

出力:
    output/result_dual_S{1..4}.png        - シナリオ波形プロット
    output/result_dual_phase_sweep.png    - 位相スイープ検出率
    output/result_dual_amp_sweep.png      - 振幅スイープ検出率
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

from esn_anomaly.data_dual import (
    AMP_B_NORMAL,
    KIND_AMPLITUDE,
    KIND_PHASE,
    SCENARIO_DUAL_AMP,
    SCENARIO_DUAL_MIXED,
    SCENARIO_DUAL_NORMAL,
    SCENARIO_DUAL_PHASE,
    DualSegmentInfo,
    generate_dual_spike_train,
    generate_dual_test_scenario,
    generate_dual_train,
)
from esn_anomaly.detector import compute_threshold, detect_persistent, smooth
from esn_anomaly.model import ESNConfig, ESNModel

# ──── 定数 ────────────────────────────────────────────────────────────────────
FS: float = 30.0
FREQ: float = 2.0
NOISE_STD: float = 0.01
TRAIN_STEPS: int = 5000
TEST_STEPS: int = 3000
WARMUP: int = 100
SMOOTH_WIN: int = 15
MIN_DURATION: int = 60       # 2 秒 @ 30 Hz
SEED: int = 42
OUTPUT_DIR = Path("output")

# スイープパラメータ
PHASE_SWEEP_DEGS: list[float] = [5, 10, 20, 30, 45, 60, 90, 120, 150, 180, 210, 270, 330]
AMP_SWEEP_RATIOS: list[float] = [
    0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.45,
    0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80,
]
N_TRIALS: int = 3


# ──── 学習 ───────────────────────────────────────────────────────────────────
def _train() -> tuple[ESNModel, np.ndarray, float]:
    """訓練データで ESN を学習し、(model, warmup_data, threshold) を返す。"""
    rng_tr = np.random.default_rng(SEED)
    u_tr = generate_dual_train(TRAIN_STEPS, FS, FREQ, NOISE_STD, rng_tr)
    X_tr, y_tr = u_tr[:-1], u_tr[1:]

    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED)
    model = ESNModel(config)
    model.fit(X_tr, y_tr)

    pred_tr = model.predict(X_tr)
    e_b_tr = np.abs(y_tr - pred_tr)[:, 1]              # B チャンネル残差
    smoothed_tr = smooth(e_b_tr, window=SMOOTH_WIN)
    threshold = compute_threshold(smoothed_tr[WARMUP:], method="3sigma")

    train_rmse_b = float(np.sqrt(np.mean(e_b_tr[WARMUP:] ** 2)))
    print(f"    訓練 RMSE (B ch): {train_rmse_b:.5f}   閾値: {threshold:.5f}")

    return model, X_tr[-WARMUP:], threshold


# ──── 推論・検知 ──────────────────────────────────────────────────────────────
def _detect(
    model: ESNModel,
    u_test: np.ndarray,
    warmup_data: np.ndarray,
    threshold: float,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """予測 → B 残差 → スムージング → 持続検知。

    Returns:
        pred       shape (n-1, 2)
        e_b        shape (n-1,)  B チャンネル残差
        smoothed_b shape (n-1,)  スムーズ済み B 残差
        detections 検知開始インデックス配列
    """
    X_te, y_te = u_test[:-1], u_test[1:]
    pred = model.predict(X_te, warmup_data=warmup_data)
    e_b = np.abs(y_te - pred)[:, 1]
    smoothed_b = smooth(e_b, window=SMOOTH_WIN)
    detections = detect_persistent(smoothed_b, threshold, min_duration=MIN_DURATION)
    return pred, e_b, smoothed_b, detections


def _segment_detected(seg: DualSegmentInfo, detections: np.ndarray, slack: int = 120) -> bool:
    """detections の中で seg 区間 ± slack に収まるものがあれば True。"""
    for d in detections:
        if seg.start - slack <= int(d) <= seg.end + slack:
            return True
    return False


# ──── プロット ────────────────────────────────────────────────────────────────
def _plot_dual(
    t: np.ndarray,
    u: np.ndarray,
    pred: np.ndarray,
    e_b: np.ndarray,
    smoothed_b: np.ndarray,
    segments: list[DualSegmentInfo],
    detections: np.ndarray,
    threshold: float,
    title: str,
    path: Path,
) -> None:
    """3 段グラフ（波形+予測 / B 残差 / スムーズ残差+検知）を保存する。"""
    path.parent.mkdir(parents=True, exist_ok=True)
    fig, (ax1, ax2, ax3) = plt.subplots(3, 1, figsize=(14, 9), sharex=True)
    fig.suptitle(title, fontsize=11)

    a_in = u[:, 0]
    b_in = u[:, 1]
    b_pred = pred[:, 1]

    # ── 上段: A, B, B 予測 ──
    ax1.plot(t, a_in,   color="steelblue", linewidth=0.7, alpha=0.85, label="A (input)")
    ax1.plot(t, b_in,   color="orange",    linewidth=0.7, alpha=0.85, label="B (input)")
    ax1.plot(t, b_pred, color="limegreen", linewidth=0.9, linestyle="--",
             alpha=0.85, label="B (predicted)")
    for seg in segments:
        ax1.axvspan(t[seg.start], t[min(seg.end, len(t) - 1)], alpha=0.15, color="red")
    ax1.axvspan(0, 0, alpha=0.3, color="red", label="Anomaly segment")
    ax1.set_ylabel("Amplitude")
    ax1.legend(loc="upper right", fontsize=8)
    ax1.grid(True, alpha=0.3)

    # ── 中段: B 残差 ──
    ax2.plot(t, e_b, color="dimgray", linewidth=0.6, alpha=0.8, label="B residual |u-pred|")
    for seg in segments:
        ax2.axvspan(t[seg.start], t[min(seg.end, len(t) - 1)], alpha=0.15, color="red")
    ax2.set_ylabel("B Residual")
    ax2.legend(loc="upper right", fontsize=8)
    ax2.grid(True, alpha=0.3)

    # ── 下段: スムーズ残差 + 閾値 + 検知マーカー ──
    ax3.plot(t, smoothed_b, color="purple", linewidth=0.9, alpha=0.85,
             label="Smoothed B residual")
    ax3.axhline(threshold, color="crimson", linestyle="--", linewidth=1.2,
                label=f"Threshold {threshold:.4f}")
    for seg in segments:
        ax3.axvspan(t[seg.start], t[min(seg.end, len(t) - 1)], alpha=0.15, color="red")
    for d in detections:
        if int(d) < len(t):
            ax3.axvline(t[int(d)], color="crimson", linewidth=1.0, alpha=0.7)
    ax3.set_xlabel("Time [s]")
    ax3.set_ylabel("Smoothed B residual")
    ax3.legend(loc="upper right", fontsize=8)
    ax3.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── シナリオ実行 ─────────────────────────────────────────────────────────────
def run_scenarios(model: ESNModel, warmup_data: np.ndarray, threshold: float) -> None:
    """S1〜S4 シナリオを実行してプロットを保存する。"""
    scenario_info = {
        SCENARIO_DUAL_NORMAL: ("S1", "Normal only"),
        SCENARIO_DUAL_PHASE:  ("S2", "Phase shift anomaly"),
        SCENARIO_DUAL_AMP:    ("S3", "Amplitude anomaly"),
        SCENARIO_DUAL_MIXED:  ("S4", "Mixed (phase + amplitude)"),
    }
    expected_normal = {SCENARIO_DUAL_NORMAL}
    all_pass = True

    print("\n[4] テストシナリオ実行\n")
    for sid, (tag, desc) in scenario_info.items():
        rng = np.random.default_rng(sid + 100)
        u_test, segments = generate_dual_test_scenario(
            sid, n_steps=TEST_STEPS, rate_hz=0.05,
            fs=FS, freq=FREQ, noise_std=NOISE_STD, rng=rng,
        )
        pred, e_b, smoothed_b, detections = _detect(model, u_test, warmup_data, threshold)
        t = np.arange(len(u_test) - 1) / FS

        if sid in expected_normal:
            ok = len(detections) == 0
            status = "OK (誤検知なし)" if ok else f"FAIL (誤検知 {len(detections)} 件)"
        else:
            n_det = sum(1 for seg in segments if _segment_detected(seg, detections))
            ok = n_det == len(segments)
            if ok:
                status = f"OK ({n_det}/{len(segments)} 検知)"
            else:
                status = f"FAIL ({n_det}/{len(segments)})"

        if not ok:
            all_pass = False
        seg_info = f"  セグメント数: {len(segments)}" if segments else ""
        print(f"  [{tag}] {desc}")
        print(f"    検知数: {len(detections)}{seg_info}  結果: {status}")

        _plot_dual(
            t=t, u=u_test[:-1], pred=pred, e_b=e_b, smoothed_b=smoothed_b,
            segments=segments, detections=detections, threshold=threshold,
            title=f"ESN Dual-Channel [{tag}]: {desc}",
            path=OUTPUT_DIR / f"result_dual_{tag}.png",
        )

    print()
    print("=" * 60)
    print("結果: 全テストパス" if all_pass else "結果: 一部失敗（パラメータ調整が必要）")


# ──── スイープ ────────────────────────────────────────────────────────────────
def _sweep_rate(
    model: ESNModel,
    warmup_data: np.ndarray,
    threshold: float,
    kind: str,
    fixed_phase_deg: float | None,
    fixed_amp_ratio: float | None,
) -> float:
    """N_TRIALS 試行の平均セグメント検出率を返す。"""
    rates: list[float] = []
    for trial in range(N_TRIALS):
        rng = np.random.default_rng(2000 + trial)
        u_test, segs = generate_dual_spike_train(
            TEST_STEPS, [kind],
            rate_hz=0.05, fs=FS, freq=FREQ, noise_std=NOISE_STD,
            fixed_phase_deg=fixed_phase_deg,
            fixed_amp_ratio=fixed_amp_ratio,
            rng=rng,
        )
        if not segs:
            continue
        _, _, smoothed_b, dets = _detect(model, u_test, warmup_data, threshold)
        detected = sum(1 for seg in segs if _segment_detected(seg, dets))
        rates.append(detected / len(segs))
    return float(np.mean(rates)) if rates else float("nan")


def run_sweep(model: ESNModel, warmup_data: np.ndarray, threshold: float) -> None:
    """位相スイープ・振幅スイープを実行し検出率グラフを保存する。"""
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    # ── 位相スイープ ──
    print("\n[5] 位相スイープ (B phase shift)")
    phase_rates = []
    for deg in PHASE_SWEEP_DEGS:
        r = _sweep_rate(model, warmup_data, threshold, KIND_PHASE,
                        fixed_phase_deg=float(deg), fixed_amp_ratio=None)
        phase_rates.append(r)
        print(f"    phase={deg:6.1f}°  detection rate={r:.2f}")

    fig, ax = plt.subplots(figsize=(10, 4))
    ax.plot(PHASE_SWEEP_DEGS, [r * 100 for r in phase_rates],
            "o-", color="royalblue", linewidth=1.5, markersize=5)
    ax.set_xlabel("B phase shift [deg]")
    ax.set_ylabel("Detection rate [%]")
    ax.set_title("ESN Dual-Channel: Detection rate vs. B phase shift  (normal ref = 0°)")
    ax.set_ylim(-5, 105)
    ax.grid(True, alpha=0.3)
    plt.tight_layout()
    p = OUTPUT_DIR / "result_dual_phase_sweep.png"
    plt.savefig(p, dpi=150)
    plt.close(fig)
    print(f"  Saved: {p}")

    # ── 振幅スイープ ──
    print("\n[6] 振幅スイープ (B amplitude ratio)")
    amp_rates = []
    for amp in AMP_SWEEP_RATIOS:
        r = _sweep_rate(model, warmup_data, threshold, KIND_AMPLITUDE,
                        fixed_phase_deg=None, fixed_amp_ratio=amp)
        amp_rates.append(r)
        print(f"    amp={amp:.2f}  detection rate={r:.2f}")

    fig, ax = plt.subplots(figsize=(10, 4))
    ax.plot(AMP_SWEEP_RATIOS, [r * 100 for r in amp_rates],
            "o-", color="darkorange", linewidth=1.5, markersize=5)
    ax.axvline(AMP_B_NORMAL, color="gray", linestyle="--", linewidth=1.0,
               label=f"Normal amp ({AMP_B_NORMAL})")
    ax.set_xlabel("B amplitude ratio (normal = 0.5)")
    ax.set_ylabel("Detection rate [%]")
    ax.set_title("ESN Dual-Channel: Detection rate vs. B amplitude ratio")
    ax.set_ylim(-5, 105)
    ax.legend(fontsize=8)
    ax.grid(True, alpha=0.3)
    plt.tight_layout()
    p = OUTPUT_DIR / "result_dual_amp_sweep.png"
    plt.savefig(p, dpi=150)
    plt.close(fig)
    print(f"  Saved: {p}")


# ──── メイン ──────────────────────────────────────────────────────────────────
def run() -> None:
    print("=" * 60)
    print("ESN 2 チャンネル同期信号 異常検知実験")
    print("=" * 60)

    print("\n[1] 訓練データ生成（正常 A-B、5000 step）")
    print("[2] ESN 学習 (units=200, 2ch 入出力)")
    print("[3] 閾値算出 (3σ)")
    model, warmup_data, threshold = _train()

    run_scenarios(model, warmup_data, threshold)
    run_sweep(model, warmup_data, threshold)

    print("=" * 60)


if __name__ == "__main__":
    run()
