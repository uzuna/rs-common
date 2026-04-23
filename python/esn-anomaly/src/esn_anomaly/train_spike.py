"""スパイク単発入力の ESN 異常検知パイプライン。

不定期（0.1〜1 Hz）に発生する 1 周期スパイク（sin/saw/square）を入力とし、
sin/saw を正常、square を異常として検知する。

実行方法:
    uv run python -m esn_anomaly.train_spike
    make run-spike
"""

from pathlib import Path

import numpy as np
from matplotlib import pyplot as plt

from esn_anomaly.data_spike import (
    SCENARIO_SPIKE_MIXED_WITH_SQUARE,
    SCENARIO_SPIKE_SAW_ONLY,
    SCENARIO_SPIKE_SIN_ONLY,
    SCENARIO_SPIKE_SQUARE_ONLY,
    SpikeInfo,
    generate_spike_test_scenario,
    generate_spike_train,
)
from esn_anomaly.detector import compute_residual
from esn_anomaly.detector_spike import (
    collect_train_spike_scores,
    compute_spike_threshold,
    evaluate_spikes,
    score_summary,
)
from esn_anomaly.model import ESNConfig, ESNModel

# ─── 定数 ─────────────────────────────────────────────────
FS = 30.0
FREQ = 2.0
WARMUP = 100
SPIKE_THRESHOLD = 0.3   # スパイク検出の振幅閾値
TRAIN_STEPS = 3000
TEST_STEPS = 500
SPIKE_RATE = 0.5        # 訓練時のスパイク発生頻度 [Hz]
TEST_SPIKE_RATE = 1.0   # テスト時のスパイク発生頻度 [Hz]（多めにサンプル取得）

_SCENARIO_LABELS = {
    SCENARIO_SPIKE_SIN_ONLY: ("S1", "Sin spikes only (normal)"),
    SCENARIO_SPIKE_SAW_ONLY: ("S2", "Sawtooth spikes only (normal)"),
    SCENARIO_SPIKE_MIXED_WITH_SQUARE: ("S3", "Sin/Saw + Square spikes (anomaly expected)"),
    SCENARIO_SPIKE_SQUARE_ONLY: ("S4", "Square spikes only (all anomalous)"),
}


