"""SNR × ノイズ種類 スイープによるスパイク異常検知の感度検証。

SNR 4〜100 / 4 種ノイズ（白色ガウス・一様・ラプラス・ピンク）について
ESN をそれぞれ訓練し、S1〜S4 シナリオの検出精度を評価する。

実行方法:
    uv run python -m esn_anomaly.train_spike_noise
    make run-spike-noise
"""

from pathlib import Path

import numpy as np
from matplotlib import pyplot as plt
from matplotlib.colors import LinearSegmentedColormap

from esn_anomaly.data_spike import (
    NOISE_TYPES,
    SCENARIO_SPIKE_MIXED_WITH_SQUARE,
    SCENARIO_SPIKE_SAW_ONLY,
    SCENARIO_SPIKE_SIN_ONLY,
    SCENARIO_SPIKE_SQUARE_ONLY,
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
TRAIN_STEPS = 3000
TEST_STEPS = 500
SPIKE_RATE = 0.5
TEST_SPIKE_RATE = 1.0
SIGMA_MULTIPLIER = 5.0

# SNR = 信号振幅 / ノイズ標準偏差 (振幅 ≈ 1.0)
SNR_LIST: list[int] = [100, 20, 10, 4]
OUTPUT_DIR = Path("output")

_SCENARIO_IDS = [
    SCENARIO_SPIKE_SIN_ONLY,
    SCENARIO_SPIKE_SAW_ONLY,
    SCENARIO_SPIKE_MIXED_WITH_SQUARE,
    SCENARIO_SPIKE_SQUARE_ONLY,
]
_SCENARIO_TAGS = {
    SCENARIO_SPIKE_SIN_ONLY: "S1",
    SCENARIO_SPIKE_SAW_ONLY: "S2",
    SCENARIO_SPIKE_MIXED_WITH_SQUARE: "S3",
    SCENARIO_SPIKE_SQUARE_ONLY: "S4",
}
_EXPECTED_ALL_NORMAL = {SCENARIO_SPIKE_SIN_ONLY, SCENARIO_SPIKE_SAW_ONLY}


def _spike_threshold_for_noise(noise_std: float) -> float:
    """ノイズ強度に応じたスパイク振幅検出閾値を返す。

    バックグラウンドノイズが 0.3 を超えるケース（noise_std > 0.12）では
    閾値を 2.5σ に引き上げて誤検出を抑制する。
    """
    return max(0.3, 2.5 * noise_std)


def _run_one(
    noise_type: str,
    snr: int,
    seed: int = 0,
) -> dict:
    """1 条件（noise_type × SNR）で ESN を訓練し、S1〜S4 を評価する。

    Returns:
        {
          "noise_type": str,
          "snr": int,
          "noise_std": float,
          "spike_threshold": float,
          "anomaly_threshold": float,
          "train_rmse": float,
          "scenario_results": {scenario_id: {"pass": bool, "anomaly": int, "total": int}},
          "pass_count": int,       # 0〜4
        }
    """
    noise_std = 1.0 / snr
    spike_thr = _spike_threshold_for_noise(noise_std)

    # 1. 訓練データ
    rng_tr = np.random.default_rng(seed)
    u_tr, _ = generate_spike_train(
        TRAIN_STEPS, ["sin", "saw"],
        rate_hz=SPIKE_RATE,
        bg_noise_std=noise_std,
        spike_noise_std=noise_std,
        noise_type=noise_type,
        rng=rng_tr,
    )

    # 2. ESN 学習
    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=42)
    model = ESNModel(config)
    X_tr, y_tr = u_tr[:-1], u_tr[1:]
    model.fit(X_tr, y_tr)

    pred_tr = model.predict(X_tr)
    e_tr = compute_residual(y_tr, pred_tr)
    train_rmse = float(np.sqrt(np.mean(e_tr[WARMUP:] ** 2)))

    # 3. 閾値計算
    scores_tr = collect_train_spike_scores(u_tr[:-1], e_tr, spike_threshold=spike_thr)
    if len(scores_tr) == 0:
        # スパイクが 1 件も検出されなかった場合は全失敗扱い
        return {
            "noise_type": noise_type,
            "snr": snr,
            "noise_std": noise_std,
            "spike_threshold": spike_thr,
            "anomaly_threshold": float("nan"),
            "train_rmse": train_rmse,
            "scenario_results": {sid: {"pass": False, "anomaly": 0, "total": 0}
                                  for sid in _SCENARIO_IDS},
            "plot_data": {},
            "pass_count": 0,
        }
    anomaly_thr = compute_spike_threshold(
        scores_tr, method="3sigma", sigma_multiplier=SIGMA_MULTIPLIER
    )

    # 4. テストシナリオ
    scenario_results = {}
    plot_data: dict[int, dict] = {}
    pass_count = 0
    for sid in _SCENARIO_IDS:
        rng_te = np.random.default_rng(sid + 200 + seed)
        u_te, true_spikes = generate_spike_test_scenario(
            sid,
            n_steps=TEST_STEPS,
            rate_hz=TEST_SPIKE_RATE,
            noise_std=noise_std,
            noise_type=noise_type,
            rng=rng_te,
        )
        pred = model.predict(u_te[:-1], warmup_data=X_tr[-WARMUP:])
        e = compute_residual(u_te[1:], pred)
        results = evaluate_spikes(u_te[:-1], e, anomaly_thr, spike_threshold=spike_thr)
        s = score_summary(results)

        if sid in _EXPECTED_ALL_NORMAL:
            ok = s["anomaly"] == 0
        else:
            ok = s["anomaly"] > 0
        scenario_results[sid] = {"pass": ok, "anomaly": s["anomaly"], "total": s["total"]}
        plot_data[sid] = {
            "u": u_te[:-1].ravel(),
            "pred": pred.ravel(),
            "errors": e.ravel(),
            "spike_results": results,
            "true_spikes": true_spikes,
        }
        if ok:
            pass_count += 1

    return {
        "noise_type": noise_type,
        "snr": snr,
        "noise_std": noise_std,
        "spike_threshold": spike_thr,
        "anomaly_threshold": anomaly_thr,
        "train_rmse": train_rmse,
        "scenario_results": scenario_results,
        "plot_data": plot_data,
        "pass_count": pass_count,
    }


