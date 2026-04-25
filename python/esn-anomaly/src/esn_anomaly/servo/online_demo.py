"""ServoAnomalyDetector のストリーミング動作デモ（Phase 15）。

シミュレーションデータで online 検知器の動作を確認する。

実行:
    uv run python -m esn_anomaly.servo.online_demo
    make run-servo-online
"""

from __future__ import annotations

import numpy as np

from esn_anomaly.model import ESNConfig
from esn_anomaly.servo.command_test import (
    NOISE,
    SEED,
    WARMUP,
    _build_sa,
    _build_sc,
    _make_periodic_profile,
)
from esn_anomaly.servo.data import generate_servo_with_profile
from esn_anomaly.servo.online import ServoAnomalyDetector

TRAIN_CYCLES = 20


def run() -> None:
    print("=" * 60)
    print("Phase 15: ServoAnomalyDetector ストリーミングデモ")
    print("=" * 60)

    # ── 訓練データ生成 ──────────────────────────────────────────────
    print("\n[1] 訓練データ生成 & 検知器構築")
    rng_train = np.random.default_rng(SEED)
    profile_train = _make_periodic_profile(TRAIN_CYCLES)
    u_raw_train, cmd_raw_train = generate_servo_with_profile(
        profile_train, noise=NOISE, rng=rng_train
    )

    det = ServoAnomalyDetector.from_training_data(
        u_raw_train,
        cmd_raw_train,
        esn_config=ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED),
    )
    print(f"  訓練データ長: {len(u_raw_train)} ステップ")

    # ── SA 正常シナリオ ─────────────────────────────────────────────
    print("\n[2] SA 正常シナリオ（誤警報率確認）")
    rng = np.random.default_rng(SEED + 200)
    u_raw, cmd_raw, _ = _build_sa(rng)

    det.reset()
    results = []
    for i in range(len(u_raw)):
        r = det.step(cmd_raw[i], u_raw[i, 0], u_raw[i, 1])
        results.append(r)

    esn_flags = [r["esn_anomaly"] for r in results[WARMUP:]]
    phys_flags = [r["phys_anomaly"] for r in results[WARMUP:]]
    print(f"  ESN 誤警報率:    {np.mean(esn_flags) * 100:.1f}%")
    print(f"  物理 誤警報率:   {np.mean(phys_flags) * 100:.1f}%")

    # ── SC 定常外力シナリオ ────────────────────────────────────────
    print("\n[3] SC 定常外力シナリオ（検知率確認）")
    rng = np.random.default_rng(SEED + 202)
    u_raw, cmd_raw, disturbances = _build_sc(rng)

    det.reset()
    results = []
    for i in range(len(u_raw)):
        r = det.step(cmd_raw[i], u_raw[i, 0], u_raw[i, 1])
        results.append(r)

    phys_flags = np.array([r["phys_anomaly"] for r in results])
    n = len(phys_flags)
    dist_rates = []
    for d in disturbances:
        lo = max(WARMUP, d.start)
        hi = min(n, d.end)
        if lo < hi:
            dist_rates.append(phys_flags[lo:hi].mean() * 100)

    overall_rate = phys_flags[WARMUP:].mean() * 100
    dist_rate = np.mean(dist_rates) if dist_rates else float("nan")
    print(f"  物理 全体検知率:    {overall_rate:.1f}%")
    print(f"  物理 外乱区間検知率: {dist_rate:.1f}%")

    print("\n完了")


if __name__ == "__main__":
    run()
