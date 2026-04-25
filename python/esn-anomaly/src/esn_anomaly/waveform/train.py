"""ESN 異常検知パイプラインのエントリポイント。

実行方法:
    uv run python -m esn_anomaly.train
    make run
"""

import numpy as np

from esn_anomaly.waveform.data import generate_test_data, generate_train_data
from esn_anomaly.detector import (
    compute_residual,
    compute_threshold,
    first_detection,
    smooth,
)
from esn_anomaly.model import ESNConfig, ESNModel
from esn_anomaly.visualize import plot_results

# ─── 定数 ─────────────────────────────────────────────────
FS = 30.0  # サンプリング周波数 [Hz]
FREQ = 2.0  # サイン波・矩形波の周波数 [Hz]
WARMUP = 100  # リザーバ安定化のためのウォームアップステップ数
SMOOTH_WIN = 15  # 移動平均ウィンドウ幅


def run() -> None:
    print("=" * 50)
    print("ESN 異常検知プロトタイプ")
    print("=" * 50)

    # ─── 1. データ生成 ─────────────────────────────────────
    print("\n[1] データ生成")
    rng_train = np.random.default_rng(0)
    X_tr, y_tr = generate_train_data(n_steps=2000, fs=FS, freq=FREQ, rng=rng_train)
    print(f"    訓練データ: {len(X_tr)} ステップ ({len(X_tr) / FS:.1f} s)")

    rng_test = np.random.default_rng(1)
    X_te, y_te, labels = generate_test_data(
        n_steps=1000, switch_at=500, fs=FS, freq=FREQ, rng=rng_test
    )
    print(
        f"    テストデータ: {len(X_te)} ステップ ({len(X_te) / FS:.1f} s)  切り替え: step 500 ({500 / FS:.2f} s)"  # noqa: E501
    )

    # ─── 2. ESN 学習 ──────────────────────────────────────
    print("\n[2] ESN 学習")
    config = ESNConfig(units=100, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=42)
    model = ESNModel(config)
    model.fit(X_tr, y_tr)

    pred_tr = model.predict(X_tr)
    e_tr = compute_residual(y_tr, pred_tr)
    rmse_tr = float(np.sqrt(np.mean(e_tr[WARMUP:] ** 2)))
    print(f"    訓練 RMSE (ウォームアップ除外): {rmse_tr:.6f}")

    # ─── 3. テスト推論 ─────────────────────────────────────
    print("\n[3] テスト推論")
    pred_te = model.predict(X_te, warmup_data=X_tr[-WARMUP:])
    e_te = compute_residual(y_te, pred_te)
    smoothed = smooth(e_te, window=SMOOTH_WIN)

    # ─── 4. 閾値・検知 ─────────────────────────────────────
    print("\n[4] 閾値計算と異常検知")
    threshold = compute_threshold(e_tr[WARMUP:], method="3sigma")
    print(f"    閾値 (mean + 3σ): {threshold:.6f}")

    # 評価はウォームアップ過渡期 (50 ステップ) 以降から
    EVAL_START = 50
    det_idx_offset = first_detection(smoothed[EVAL_START:], threshold)
    detection_idx = det_idx_offset + EVAL_START if det_idx_offset is not None else None

    if detection_idx is not None:
        detection_time = detection_idx / FS
        print(f"    最初の検知: step {detection_idx}  ({detection_time:.2f} s)")
        print(
            f"    切り替えから {detection_idx - 500} ステップ後 ({(detection_idx - 500) / FS:.2f} s 後)"  # noqa: E501
        )
    else:
        print("    異常は検知されませんでした")

    # ─── 5. 可視化 ─────────────────────────────────────────
    print("\n[5] グラフ生成")
    n = len(X_te)
    t = np.arange(n) / FS
    plot_results(
        t=t,
        y_real=y_te,
        y_pred=pred_te,
        smoothed_errors=smoothed,
        threshold=threshold,
        switch_at=499,  # X_te は X[:-1] なので 500ステップ目は index 499
        detection_idx=detection_idx,
        output_path="output/result.png",
        fs=FS,
    )

    print("\n完了")
    print("=" * 50)


if __name__ == "__main__":
    run()