def _print_table(results: list[dict]) -> None:
    """ノイズ種類 × SNR のパス数テーブルを標準出力に表示する。"""
    noise_types = list(dict.fromkeys(r["noise_type"] for r in results))
    snr_list = list(dict.fromkeys(r["snr"] for r in results))
    tag_map = {
        SCENARIO_SPIKE_SIN_ONLY: "S1",
        SCENARIO_SPIKE_SAW_ONLY: "S2",
        SCENARIO_SPIKE_MIXED_WITH_SQUARE: "S3",
        SCENARIO_SPIKE_SQUARE_ONLY: "S4",
    }

    # インデックスを作る
    data: dict[tuple[str, int], dict] = {(r["noise_type"], r["snr"]): r for r in results}

    # ヘッダー
    snr_hdr = "  ".join(f"SNR={s:>3}" for s in snr_list)
    print(f"\n{'Noise':>12}  {snr_hdr}")
    sep = "-" * (14 + 9 * len(snr_list))
    print(sep)
    for nt in noise_types:
        row_parts = []
        for s in snr_list:
            r = data[(nt, s)]
            pc = r["pass_count"]
            cell = f"{pc}/4"
            row_parts.append(f"{cell:>7}")
        print(f"{nt:>12}  {'  '.join(row_parts)}")
    print(sep)

    # シナリオ別詳細
    print("\n--- シナリオ別詳細 (P=Pass, F=Fail) ---")
    hdr2 = "  ".join(f"SNR={s:>3}" for s in snr_list)
    print(f"\n{'':>16}  {hdr2}")
    for nt in noise_types:
        for sid in _SCENARIO_IDS:
            tag = tag_map[sid]
            row_parts = []
            for s in snr_list:
                r = data[(nt, s)]
                ok = r["scenario_results"][sid]["pass"]
                row_parts.append(f"{'P':>7}" if ok else f"{'F':>7}")
            label = f"{nt}/{tag}"
            print(f"{label:>16}  {'  '.join(row_parts)}")
        print()


