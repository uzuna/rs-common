"""マルチ波形 ESN 異常検知パイプライン（混合学習）のエントリポイント。

訓練データにサイン波とノコギリ波を交互に混在させ、
矩形波のみを「異常」として検知する。

実行方法:
    uv run python -m esn_anomaly.train_multi
    make run-multi
"""

import numpy as np

from esn_anomaly.waveform.data import (
    SCENARIO_SAWTOOTH_ONLY,
    SCENARIO_SAWTOOTH_TO_SQUARE,
    SCENARIO_SINE_ONLY,
    SCENARIO_SINE_SAWTOOTH_SINE,
    SCENARIO_SINE_TO_SQUARE,
    generate_mixed_train_data,
    generate_test_scenario,
)
from esn_anomaly.detector import (
    compute_residual,
    compute_threshold,
    first_persistent_detection,
    smooth,
)
from esn_anomaly.model import ESNConfig, ESNModel
from esn_anomaly.visualize import plot_scenario

# ─── 定数 ─────────────────────────────────────────────────
FS = 30.0       # サンプリング周波数 [Hz]
FREQ = 2.0      # 波形周波数 [Hz]
WARMUP = 100    # ウォームアップステップ数
SMOOTH_WIN = 30  # 移動平均ウィンドウ幅（切り替え過渡誤差を吸収）
MIN_DURATION = 60  # 持続異常の最小連続ステップ数（= 2 s @ 30 Hz）
EVAL_START = 50  # 評価開始ステップ（過渡状態を除外）

_SCENARIO_LABELS = {
    SCENARIO_SINE_ONLY: ("T1", "Sine only (normal)"),
    SCENARIO_SAWTOOTH_ONLY: ("T2", "Sawtooth only (normal)"),
    SCENARIO_SINE_TO_SQUARE: ("T3", "Sine -> Square (anomaly expected)"),
    SCENARIO_SAWTOOTH_TO_SQUARE: ("T4", "Sawtooth -> Square (anomaly expected)"),
    SCENARIO_SINE_SAWTOOTH_SINE: ("T5", "Sine -> Sawtooth -> Sine (normal switch)"),
}

_SCENARIO_SWITCH_INDICES = {
    SCENARIO_SINE_ONLY: [],
    SCENARIO_SAWTOOTH_ONLY: [],
    SCENARIO_SINE_TO_SQUARE: [499],   # 500ステップ目（X[:-1] なので -1）
    SCENARIO_SAWTOOTH_TO_SQUARE: [499],
    SCENARIO_SINE_SAWTOOTH_SINE: [332, 665],  # 333, 666 ステップ目
}


def run() -> None:
    print("=" * 60)
    print("ESN 異常検知 - マルチ波形（混合学習）")
    print("=" * 60)

    # ─── 1. 混合訓練データ生成 ─────────────────────────────
    print("\n[1] 混合訓練データ生成（サイン波 + ノコギリ波）")
    rng_train = np.random.default_rng(0)
    X_tr, y_tr = generate_mixed_train_data(
        n_steps=2000, segment_len=250, fs=FS, freq=FREQ, rng=rng_train
    )
    print(f"    訓練データ: {len(X_tr)} ステップ ({len(X_tr) / FS:.1f} s)")
    print("    構成: [sine×250, sawtooth×250] × 4 = 2000 step")

    # ─── 2. ESN 学習（units=200） ──────────────────────────
    print("\n[2] ESN 学習 (units=200)")
    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=42)
    model = ESNModel(config)
    model.fit(X_tr, y_tr)

    pred_tr = model.predict(X_tr)
    e_tr = compute_residual(y_tr, pred_tr)
    rmse_tr = float(np.sqrt(np.mean(e_tr[WARMUP:] ** 2)))
    print(f"    訓練 RMSE (ウォームアップ除外): {rmse_tr:.6f}")

    # ─── 3. 閾値計算 ──────────────────────────────────────
    threshold = compute_threshold(e_tr[WARMUP:], method="3sigma")
    print(f"    閾値 (mean + 3σ): {threshold:.6f}")

    # ─── 4. テストシナリオ実行 ─────────────────────────────
    print("\n[3] テストシナリオ実行")
    print(f"    平滑化窓: {SMOOTH_WIN}  持続判定: {MIN_DURATION} step ({MIN_DURATION / FS:.1f} s)")
    print()

    results = {}
    for scenario_id in [
        SCENARIO_SINE_ONLY,
        SCENARIO_SAWTOOTH_ONLY,
        SCENARIO_SINE_TO_SQUARE,
        SCENARIO_SAWTOOTH_TO_SQUARE,
        SCENARIO_SINE_SAWTOOTH_SINE,
    ]:
        tag, desc = _SCENARIO_LABELS[scenario_id]
        print(f"  [{tag}] {desc}")

        rng_test = np.random.default_rng(scenario_id)
        X_te, y_te, _labels = generate_test_scenario(
            scenario_id, n_steps=1000, fs=FS, freq=FREQ, rng=rng_test
        )

        pred_te = model.predict(X_te, warmup_data=X_tr[-WARMUP:])
        e_te = compute_residual(y_te, pred_te)
        smoothed = smooth(e_te, window=SMOOTH_WIN)

        det_idx = first_persistent_detection(
            smoothed[EVAL_START:], threshold, min_duration=MIN_DURATION
        )
        detection_idx = det_idx + EVAL_START if det_idx is not None else None

        if detection_idx is not None:
            t_det = detection_idx / FS
            print(f"        検知: step {detection_idx} ({t_det:.2f} s)")
        else:
            print("        検知なし")

        results[tag] = detection_idx

        # ─── 可視化 ───────────────────────────────────────
        n = len(X_te)
        t = np.arange(n) / FS
        switch_indices = _SCENARIO_SWITCH_INDICES[scenario_id]
        plot_scenario(
            t=t,
            y_real=y_te,
            y_pred=pred_te,
            smoothed_errors=smoothed,
            threshold=threshold,
            switch_indices=switch_indices,
            detection_idx=detection_idx,
            title=f"ESN Multi-waveform [{tag}]: {desc}",
            output_path=f"output/result_multi_{tag}.png",
        )
        print()

    # ─── 5. サマリ ────────────────────────────────────────
    print("=" * 60)
    print("結果サマリ")
    print("=" * 60)
    expected_no_anomaly = {"T1", "T2", "T5"}
    all_pass = True
    for tag, det in results.items():
        if tag in expected_no_anomaly:
            status = "OK (no anomaly)" if det is None else f"FAIL (false positive at step {det})"
            if det is not None:
                all_pass = False
        else:
            if det is not None:
                status = f"OK (detected at step {det}, {det / FS:.2f}s)"
            else:
                status = "FAIL (missed)"
                all_pass = False
        print(f"  {tag}: {status}")
    print()
    print("結果:", "全テストパス" if all_pass else "一部失敗あり（パラメータ調整が必要）")


if __name__ == "__main__":
    run()
