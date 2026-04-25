"""ESN によるサーボ位置・負荷の 1 ステップ予測実験。

定期往復モーション（home → up → home → down → home）を訓練データとし、
ESN が (pos_t, load_t) → (pos_{t+1}, load_{t+1}) のマッピングを学習する。

入力・出力チャンネル:
  ch0: 位置 [mrad] 正規化値
  ch1: 負荷電流 [mA] 正規化値

出力:
  output/result_servo_train.png  - 訓練残差（pos/load チャンネル）
  output/result_servo_valid.png  - 検証: 波形+予測 / 残差+閾値（4 段グラフ）
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

from esn_anomaly.servo.data import (
    MeasurementNoise,
    MotionPattern,
    generate_servo_periodic,
    normalize_servo_obs,
)
from esn_anomaly.detector import compute_threshold, smooth
from esn_anomaly.model import ESNConfig, ESNModel

# ──── 定数 ────────────────────────────────────────────────────────────────────
TRAIN_CYCLES: int = 20
VALID_CYCLES: int = 5
WARMUP: int = 100
SMOOTH_WIN: int = 15
SEED: int = 42
OUTPUT_DIR = Path("output")

NOISE = MeasurementNoise(pos_std=2.0, load_std=3.0)


# ──── 学習 ───────────────────────────────────────────────────────────────────
def _train() -> tuple[ESNModel, np.ndarray, float, float]:
    """訓練データで ESN を学習し (model, warmup_data, thr_pos, thr_load) を返す。"""
    rng = np.random.default_rng(SEED)
    u_raw = generate_servo_periodic(TRAIN_CYCLES, noise=NOISE, rng=rng)
    u = normalize_servo_obs(u_raw)
    X_tr, y_tr = u[:-1], u[1:]

    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED)
    model = ESNModel(config)
    model.fit(X_tr, y_tr)

    pred_tr = model.predict(X_tr)
    e_pos  = np.abs(y_tr - pred_tr)[:, 0]
    e_load = np.abs(y_tr - pred_tr)[:, 1]

    s_pos  = smooth(e_pos,  window=SMOOTH_WIN)
    s_load = smooth(e_load, window=SMOOTH_WIN)

    thr_pos  = compute_threshold(s_pos[WARMUP:],  method="3sigma")
    thr_load = compute_threshold(s_load[WARMUP:], method="3sigma")

    rmse_pos  = float(np.sqrt(np.mean(e_pos[WARMUP:] ** 2)))
    rmse_load = float(np.sqrt(np.mean(e_load[WARMUP:] ** 2)))
    print(f"    訓練 RMSE  pos: {rmse_pos:.5f}  load: {rmse_load:.5f}")
    print(f"    閾値       pos: {thr_pos:.5f}  load: {thr_load:.5f}")

    _plot_train_residuals(e_pos, e_load, s_pos, s_load, thr_pos, thr_load,
                          WARMUP, OUTPUT_DIR / "result_servo_train.png")

    return model, X_tr[-WARMUP:], thr_pos, thr_load


# ──── 検証 ───────────────────────────────────────────────────────────────────
def _validate(
    model: ESNModel,
    warmup_data: np.ndarray,
    thr_pos: float,
    thr_load: float,
) -> None:
    """検証データで 1 ステップ予測し、結果をプロットする。"""
    rng = np.random.default_rng(SEED + 100)
    u_raw = generate_servo_periodic(VALID_CYCLES, noise=NOISE, rng=rng)
    u = normalize_servo_obs(u_raw)
    X_te, y_te = u[:-1], u[1:]

    pred = model.predict(X_te, warmup_data=warmup_data)

    e_pos  = np.abs(y_te - pred)[:, 0]
    e_load = np.abs(y_te - pred)[:, 1]
    s_pos  = smooth(e_pos,  window=SMOOTH_WIN)
    s_load = smooth(e_load, window=SMOOTH_WIN)

    rmse_pos  = float(np.sqrt(np.mean(e_pos[WARMUP:] ** 2)))
    rmse_load = float(np.sqrt(np.mean(e_load[WARMUP:] ** 2)))

    # 閾値超過率
    over_pos  = float((s_pos[WARMUP:]  > thr_pos).mean()  * 100)
    over_load = float((s_load[WARMUP:] > thr_load).mean() * 100)

    print(f"    検証 RMSE  pos: {rmse_pos:.5f}  load: {rmse_load:.5f}")
    print(f"    閾値超過率  pos: {over_pos:.1f}%  load: {over_load:.1f}%")

    pat = MotionPattern()
    t = np.arange(len(X_te)) * 0.01  # dt = 0.01 s
    _plot_validation(
        t=t,
        u=y_te,
        pred=pred,
        s_pos=s_pos,
        s_load=s_load,
        thr_pos=thr_pos,
        thr_load=thr_load,
        warmup=WARMUP,
        n_cycles=VALID_CYCLES,
        steps_per_cycle=pat.steps_per_cycle,
        path=OUTPUT_DIR / "result_servo_valid.png",
    )


# ──── プロット ────────────────────────────────────────────────────────────────
def _plot_train_residuals(
    e_pos: np.ndarray,
    e_load: np.ndarray,
    s_pos: np.ndarray,
    s_load: np.ndarray,
    thr_pos: float,
    thr_load: float,
    warmup: int,
    path: Path,
) -> None:
    """訓練残差の 2 段グラフを保存する。"""
    path.parent.mkdir(parents=True, exist_ok=True)
    t = np.arange(len(e_pos)) * 0.01

    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(14, 6), sharex=True)
    fig.suptitle("ESN Servo: Training Residuals", fontsize=11)

    ax1.axvspan(0, warmup * 0.01, alpha=0.1, color="gray", label="warmup")
    ax1.plot(t, e_pos, color="steelblue", linewidth=0.5, alpha=0.6, label="|pos error|")
    ax1.plot(t, s_pos, color="navy", linewidth=1.0, label="smoothed")
    ax1.axhline(thr_pos, color="crimson", linestyle="--", linewidth=1.2,
                label=f"threshold {thr_pos:.4f}")
    ax1.set_ylabel("pos residual (normalized)")
    ax1.legend(loc="upper right", fontsize=8)
    ax1.grid(True, alpha=0.3)

    ax2.axvspan(0, warmup * 0.01, alpha=0.1, color="gray", label="warmup")
    ax2.plot(t, e_load, color="darkorange", linewidth=0.5, alpha=0.6, label="|load error|")
    ax2.plot(t, s_load, color="saddlebrown", linewidth=1.0, label="smoothed")
    ax2.axhline(thr_load, color="crimson", linestyle="--", linewidth=1.2,
                label=f"threshold {thr_load:.4f}")
    ax2.set_ylabel("load residual (normalized)")
    ax2.set_xlabel("Time [s]")
    ax2.legend(loc="upper right", fontsize=8)
    ax2.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


def _plot_validation(
    t: np.ndarray,
    u: np.ndarray,
    pred: np.ndarray,
    s_pos: np.ndarray,
    s_load: np.ndarray,
    thr_pos: float,
    thr_load: float,
    warmup: int,
    n_cycles: int,
    steps_per_cycle: int,
    path: Path,
) -> None:
    """検証結果の 4 段グラフ（pos波形, load波形, pos残差, load残差）を保存する。"""
    path.parent.mkdir(parents=True, exist_ok=True)
    fig, axes = plt.subplots(4, 1, figsize=(14, 12), sharex=True)
    fig.suptitle(f"ESN Servo: Validation ({n_cycles} cycles)", fontsize=11)

    # サイクル境界を垂直線で表示
    cycle_times = [i * steps_per_cycle * 0.01 for i in range(1, n_cycles)]

    def _add_cycle_lines(ax: plt.Axes) -> None:
        for ct in cycle_times:
            ax.axvline(ct, color="gray", linewidth=0.5, alpha=0.5, linestyle=":")

    # ── 上段: pos ──
    ax = axes[0]
    ax.axvspan(0, warmup * 0.01, alpha=0.1, color="gray")
    ax.plot(t, u[:, 0], color="steelblue", linewidth=0.7, alpha=0.8, label="pos (actual)")
    ax.plot(t, pred[:, 0], color="limegreen", linewidth=0.9, linestyle="--",
            alpha=0.9, label="pos (predicted)")
    _add_cycle_lines(ax)
    ax.set_ylabel("pos (normalized)")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── 第 2 段: load ──
    ax = axes[1]
    ax.axvspan(0, warmup * 0.01, alpha=0.1, color="gray")
    ax.plot(t, u[:, 1], color="darkorange", linewidth=0.7, alpha=0.8, label="load (actual)")
    ax.plot(t, pred[:, 1], color="limegreen", linewidth=0.9, linestyle="--",
            alpha=0.9, label="load (predicted)")
    _add_cycle_lines(ax)
    ax.set_ylabel("load (normalized)")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── 第 3 段: pos 残差 ──
    ax = axes[2]
    ax.axvspan(0, warmup * 0.01, alpha=0.1, color="gray")
    ax.plot(t, s_pos, color="navy", linewidth=0.9, alpha=0.85, label="smoothed pos residual")
    ax.axhline(thr_pos, color="crimson", linestyle="--", linewidth=1.2,
               label=f"threshold {thr_pos:.4f}")
    _add_cycle_lines(ax)
    ax.set_ylabel("pos residual")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── 下段: load 残差 ──
    ax = axes[3]
    ax.axvspan(0, warmup * 0.01, alpha=0.1, color="gray")
    ax.plot(t, s_load, color="saddlebrown", linewidth=0.9, alpha=0.85,
            label="smoothed load residual")
    ax.axhline(thr_load, color="crimson", linestyle="--", linewidth=1.2,
               label=f"threshold {thr_load:.4f}")
    _add_cycle_lines(ax)
    ax.set_xlabel("Time [s]")
    ax.set_ylabel("load residual")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── 外部 API ───────────────────────────────────────────────────────────────
def build_detector() -> tuple["ESNModel", np.ndarray, float, float]:
    """学習済みモデルと検知閾値を返す（外部モジュールから利用）。

    Returns:
        (model, warmup_data, thr_pos, thr_load)
    """
    return _train()


# ──── メイン ──────────────────────────────────────────────────────────────────
def run() -> None:
    print("=" * 60)
    print("ESN サーボ位置・負荷 1 ステップ予測実験")
    print("=" * 60)

    pat = MotionPattern()
    print(f"\n[設定]")
    print(f"  訓練サイクル数: {TRAIN_CYCLES}  ({TRAIN_CYCLES * pat.steps_per_cycle} step)")
    print(f"  検証サイクル数: {VALID_CYCLES}  ({VALID_CYCLES * pat.steps_per_cycle} step)")
    print(f"  MotionPattern: home={pat.home_mrad:.1f} mrad, "
          f"up={pat.up_mrad:.1f} mrad, down={pat.down_mrad:.1f} mrad")
    print(f"  ノイズ: pos_std={NOISE.pos_std} mrad, load_std={NOISE.load_std} mA")

    print(f"\n[1] 訓練データ生成・ESN 学習 (units=200, 2ch 入出力)")
    model, warmup_data, thr_pos, thr_load = _train()

    print(f"\n[2] 検証データで 1 ステップ予測")
    _validate(model, warmup_data, thr_pos, thr_load)

    print("\n" + "=" * 60)


if __name__ == "__main__":
    run()