def _save_heatmap(results: list[dict]) -> None:
    """パス数ヒートマップを保存する。"""
    noise_types = list(dict.fromkeys(r["noise_type"] for r in results))
    snr_list = list(dict.fromkeys(r["snr"] for r in results))
    data = {(r["noise_type"], r["snr"]): r for r in results}

    mat = np.array(
        [[data[(nt, s)]["pass_count"] for s in snr_list] for nt in noise_types],
        dtype=float,
    )

    cmap = LinearSegmentedColormap.from_list("rg", ["#d62728", "#ffaa00", "#2ca02c"], N=5)
    fig, ax = plt.subplots(figsize=(7, 4))
    im = ax.imshow(mat, vmin=0, vmax=4, cmap=cmap, aspect="auto")
    plt.colorbar(im, ax=ax, label="Pass count (0-4 scenarios)")

    ax.set_xticks(range(len(snr_list)))
    ax.set_xticklabels([f"SNR={s}" for s in snr_list])
    ax.set_yticks(range(len(noise_types)))
    ax.set_yticklabels(noise_types)
    ax.set_title("ESN Spike Detection: Pass Count by Noise Type × SNR")

    for i, nt in enumerate(noise_types):
        for j, s in enumerate(snr_list):
            pc = int(mat[i, j])
            ax.text(j, i, str(pc), ha="center", va="center", fontsize=14,
                    color="white" if pc <= 1 else "black")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    path = OUTPUT_DIR / "result_spike_noise_heatmap.png"
    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"\nSaved: {path}")


def _save_snr_comparison(results: list[dict]) -> None:
    """ノイズ種類ごとに SNR vs 閾値・RMSE の推移グラフを保存する。"""
    noise_types = list(dict.fromkeys(r["noise_type"] for r in results))
    snr_list = list(dict.fromkeys(r["snr"] for r in results))
    data = {(r["noise_type"], r["snr"]): r for r in results}

    fig, axes = plt.subplots(2, 2, figsize=(12, 8), sharex=True)
    fig.suptitle("ESN metrics vs SNR by noise type", fontsize=12)
    colors = plt.rcParams["axes.prop_cycle"].by_key()["color"]

    snr_arr = np.array(snr_list, dtype=float)

    for ax, nt, c in zip(axes.ravel(), noise_types, colors):
        rmse_arr = [data[(nt, s)]["train_rmse"] for s in snr_list]
        thr_arr = [data[(nt, s)]["anomaly_threshold"] for s in snr_list]
        pass_arr = [data[(nt, s)]["pass_count"] for s in snr_list]

        ax2 = ax.twinx()
        ax.plot(snr_arr, rmse_arr, "o-", color=c, label="Train RMSE")
        ax.plot(snr_arr, thr_arr, "s--", color=c, alpha=0.6, label="Anomaly thr")
        ax2.bar(snr_arr, pass_arr, width=snr_arr * 0.15, alpha=0.25, color=c, label="Pass count")
        ax.set_title(nt)
        ax.set_xlabel("SNR")
        ax.set_ylabel("RMSE / Threshold")
        ax2.set_ylabel("Pass count")
        ax2.set_ylim(0, 4.5)
        ax.set_xscale("log")
        ax.set_xticks(snr_arr)
        ax.set_xticklabels([str(s) for s in snr_list])
        ax.legend(loc="upper left", fontsize=7)
        ax.grid(True, alpha=0.3)

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    path = OUTPUT_DIR / "result_spike_noise_metrics.png"
    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"Saved: {path}")


def _plot_wave_on_ax(
    ax: plt.Axes,
    u: np.ndarray,
    spike_results: list[dict],
    anomaly_threshold: float,
    fs: float,
    title: str,
    *,
    pred: np.ndarray | None = None,
    show_xlabel: bool = False,
) -> None:
    """1 サブプロットに波形（＋オプションで予測波形）とスパイク領域ハイライトを描画する。"""
    t = np.arange(len(u)) / fs
    ax.plot(t, u, color="steelblue", linewidth=0.6, alpha=0.8, label="Input")
    if pred is not None:
        ax.plot(t, pred, color="orange", linewidth=0.8, alpha=0.85, label="Prediction")
        ax.legend(loc="upper right", fontsize=6)
    for r in spike_results:
        color = "red" if r["anomaly"] else "limegreen"
        ax.axvspan(
            t[r["start"]],
            t[min(r["end"], len(t) - 1)],
            alpha=0.25,
            color=color,
        )
    ax.set_title(title, fontsize=8)
    ax.set_ylabel("Amplitude", fontsize=7)
    if show_xlabel:
        ax.set_xlabel("Time [s]", fontsize=7)
    ax.tick_params(labelsize=6)
    ax.grid(True, alpha=0.25)


