"""アノテーション付きデータを用いた学習条件比較実験。

4 条件で ESN を学習して異常検知性能を比較する:

  A (clean)        正常データのみで学習（ベースライン）
  B (contaminated) 異常区間を含む汚染データで学習（アノテーションなし）
  C (masked)       汚染データでリザーバを動かし、Readout は正常区間のみで学習
  D (ann-input)    学習時は [A, B, ann(0/1)] → [A, B]。テスト時は ann=0 で推論

出力:
    output/result_dual_ann_heatmap.png        条件×シナリオ 検出率ヒートマップ
    output/result_dual_ann_waveform_S2.png    S2 での 4条件波形比較
    output/result_dual_ann_waveform_S3.png    S3 での 4条件波形比較
    output/result_dual_ann_waveform_S4.png    S4 での 4条件波形比較
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

from esn_anomaly.data_dual import (
    KIND_AMPLITUDE,
    KIND_MIXED,
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
MIN_DURATION: int = 60
SEED: int = 42
# 汚染データの異常発生率（~26% の区間が異常になる）
CONT_RATE_HZ: float = 0.03
OUTPUT_DIR = Path("output")

_TrainResult = tuple[ESNModel, np.ndarray, float, str]

_COND_LABELS: dict[str, str] = {
    "A": "A: Clean only",
    "B": "B: Contaminated",
    "C": "C: Masked readout",
    "D": "D: Ann as 3rd ch",
}
_SCENARIO_LABELS: dict[int, str] = {
    SCENARIO_DUAL_NORMAL: "S1 Normal",
    SCENARIO_DUAL_PHASE:  "S2 Phase",
    SCENARIO_DUAL_AMP:    "S3 Amplitude",
    SCENARIO_DUAL_MIXED:  "S4 Mixed",
}


# ──── アノテーションマスク生成 ─────────────────────────────────────────────────
def _make_normal_mask(n: int, segments: list[DualSegmentInfo]) -> np.ndarray:
    """True=正常 のブール配列を生成する。"""
    mask = np.ones(n, dtype=bool)
    for seg in segments:
        mask[seg.start:seg.end] = False
    return mask


# ──── 汚染データの生成（B/C/D で共有） ───────────────────────────────────────
def _make_contaminated_train(seed: int) -> tuple[np.ndarray, list[DualSegmentInfo]]:
    """B/C/D 条件で共用する訓練データ（異常区間混在）を生成する。"""
    rng = np.random.default_rng(seed)
    all_kinds = [KIND_PHASE, KIND_AMPLITUDE, KIND_MIXED]
    u, segs = generate_dual_spike_train(
        TRAIN_STEPS, all_kinds,
        rate_hz=CONT_RATE_HZ, fs=FS, freq=FREQ, noise_std=NOISE_STD, rng=rng,
    )
    return u, segs


# ──── 各条件の学習 ─────────────────────────────────────────────────────────────
def _esn_config() -> ESNConfig:
    return ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED)


def train_cond_a() -> tuple[ESNModel, np.ndarray, float, str]:
    """A: 正常データのみで学習（ベースライン）。"""
    rng = np.random.default_rng(SEED)
    u_tr = generate_dual_train(TRAIN_STEPS, FS, FREQ, NOISE_STD, rng)
    X_tr, y_tr = u_tr[:-1], u_tr[1:]
    model = ESNModel(_esn_config())
    model.fit(X_tr, y_tr)
    e_b = np.abs(y_tr - model.predict(X_tr))[:, 1]
    thr = compute_threshold(smooth(e_b, SMOOTH_WIN)[WARMUP:], method="3sigma")
    return model, X_tr[-WARMUP:], thr, "A"


def train_cond_b(u_tr: np.ndarray, segs: list[DualSegmentInfo]) -> _TrainResult:
    """B: 汚染データで学習。閾値計算も全区間（アノテーションなし）。"""
    X_tr, y_tr = u_tr[:-1], u_tr[1:]
    model = ESNModel(_esn_config())
    model.fit(X_tr, y_tr)
    e_b = np.abs(y_tr - model.predict(X_tr))[:, 1]
    # アノテーションなし → 全区間から閾値を計算
    thr = compute_threshold(smooth(e_b, SMOOTH_WIN)[WARMUP:], method="3sigma")
    return model, X_tr[-WARMUP:], thr, "B"


def train_cond_c(u_tr: np.ndarray, segs: list[DualSegmentInfo]) -> _TrainResult:
    """C: 汚染データでリザーバを動かし、Readout は正常区間のみで学習。"""
    X_tr, y_tr = u_tr[:-1], u_tr[1:]
    normal_mask = _make_normal_mask(len(X_tr), segs)
    model = ESNModel(_esn_config())
    model.fit_masked(X_tr, y_tr, normal_mask)
    e_b = np.abs(y_tr - model.predict(X_tr))[:, 1]
    thr = compute_threshold(smooth(e_b, SMOOTH_WIN)[WARMUP:][normal_mask[WARMUP:]], method="3sigma")
    return model, X_tr[-WARMUP:], thr, "C"


def train_cond_d(u_tr: np.ndarray, segs: list[DualSegmentInfo]) -> _TrainResult:
    """D: [A, B, ann] → [A, B] 学習。テスト時は ann=0 で推論。"""
    X_tr_2d, y_tr = u_tr[:-1], u_tr[1:]
    ann = (~_make_normal_mask(len(X_tr_2d), segs)).astype(np.float64).reshape(-1, 1)
    X_tr_3d = np.hstack([X_tr_2d, ann])  # shape (n, 3)
    model = ESNModel(_esn_config())
    model.fit(X_tr_3d, y_tr)
    # 閾値は正常区間の B 残差から計算
    warmup_3d = X_tr_3d[-WARMUP:]
    pred_tr = model.predict(X_tr_3d, warmup_data=warmup_3d)
    e_b = np.abs(y_tr - pred_tr)[:, 1]
    normal_mask = _make_normal_mask(len(X_tr_2d), segs)
    thr = compute_threshold(smooth(e_b, SMOOTH_WIN)[WARMUP:][normal_mask[WARMUP:]], method="3sigma")
    return model, warmup_3d, thr, "D"


# ──── 推論・検知 ──────────────────────────────────────────────────────────────
def _detect(
    cond: str,
    model: ESNModel,
    u_test: np.ndarray,
    warmup_data: np.ndarray,
    threshold: float,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """条件に応じた推論と検知。

    Returns:
        pred       shape (n-1, 2)
        e_b        B チャンネル残差
        smoothed_b スムーズ済み残差
        detections 検知開始インデックス
    """
    X_te, y_te = u_test[:-1], u_test[1:]
    if cond == "D":
        zeros = np.zeros((len(X_te), 1))
        X_input = np.hstack([X_te, zeros])
        wd = np.hstack([warmup_data[:, :2], np.zeros((len(warmup_data), 1))])
    else:
        X_input, wd = X_te, warmup_data
    pred = model.predict(X_input, warmup_data=wd)
    e_b = np.abs(y_te - pred)[:, 1]
    smoothed_b = smooth(e_b, window=SMOOTH_WIN)
    detections = detect_persistent(smoothed_b, threshold, min_duration=MIN_DURATION)
    return pred, e_b, smoothed_b, detections


def _segment_detected(seg: DualSegmentInfo, detections: np.ndarray, slack: int = 120) -> bool:
    for d in detections:
        if seg.start - slack <= int(d) <= seg.end + slack:
            return True
    return False


# ──── 可視化 ──────────────────────────────────────────────────────────────────
def _plot_waveform_comparison(
    scenario_id: int,
    u_test: np.ndarray,
    segments: list[DualSegmentInfo],
    results: dict[str, tuple],   # cond → (pred, e_b, smoothed_b, detections, threshold)
) -> None:
    """4条件の波形比較を 4行 × 2列 で保存する。"""
    conds = list(results.keys())
    n = len(u_test) - 1
    t = np.arange(n) / FS

    fig, axes = plt.subplots(len(conds), 2, figsize=(14, 3 * len(conds)), sharex=True)
    tag = _SCENARIO_LABELS.get(scenario_id, f"S{scenario_id}")
    fig.suptitle(
        f"4-condition comparison: {tag}\n"
        "Left: A(blue) B(orange) B-pred(green dashed)   Right: Smoothed B residual + threshold",
        fontsize=10,
    )

    for row, cond in enumerate(conds):
        pred, e_b, smoothed_b, dets, thr = results[cond]
        ax_w = axes[row][0]
        ax_r = axes[row][1]
        label = _COND_LABELS[cond]

        # 左: 波形
        ax_w.plot(t, u_test[:-1, 0], color="steelblue",  lw=0.6, alpha=0.8, label="A")
        ax_w.plot(t, u_test[:-1, 1], color="orange",     lw=0.6, alpha=0.8, label="B actual")
        ax_w.plot(t, pred[:, 1],     color="limegreen",  lw=0.8, ls="--", alpha=0.9, label="B pred")
        for seg in segments:
            ax_w.axvspan(t[seg.start], t[min(seg.end, n - 1)], alpha=0.15, color="red")
        ax_w.set_ylabel(label, fontsize=8)
        ax_w.legend(loc="upper right", fontsize=6)
        ax_w.grid(True, alpha=0.3)
        ax_w.tick_params(labelsize=6)

        # 右: スムーズ残差 + 閾値 + 検知マーカー
        ax_r.plot(t, smoothed_b, color="purple", lw=0.8, alpha=0.85)
        ax_r.axhline(thr, color="crimson", ls="--", lw=1.0,
                     label=f"thr={thr:.4f}")
        for seg in segments:
            ax_r.axvspan(t[seg.start], t[min(seg.end, n - 1)], alpha=0.15, color="red")
        for d in dets:
            if int(d) < n:
                ax_r.axvline(t[int(d)], color="crimson", lw=0.8, alpha=0.7)
        ax_r.legend(loc="upper right", fontsize=6)
        ax_r.grid(True, alpha=0.3)
        ax_r.tick_params(labelsize=6)

        if row == len(conds) - 1:
            ax_w.set_xlabel("Time [s]", fontsize=8)
            ax_r.set_xlabel("Time [s]", fontsize=8)

    plt.tight_layout()
    sid = f"S{scenario_id}"
    path = OUTPUT_DIR / f"result_dual_ann_waveform_{sid}.png"
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


def _plot_heatmap(table: dict[str, dict[int, float]]) -> None:
    """条件 × シナリオ の検出率ヒートマップを保存する。"""
    conds = list(table.keys())
    sids = [SCENARIO_DUAL_NORMAL, SCENARIO_DUAL_PHASE, SCENARIO_DUAL_AMP, SCENARIO_DUAL_MIXED]
    data = np.array([[table[c][s] for s in sids] for c in conds])

    fig, ax = plt.subplots(figsize=(7, 4))
    im = ax.imshow(data, vmin=0, vmax=1, cmap="RdYlGn", aspect="auto")
    ax.set_xticks(range(len(sids)))
    ax.set_xticklabels([_SCENARIO_LABELS[s] for s in sids], fontsize=9)
    ax.set_yticks(range(len(conds)))
    ax.set_yticklabels([_COND_LABELS[c] for c in conds], fontsize=9)
    plt.colorbar(im, ax=ax, label="Detection rate (S1=FP rate, S2-S4=detection)")
    for i, cond in enumerate(conds):
        for j, sid in enumerate(sids):
            v = data[i, j]
            ax.text(j, i, f"{v:.2f}", ha="center", va="center",
                    fontsize=10, color="black")
    ax.set_title(
        "Annotation experiment: detection rate by condition × scenario\n"
        "(S1: lower is better / S2〜S4: higher is better)",
        fontsize=9,
    )
    plt.tight_layout()
    path = OUTPUT_DIR / "result_dual_ann_heatmap.png"
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── メイン ──────────────────────────────────────────────────────────────────
def run() -> None:
    print("=" * 60)
    print("ESN Dual-Channel: アノテーション学習条件比較")
    print("=" * 60)

    # 1. 汚染訓練データを生成（B/C/D 共用）
    u_cont, segs_cont = _make_contaminated_train(SEED)
    normal_frac = _make_normal_mask(len(u_cont) - 1, segs_cont).mean()
    print(f"\n[1] 汚染訓練データ: {TRAIN_STEPS} step  "
          f"異常区間={1 - normal_frac:.1%}  正常区間={normal_frac:.1%}")

    # 2. 各条件で学習
    print("\n[2] 各条件で ESN 学習")
    trained: dict[str, tuple[ESNModel, np.ndarray, float]] = {}
    for cond, (model, wd, thr, _) in [
        ("A", train_cond_a()),
        ("B", train_cond_b(u_cont, segs_cont)),
        ("C", train_cond_c(u_cont, segs_cont)),
        ("D", train_cond_d(u_cont, segs_cont)),
    ]:
        trained[cond] = (model, wd, thr)
        print(f"  [{cond}] {_COND_LABELS[cond]}  threshold={thr:.5f}")

    # 3. テストシナリオを実行
    print("\n[3] テストシナリオ実行")
    scenarios = {
        SCENARIO_DUAL_NORMAL: ("S1", "Normal only"),
        SCENARIO_DUAL_PHASE:  ("S2", "Phase shift"),
        SCENARIO_DUAL_AMP:    ("S3", "Amplitude"),
        SCENARIO_DUAL_MIXED:  ("S4", "Mixed"),
    }
    expected_normal = {SCENARIO_DUAL_NORMAL}
    table: dict[str, dict[int, float]] = {c: {} for c in trained}
    waveform_sids = [SCENARIO_DUAL_PHASE, SCENARIO_DUAL_AMP, SCENARIO_DUAL_MIXED]

    for sid, (tag, desc) in scenarios.items():
        rng = np.random.default_rng(sid + 200)
        u_test, segments = generate_dual_test_scenario(
            sid, n_steps=TEST_STEPS, rate_hz=0.05,
            fs=FS, freq=FREQ, noise_std=NOISE_STD, rng=rng,
        )
        print(f"\n  [{tag}] {desc}  segments={len(segments)}")

        cond_results: dict[str, tuple] = {}
        for cond, (model, wd, thr) in trained.items():
            pred, e_b, smo, dets = _detect(cond, model, u_test, wd, thr)
            if sid in expected_normal:
                rate = len(dets) / max(1, TEST_STEPS // FS)  # FP/sec
                ok_str = "OK" if len(dets) == 0 else f"FP={len(dets)}"
                table[cond][sid] = float(len(dets) == 0)
            else:
                n_det = sum(1 for seg in segments if _segment_detected(seg, dets))
                rate = n_det / max(len(segments), 1)
                ok_str = f"{n_det}/{len(segments)}"
                table[cond][sid] = rate
            print(f"    [{cond}] thr={thr:.4f}  detect={len(dets)}  {ok_str}")
            cond_results[cond] = (pred, e_b, smo, dets, thr)

        if sid in waveform_sids:
            _plot_waveform_comparison(sid, u_test, segments, cond_results)

    # 4. ヒートマップと要約
    print("\n[4] 結果サマリ")
    header = "        " + "  ".join(f"{_SCENARIO_LABELS[s]:12s}" for s in sorted(scenarios))
    print(header)
    for cond in trained:
        row_vals = "  ".join(f"{table[cond][s]:.2f}        " for s in sorted(scenarios))
        print(f"  [{cond}]  {row_vals}")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    _plot_heatmap(table)

    print("\n" + "=" * 60)


if __name__ == "__main__":
    run()
