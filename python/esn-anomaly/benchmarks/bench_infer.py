"""推論性能ベンチマーク。

ESN の 1 ステップ推論時間とバッチスループットを計測する。
30 Hz リアルタイム動作（1 step < 33.3 ms）の可否を判定する。

実行:
    uv run python benchmarks/bench_infer.py
    make bench
"""

import sys
import time

# src/ を sys.path に追加（パッケージインストール済みでなくても動作するように）
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).parent.parent / "src"))

from esn_anomaly.waveform.data import generate_train_data
from esn_anomaly.model import ESNConfig, ESNModel

REALTIME_BUDGET_MS = 1000.0 / 30.0  # 33.33 ms（30 Hz 予算）


def build_and_train_model() -> ESNModel:
    rng = np.random.default_rng(0)
    X_tr, y_tr = generate_train_data(n_steps=2000, rng=rng)
    model = ESNModel(ESNConfig(units=100, seed=42))
    model.fit(X_tr, y_tr)
    return model


def bench_batch(
    model: ESNModel,
    batch_size: int,
    n_repeat: int = 500,
) -> tuple[float, float]:
    """指定バッチサイズの推論時間を計測する。

    Args:
        model: 学習済み ESNModel
        batch_size: 1 回ごとの推論ステップ数
        n_repeat: 繰り返し回数

    Returns:
        (mean_ms, std_ms): 1バッチあたりの平均・標準偏差 [ms]
    """
    X = np.random.randn(batch_size, 1)
    warmup_data = np.random.randn(100, 1)

    # ウォームアップ実行（JIT 等の安定化）
    for _ in range(10):
        model.predict(X, warmup_data=warmup_data)

    # 計測
    times = []
    for _ in range(n_repeat):
        t0 = time.perf_counter()
        model.predict(X, warmup_data=warmup_data)
        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)  # ms

    arr = np.array(times)
    return float(arr.mean()), float(arr.std())


def main() -> None:
    print("=" * 60)
    print("ESN Inference Benchmark")
    print(f"Realtime budget (30 Hz): {REALTIME_BUDGET_MS:.2f} ms/step")
    print("=" * 60)

    print("\nBuilding and training ESN (units=100)...")
    model = build_and_train_model()
    print("Done.\n")

    batch_sizes = [1, 10, 100, 1000]
    print(
        f"{'Batch':>8}  {'Mean [ms]':>12}  {'Std [ms]':>10}  {'Steps/sec':>12}  {'Realtime?':>10}"
    )
    print("-" * 60)

    for bs in batch_sizes:
        mean_ms, std_ms = bench_batch(model, batch_size=bs)
        steps_per_sec = bs / (mean_ms / 1000.0)
        ms_per_step = mean_ms / bs
        realtime_ok = "OK" if ms_per_step < REALTIME_BUDGET_MS else "NG"
        print(
            f"{bs:>8}  {mean_ms:>12.3f}  {std_ms:>10.3f}  {steps_per_sec:>12.1f}  {realtime_ok:>10}"
        )

    print()
    # 1ステップおよびバッチ1での単体計測を追加表示
    mean_1, std_1 = bench_batch(model, batch_size=1, n_repeat=1000)
    ms_per_step = mean_1
    print(f"Single-step latency: {ms_per_step:.3f} ± {std_1:.3f} ms")
    verdict = "PASS" if ms_per_step < REALTIME_BUDGET_MS else "FAIL"
    print(f"30 Hz realtime feasibility: {verdict} (budget={REALTIME_BUDGET_MS:.2f} ms)")
    print("=" * 60)


if __name__ == "__main__":
    main()