def _plot_residual_on_ax(
    ax: plt.Axes,
    errors: np.ndarray,
    spike_results: list[dict],
    fs: float,
    *,
    show_xlabel: bool = False,
) -> None:
    """1 サブプロットに残差（|u - pred|）の時系列を描画する。"""
    t = np.arange(len(errors)) / fs
    ax.plot(t, errors, color="dimgray", linewidth=0.6, alpha=0.8)
    for r in spike_results:
        color = "red" if r["anomaly"] else "limegreen"
        ax.axvspan(
            t[r["start"]],
            t[min(r["end"], len(t) - 1)],
            alpha=0.25,
            color=color,
        )
    ax.set_ylabel("Residual", fontsize=7)
    if show_xlabel:
        ax.set_xlabel("Time [s]", fontsize=7)
    ax.tick_params(labelsize=6)
    ax.grid(True, alpha=0.25)


def _plot_mae_on_ax(
    ax: plt.Axes,
    u: np.ndarray,
    spike_results: list[dict],
    anomaly_threshold: float,
    fs: float,
    *,
    show_xlabel: bool = False,
) -> None:
    """1 サブプロットにスパイク MAE バーチャートを描画する。"""
    t = np.arange(len(u)) / fs
    ax.axhline(
        anomaly_threshold,
        color="crimson",
        linestyle="--",
        linewidth=1.0,
        label=f"Thr {anomaly_threshold:.2f}",
    )
    for r in spike_results:
        center = t[(r["start"] + r["end"]) // 2]
        color = "red" if r["anomaly"] else "limegreen"
        ax.bar(center, r["score"], width=0.25, color=color, alpha=0.85)
    ax.set_ylabel("MAE", fontsize=7)
    if show_xlabel:
        ax.set_xlabel("Time [s]", fontsize=7)
    ax.legend(loc="upper right", fontsize=6)
    ax.tick_params(labelsize=6)
    ax.grid(True, alpha=0.25)


def _save_waveform_overview(results: list[dict]) -> None:
    """S3 シナリオの波形を noise_type × SNR の 4×4 グリッドで保存する。

    rows = noise_type, cols = SNR
    """
    noise_types = list(dict.fromkeys(r["noise_type"] for r in results))
    snr_list = list(dict.fromkeys(r["snr"] for r in results))
    data = {(r["noise_type"], r["snr"]): r for r in results}

    n_rows, n_cols = len(noise_types), len(snr_list)
    fig, axes = plt.subplots(n_rows, n_cols, figsize=(4 * n_cols, 3 * n_rows))
    fig.suptitle("S3 Waveform: noise type × SNR  (green=normal, red=anomaly)", fontsize=11)

    for i, nt in enumerate(noise_types):
        for j, snr in enumerate(snr_list):
            ax = axes[i][j]
            r = data[(nt, snr)]
            pd = r["plot_data"].get(SCENARIO_SPIKE_MIXED_WITH_SQUARE)
            if pd is None:
                ax.set_visible(False)
                continue
            ok = r["scenario_results"][SCENARIO_SPIKE_MIXED_WITH_SQUARE]["pass"]
            status = "PASS" if ok else "FAIL"
            _plot_wave_on_ax(
                ax,
                pd["u"],
                pd["spike_results"],
                r["anomaly_threshold"],
                FS,
                title=f"{nt} / SNR={snr} [{status}]",
                pred=pd.get("pred"),
                show_xlabel=(i == n_rows - 1),
            )

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    path = OUTPUT_DIR / "result_spike_noise_waveforms.png"
    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"Saved: {path}")


