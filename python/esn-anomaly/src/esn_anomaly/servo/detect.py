"""サーボ異常注入・検知評価実験（Phase 12）。

訓練済み ESN（Phase 11 と同一設定）を使い、4 種の異常を注入したテストデータで
検知性能（検知率・誤警報率）を評価する。

異常種別:
  force      — 外力: 負荷チャンネルに一定オフセット
  pos_spike  — 位置センサスパイク: 突発的な位置誤差
  pos_drift  — 位置センサドリフト: 徐々に増大する位置誤差
  load_stuck — 負荷センサ固着: センサ値が区間先頭で固定

実行:
    uv run python -m esn_anomaly.servo.detect
    make run-servo-detect
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

from esn_anomaly.detector import detect_persistent, smooth
from esn_anomaly.model import ESNModel
from esn_anomaly.servo.data import (
    ANOMALY_FORCE,
    ANOMALY_LOAD_STUCK,
    ANOMALY_POS_DRIFT,
    ANOMALY_POS_SPIKE,
    MeasurementNoise,
    MotionPattern,
    ServoAnomalySegment,
    generate_servo_anomaly_test,
    normalize_servo_obs,
)
from esn_anomaly.servo.train import NOISE, SMOOTH_WIN, WARMUP, build_detector

# ──── 定数 ────────────────────────────────────────────────────────────────────
DETECT_CYCLES: int = 10
SEED_DETECT: int = 99
OUTPUT_DIR = Path("output")

# 検知判定ロジック: 異常区間前後 slack ステップ以内に閾値超過があれば「検知」
_DETECT_SLACK: int = 30
# 持続判定の最小継続長（スムーズ後）
_MIN_DURATION: int = 15


# ──── 内部ユーティリティ ──────────────────────────────────────────────────────

def _segment_detected(
    seg: ServoAnomalySegment,
    s_pos: np.ndarray,
    s_load: np.ndarray,
    thr_pos: float,
    thr_load: float,
    slack: int = _DETECT_SLACK,
) -> bool:
    """pos または load のどちらかが異常区間付近で閾値を超えれば True。"""
    lo = max(0, seg.start - slack)
    hi = min(len(s_pos), seg.end + slack)
    return bool(np.any(s_pos[lo:hi] > thr_pos) or np.any(s_load[lo:hi] > thr_load))


def _run_scenario(
    model: ESNModel,
    warmup_data: np.ndarray,
    thr_pos: float,
    thr_load: float,
    name: str,
    u_raw: np.ndarray,
    segments: list[ServoAnomalySegment],
) -> dict:
    """1 シナリオの推論・検知処理を実行し結果辞書を返す。

    Returns:
        dict with keys: name, detected, total, fp_rate,
          s_pos, s_load, u_norm, pred, segments
    """
    u = normalize_servo_obs(u_raw)
    X, y = u[:-1], u[1:]

    pred = model.predict(X, warmup_data=warmup_data)

    e_pos  = np.abs(y - pred)[:, 0]
    e_load = np.abs(y - pred)[:, 1]
    s_pos  = smooth(e_pos,  window=SMOOTH_WIN)
    s_load = smooth(e_load, window=SMOOTH_WIN)

    # セグメント検知率
    detected = sum(
        _segment_detected(seg, s_pos, s_load, thr_pos, thr_load)
        for seg in segments
    )
    total = len(segments)

    # 誤警報: ウォームアップ後の正常区間での閾値超過率を計算
    # 正常区間マスク（全区間 - 異常セグメント - ウォームアップ）
    n = len(s_pos)
    normal_mask = np.ones(n, dtype=bool)
    normal_mask[:WARMUP] = False
    for seg in segments:
        lo = max(0, seg.start - _DETECT_SLACK)
        hi = min(n, seg.end + _DETECT_SLACK)
        normal_mask[lo:hi] = False
    fp_over = (s_pos[normal_mask] > thr_pos) | (s_load[normal_mask] > thr_load)
    fp_rate = float(fp_over.mean() * 100) if normal_mask.any() else 0.0

    return dict(
        name=name,
        detected=detected,
        total=total,
        fp_rate=fp_rate,
        s_pos=s_pos,
        s_load=s_load,
        u_norm=y,
        pred=pred,
        segments=segments,
        thr_pos=thr_pos,
        thr_load=thr_load,
    )


# ──── プロット ────────────────────────────────────────────────────────────────

def _plot_scenario(result: dict, path: Path) -> None:
    """4 段グラフ（pos/load 波形、pos/load 残差）を保存する。"""
    path.parent.mkdir(parents=True, exist_ok=True)

    u = result["u_norm"]
    pred = result["pred"]
    s_pos = result["s_pos"]
    s_load = result["s_load"]
    thr_pos = result["thr_pos"]
    thr_load = result["thr_load"]
    segments = result["segments"]
    name = result["name"]
    n = len(s_pos)
    t = np.arange(n) * 0.01

    fig, axes = plt.subplots(4, 1, figsize=(14, 12), sharex=True)
    fig.suptitle(f"ESN Servo Anomaly Detection: {name}", fontsize=11)

    def _shade_segments(ax: plt.Axes) -> None:
        for seg in segments:
            ax.axvspan(seg.start * 0.01, (seg.end - 1) * 0.01,
                       alpha=0.15, color="red", label="_nolegend_")

    # ── pos 波形 ──
    ax = axes[0]
    ax.axvspan(0, WARMUP * 0.01, alpha=0.1, color="gray")
    _shade_segments(ax)
    ax.plot(t, u[:, 0], color="steelblue", linewidth=0.6, alpha=0.8, label="pos actual")
    ax.plot(t, pred[:, 0], color="limegreen", linewidth=0.8, linestyle="--",
            alpha=0.9, label="pos predicted")
    ax.set_ylabel("pos (normalized)")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── load 波形 ──
    ax = axes[1]
    ax.axvspan(0, WARMUP * 0.01, alpha=0.1, color="gray")
    _shade_segments(ax)
    ax.plot(t, u[:, 1], color="darkorange", linewidth=0.6, alpha=0.8, label="load actual")
    ax.plot(t, pred[:, 1], color="limegreen", linewidth=0.8, linestyle="--",
            alpha=0.9, label="load predicted")
    ax.set_ylabel("load (normalized)")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── pos 残差 ──
    ax = axes[2]
    ax.axvspan(0, WARMUP * 0.01, alpha=0.1, color="gray")
    _shade_segments(ax)
    ax.plot(t, s_pos, color="navy", linewidth=0.8, label="smoothed pos residual")
    ax.axhline(thr_pos, color="crimson", linestyle="--", linewidth=1.2,
               label=f"threshold {thr_pos:.4f}")
    ax.set_ylabel("pos residual")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── load 残差 ──
    ax = axes[3]
    ax.axvspan(0, WARMUP * 0.01, alpha=0.1, color="gray")
    _shade_segments(ax)
    ax.plot(t, s_load, color="saddlebrown", linewidth=0.8, label="smoothed load residual")
    ax.axhline(thr_load, color="crimson", linestyle="--", linewidth=1.2,
               label=f"threshold {thr_load:.4f}")
    ax.set_xlabel("Time [s]")
    ax.set_ylabel("load residual")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── メイン ──────────────────────────────────────────────────────────────────

def run() -> None:
    print("=" * 65)
    print("ESN サーボ異常検知評価（Phase 12）")
    print("=" * 65)

    print("\n[1] ESN 学習 (Phase 11 と同一設定) ...")
    model, warmup_data, thr_pos, thr_load = build_detector()
    print(f"    閾値 pos={thr_pos:.5f}  load={thr_load:.5f}")

    pat = MotionPattern()
    n_steps = DETECT_CYCLES * pat.steps_per_cycle
    print(f"\n[2] テストデータ生成 ({DETECT_CYCLES} cycles = {n_steps} steps)")

    scenarios_def = [
        # (name, anomaly_kinds)
        ("S1_Normal",     []),
        ("S2_Force",      [ANOMALY_FORCE]),
        ("S3_PosSpike",   [ANOMALY_POS_SPIKE]),
        ("S4_PosDrift",   [ANOMALY_POS_DRIFT]),
        ("S5_LoadStuck",  [ANOMALY_LOAD_STUCK]),
    ]

    print(f"\n{'Scenario':<18}  {'Segments':>8}  {'Detected':>8}  "
          f"{'Det.Rate':>10}  {'FP Rate':>10}")
    print("-" * 65)

    for i, (name, kinds) in enumerate(scenarios_def):
        rng = np.random.default_rng(SEED_DETECT + i)
        u_raw, segments = generate_servo_anomaly_test(
            n_cycles=DETECT_CYCLES,
            anomaly_kinds=kinds,
            noise=NOISE,
            rng=rng,
        )
        result = _run_scenario(
            model, warmup_data, thr_pos, thr_load,
            name, u_raw, segments,
        )
        det = result["detected"]
        tot = result["total"]
        det_rate = f"{det}/{tot}" if tot > 0 else "N/A"
        det_pct  = f"{det/tot*100:.0f}%" if tot > 0 else "-"
        fp_str   = f"{result['fp_rate']:.1f}%"
        print(f"  {name:<16}  {tot:>8}  {det:>8}  {det_pct:>10}  {fp_str:>10}")

        _plot_scenario(result, OUTPUT_DIR / f"result_servo_detect_{name.lower()}.png")

    print("\n" + "=" * 65)


if __name__ == "__main__":
    run()