def _plot_scenario(
    t: np.ndarray,
    u: np.ndarray,
    errors: np.ndarray,
    spike_results: list[dict],
    true_spikes: list[SpikeInfo],
    anomaly_threshold: float,
    title: str,
    output_path: Path,
) -> None:
    """スパイク検知結果を 2 段グラフで保存する。

    上段: 入力波形（正常スパイクを緑、異常スパイクを赤でハイライト）
    下段: スパイク区間 MAE スコア（正常/異常を色分け）
    """
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(14, 6), sharex=True)
    fig.suptitle(title, fontsize=11)

    u1d = u.ravel()
    # 上段: 波形
    ax1.plot(t, u1d, color="gray", linewidth=0.6, alpha=0.7, label="Input signal")
    for r in spike_results:
        color = "red" if r["anomaly"] else "green"
        ax1.axvspan(t[r["start"]], t[min(r["end"], len(t) - 1)], alpha=0.25, color=color)
    # 凡例用ダミー
    ax1.axvspan(0, 0, alpha=0.4, color="green", label="Normal spike")
    ax1.axvspan(0, 0, alpha=0.4, color="red", label="Anomaly spike")
    ax1.set_ylabel("Amplitude")
    ax1.legend(loc="upper right", fontsize=8)
    ax1.grid(True, alpha=0.3)

    # 下段: スパイクスコア（棒グラフ）
    ax2.axhline(anomaly_threshold, color="crimson", linestyle="--", linewidth=1.2,
                label=f"Threshold {anomaly_threshold:.3f}")
    for r in spike_results:
        center_t = t[(r["start"] + r["end"]) // 2]
        color = "red" if r["anomaly"] else "green"
        ax2.bar(center_t, r["score"], width=0.3, color=color, alpha=0.8)
    ax2.set_xlabel("Time [s]")
    ax2.set_ylabel("Spike MAE score")
    ax2.legend(loc="upper right", fontsize=8)
    ax2.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(output_path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {output_path}")


def run() -> None:
    print("=" * 60)
    print("ESN スパイク異常検知")
    print("=" * 60)

    # ─── 1. 訓練データ生成（sin + saw スパイク混在）─────────
    print("\n[1] 訓練データ生成（sin + saw スパイク、rate=0.5 Hz）")
    rng_train = np.random.default_rng(0)
    u_train, train_spikes = generate_spike_train(
        TRAIN_STEPS, ["sin", "saw"], rate_hz=SPIKE_RATE, rng=rng_train
    )
    print(f"    {TRAIN_STEPS} step ({TRAIN_STEPS / FS:.1f} s)  スパイク数: {len(train_spikes)}")

    # ─── 2. ESN 学習 ──────────────────────────────────────
    print("\n[2] ESN 学習 (units=200)")
    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=42)
    model = ESNModel(config)
    X_tr, y_tr = u_train[:-1], u_train[1:]
    model.fit(X_tr, y_tr)

    pred_tr = model.predict(X_tr)
    e_tr = compute_residual(y_tr, pred_tr)
    rmse_tr = float(np.sqrt(np.mean(e_tr[WARMUP:] ** 2)))
    print(f"    訓練 RMSE (warmup除外): {rmse_tr:.4f}")

    # ─── 3. 閾値計算 ──────────────────────────────────────
    print("\n[3] 異常判定閾値算出")
    scores_tr = collect_train_spike_scores(u_train[:-1], e_tr, spike_threshold=SPIKE_THRESHOLD)
    anomaly_threshold = compute_spike_threshold(scores_tr, method="3sigma", sigma_multiplier=5.0)
    print(f"    訓練スパイク数: {len(scores_tr)}")
    arr = np.array(scores_tr)
    print(f"    スコア mean={arr.mean():.4f}  std={arr.std():.4f}  max={arr.max():.4f}")
    print(f"    異常閾値 (mean+5σ): {anomaly_threshold:.4f}")

    # ─── 4. テストシナリオ実行 ─────────────────────────────
    print("\n[4] テストシナリオ実行")
    all_pass = True
    expected_all_normal = {SCENARIO_SPIKE_SIN_ONLY, SCENARIO_SPIKE_SAW_ONLY}

    for scenario_id in [
        SCENARIO_SPIKE_SIN_ONLY,
        SCENARIO_SPIKE_SAW_ONLY,
        SCENARIO_SPIKE_MIXED_WITH_SQUARE,
        SCENARIO_SPIKE_SQUARE_ONLY,
    ]:
        tag, desc = _SCENARIO_LABELS[scenario_id]
        print(f"\n  [{tag}] {desc}")

        rng_test = np.random.default_rng(scenario_id + 100)
        u_test, true_spikes = generate_spike_test_scenario(
            scenario_id, n_steps=TEST_STEPS, rate_hz=TEST_SPIKE_RATE, rng=rng_test
        )
        pred = model.predict(u_test[:-1], warmup_data=X_tr[-WARMUP:])
        e = compute_residual(u_test[1:], pred)
        spike_results = evaluate_spikes(
            u_test[:-1], e, anomaly_threshold, spike_threshold=SPIKE_THRESHOLD
        )

        summary = score_summary(spike_results)

        total, anomaly, normal = summary["total"], summary["anomaly"], summary["normal"]
        print(f"    検出スパイク数: {total}  異常: {anomaly}  正常: {normal}")
        if scenario_id in expected_all_normal:
            ok = anomaly == 0
            status = "OK (誤検知なし)" if ok else f"FAIL (誤検知 {anomaly} 件)"
        else:
            # S3: square スパイクが検知されているか
            # S4: 全スパイクが異常
            ok = anomaly > 0
            status = f"OK (square {anomaly} 件検知)" if ok else "FAIL (square 未検知)"
        if not ok:
            all_pass = False
        print(f"    結果: {status}")

        t = np.arange(len(u_test) - 1) / FS
        _plot_scenario(
            t=t,
            u=u_test[:-1],
            errors=e,
            spike_results=spike_results,
            true_spikes=true_spikes,
            anomaly_threshold=anomaly_threshold,
            title=f"ESN Spike Detection [{tag}]: {desc}",
            output_path=Path(f"output/result_spike_{tag}.png"),
        )

    # ─── 5. サマリ ────────────────────────────────────────
    print("\n" + "=" * 60)
    print("結果:", "全テストパス" if all_pass else "一部失敗あり（パラメータ調整が必要）")


if __name__ == "__main__":
    run()