def _save_waveform_detail(results: list[dict]) -> None:
    """ノイズ種類ごとに全 SNR × 全シナリオの詳細波形+残差+MAE を保存する。

    ファイル: result_spike_noise_waveform_{noise_type}.png
    レイアウト: rows = SNR (上=入力+予測, 中=残差, 下=MAE) × cols = S1〜S4
    """
    noise_types = list(dict.fromkeys(r["noise_type"] for r in results))
    snr_list = list(dict.fromkeys(r["snr"] for r in results))
    data = {(r["noise_type"], r["snr"]): r for r in results}

    _s_labels = {
        SCENARIO_SPIKE_SIN_ONLY: "S1 sin-only",
        SCENARIO_SPIKE_SAW_ONLY: "S2 saw-only",
        SCENARIO_SPIKE_MIXED_WITH_SQUARE: "S3 mixed+sq",
        SCENARIO_SPIKE_SQUARE_ONLY: "S4 sq-only",
    }

    for nt in noise_types:
        n_snr = len(snr_list)
        n_sc = len(_SCENARIO_IDS)
        # 各 SNR ごとに 3 行（波形+予測 / 残差 / MAE）× n_sc 列
        fig, axes = plt.subplots(
            n_snr * 3, n_sc,
            figsize=(4 * n_sc, 2.2 * n_snr * 3),
            squeeze=False,
        )
        fig.suptitle(
            f"Waveform detail — noise: {nt}  (green=normal, red=anomaly)\n"
            "rows per SNR: [Input+Pred | Residual | MAE score]",
            fontsize=10,
        )

        for i, snr in enumerate(snr_list):
            r = data[(nt, snr)]
            row_wave = i * 3
            row_res  = i * 3 + 1
            row_mae  = i * 3 + 2
            for j, sid in enumerate(_SCENARIO_IDS):
                ax_w = axes[row_wave][j]
                ax_r = axes[row_res][j]
                ax_m = axes[row_mae][j]
                pd = r["plot_data"].get(sid)
                if pd is None:
                    for ax in (ax_w, ax_r, ax_m):
                        ax.set_visible(False)
                    continue
                ok = r["scenario_results"][sid]["pass"]
                status = "PASS" if ok else "FAIL"
                title = (
                    f"{_s_labels[sid]}  SNR={snr} [{status}]"
                    if i == 0
                    else f"SNR={snr} [{status}]"
                )
                _plot_wave_on_ax(
                    ax_w, pd["u"], pd["spike_results"],
                    r["anomaly_threshold"], FS,
                    title=title,
                    pred=pd.get("pred"),
                )
                _plot_residual_on_ax(
                    ax_r, pd["errors"], pd["spike_results"], FS,
                )
                _plot_mae_on_ax(
                    ax_m, pd["u"], pd["spike_results"],
                    r["anomaly_threshold"], FS,
                    show_xlabel=(i == n_snr - 1),
                )
                if j == 0:
                    ax_w.set_ylabel(f"SNR={snr}\nAmplitude", fontsize=7)
                    ax_r.set_ylabel(f"SNR={snr}\nResidual", fontsize=7)

        OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
        path = OUTPUT_DIR / f"result_spike_noise_waveform_{nt}.png"
        plt.tight_layout()
        plt.savefig(path, dpi=120)
        plt.close(fig)
        print(f"Saved: {path}")


def run() -> None:
    print("=" * 60)
    print("ESN スパイク異常検知 — SNR × ノイズ種類スイープ")
    print(f"  ノイズ種類: {list(NOISE_TYPES)}")
    print(f"  SNR リスト: {SNR_LIST}")
    print("=" * 60)

    results: list[dict] = []
    total = len(NOISE_TYPES) * len(SNR_LIST)
    done = 0
    for noise_type in NOISE_TYPES:
        for snr in SNR_LIST:
            done += 1
            noise_std = 1.0 / snr
            print(f"\n[{done:2d}/{total}] noise={noise_type}  SNR={snr:3d}  "
                  f"noise_std={noise_std:.3f}", end="  ")
            r = _run_one(noise_type, snr)
            tag_map = {
                SCENARIO_SPIKE_SIN_ONLY: "S1",
                SCENARIO_SPIKE_SAW_ONLY: "S2",
                SCENARIO_SPIKE_MIXED_WITH_SQUARE: "S3",
                SCENARIO_SPIKE_SQUARE_ONLY: "S4",
            }
            detail = " ".join(
                f"{tag_map[sid]}={'O' if v['pass'] else 'X'}"
                for sid, v in r["scenario_results"].items()
            )
            print(f"pass={r['pass_count']}/4  [{detail}]  "
                  f"thr={r['anomaly_threshold']:.3f}  rmse={r['train_rmse']:.4f}")
            results.append(r)

    _print_table(results)
    _save_heatmap(results)
    _save_snr_comparison(results)
    _save_waveform_overview(results)
    _save_waveform_detail(results)

    # 全 vs 失敗数
    total_tests = len(results) * 4
    total_pass = sum(r["pass_count"] for r in results)
    print(f"\n総合: {total_pass}/{total_tests} シナリオパス")


if __name__ == "__main__":
    run()
