"""ESN リザーバ状態による 6 モード 2ch 波形分類実験。

リザーバ（固定ランダム重み）→ Ridge 読み出し（6 次元 one-hot）の構成。
各モードを独立したセグメントで学習し、連続テスト系列で逐次分類する。

モード定義（A-B の波形対）:
  0: sin-sin   1: sin-saw   2: sin-square
  3: saw-saw   4: saw-square  5: square-square

出力:
    output/result_multimode_confusion.png       混同行列
    output/result_multimode_accuracy.png        モード別精度バーグラフ
    output/result_multimode_timeseries.png      テスト系列の逐次分類結果
    output/result_multimode_unknown.png         未知モード検出結果
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
from reservoirpy.nodes import Reservoir, Ridge

from esn_anomaly.multimode.data import (
    FREQ,
    MODE_NAMES,
    N_MODES,
    UNKNOWN_MODE_NAME,
    generate_mode_segment,
    generate_test_sequence,
    generate_train_dataset,
    generate_unknown_segment,
)

# ──── 定数 ────────────────────────────────────────────────────────────────────
FS: float = 30.0
NOISE_STD: float = 0.01
TRAIN_STEPS_PER_MODE: int = 2000
TEST_STEPS_PER_MODE: int = 1000
WARMUP: int = 100
SEED: int = 42
RIDGE_COEF: float = 1e-4
OUTPUT_DIR = Path("output")

_MODE_COLORS = [
    "#4e79a7", "#f28e2b", "#e15759",
    "#76b7b2", "#59a14f", "#edc948",
]


# ──── ユーティリティ ───────────────────────────────────────────────────────────
def _one_hot(labels: np.ndarray, n_classes: int) -> np.ndarray:
    oh = np.zeros((len(labels), n_classes), dtype=np.float64)
    oh[np.arange(len(labels)), labels] = 1.0
    return oh


def _softmax(x: np.ndarray) -> np.ndarray:
    """行方向の softmax（スコアを確率に変換する用途）。"""
    e = np.exp(x - x.max(axis=1, keepdims=True))
    return e / e.sum(axis=1, keepdims=True)


# ──── 学習 ───────────────────────────────────────────────────────────────────
def train(
    n_steps_per_mode: int = TRAIN_STEPS_PER_MODE,
) -> tuple[Reservoir, Ridge]:
    """各モードを独立セグメントで学習し (reservoir, readout) を返す。

    各モードの先頭 WARMUP ステップはリザーバ過渡状態のため除外する。
    モード間でリザーバをリセットすることで境界過渡の影響を受けない状態を収集する。
    """
    rng = np.random.default_rng(SEED)
    segments, mode_indices = generate_train_dataset(
        n_steps_per_mode, noise_std=NOISE_STD, rng=rng
    )

    reservoir = Reservoir(units=200, sr=0.9, lr=0.3, seed=SEED)
    readout = Ridge(ridge=RIDGE_COEF)
    esn = reservoir >> readout

    # リザーバの内部 state を確立するために全データで一度 fit
    X_all = np.concatenate(segments, axis=0)
    dummy_Y = _one_hot(
        np.concatenate([np.full(len(s), i) for i, s in enumerate(segments)]),
        N_MODES,
    )
    esn.fit(X_all, dummy_Y, warmup=0)

    # モードごとにリセット → 状態収集（ウォームアップ除外）
    all_states: list[np.ndarray] = []
    all_labels: list[np.ndarray] = []
    for mode_idx, seg in zip(mode_indices, segments):
        reservoir.reset()
        states = reservoir.run(seg)                              # (n, 200)
        all_states.append(states[WARMUP:])
        all_labels.append(
            np.full(n_steps_per_mode - WARMUP, mode_idx, dtype=np.int64)
        )

    X_states = np.vstack(all_states)                            # (N*n', 200)
    Y_oh = _one_hot(np.concatenate(all_labels), N_MODES)        # (N*n', 6)
    readout.fit(X_states, Y_oh)
    reservoir.reset()

    n_train = len(X_states)
    print(f"  学習: {n_train} サンプル × 6 モード → Ridge({RIDGE_COEF})")
    return reservoir, readout


# ──── 推論 ───────────────────────────────────────────────────────────────────
def classify(
    reservoir: Reservoir,
    readout: Ridge,
    X_test: np.ndarray,
    warmup_data: np.ndarray | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """連続テスト系列を逐次分類する（リザーバリセットなし）。

    Returns:
        pred_labels: shape (n,)    予測モードインデックス
        scores:      shape (n, 6)  各モードの線形スコア（softmax 前）
    """
    reservoir.reset()
    if warmup_data is not None and len(warmup_data) > 0:
        reservoir.run(warmup_data)
    states = reservoir.run(X_test)      # (n, 200)
    scores = readout.run(states)        # (n, 6)
    pred_labels = np.argmax(scores, axis=1)
    return pred_labels, scores


# ──── 評価 ───────────────────────────────────────────────────────────────────
def evaluate(
    true_labels: np.ndarray,
    pred_labels: np.ndarray,
    segments: list[tuple[int, int, int]],
    warmup: int = WARMUP,
) -> tuple[float, np.ndarray, np.ndarray]:
    """混同行列と精度を計算する（各モードブロック先頭の warmup ステップを除外）。

    Returns:
        overall_acc:  全体精度
        per_mode_acc: shape (N_MODES,) モード別精度
        confusion:    shape (N_MODES, N_MODES) 混同行列（行=正解、列=予測）
    """
    mask = np.ones(len(true_labels), dtype=bool)
    for _, start, _ in segments:
        mask[start:min(start + warmup, len(mask))] = False

    t = true_labels[mask]
    p = pred_labels[mask]

    confusion = np.zeros((N_MODES, N_MODES), dtype=np.int64)
    for ti, pi in zip(t, p):
        confusion[ti, pi] += 1

    per_mode_acc = np.array([
        confusion[i, i] / confusion[i].sum() if confusion[i].sum() > 0 else 0.0
        for i in range(N_MODES)
    ])
    overall_acc = float(np.diag(confusion).sum() / confusion.sum())
    return overall_acc, per_mode_acc, confusion


# ──── 可視化 ─────────────────────────────────────────────────────────────────
def _plot_confusion(confusion: np.ndarray, overall_acc: float) -> None:
    fig, ax = plt.subplots(figsize=(7, 6))
    im = ax.imshow(confusion, cmap="Blues")
    plt.colorbar(im, ax=ax)
    ax.set_xticks(range(N_MODES))
    ax.set_xticklabels(MODE_NAMES, rotation=45, ha="right", fontsize=9)
    ax.set_yticks(range(N_MODES))
    ax.set_yticklabels(MODE_NAMES, fontsize=9)
    ax.set_xlabel("Predicted", fontsize=10)
    ax.set_ylabel("True", fontsize=10)
    ax.set_title(f"Confusion Matrix  (overall accuracy: {overall_acc:.1%})", fontsize=11)

    row_sum = confusion.sum(axis=1, keepdims=True)
    for i in range(N_MODES):
        for j in range(N_MODES):
            pct = confusion[i, j] / row_sum[i, 0] if row_sum[i, 0] > 0 else 0.0
            ax.text(
                j, i, f"{confusion[i, j]}\n({pct:.0%})",
                ha="center", va="center", fontsize=8,
                color="white" if confusion[i, j] > confusion.max() * 0.5 else "black",
            )

    plt.tight_layout()
    path = OUTPUT_DIR / "result_multimode_confusion.png"
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


def _plot_accuracy(per_mode_acc: np.ndarray) -> None:
    fig, ax = plt.subplots(figsize=(8, 4))
    bars = ax.bar(range(N_MODES), per_mode_acc * 100,
                  color=_MODE_COLORS, edgecolor="white", width=0.6)
    ax.set_xticks(range(N_MODES))
    ax.set_xticklabels(MODE_NAMES, fontsize=9)
    ax.set_ylabel("Accuracy [%]")
    ax.set_ylim(0, 115)
    ax.set_title("Per-mode classification accuracy")
    ax.axhline(100, color="gray", lw=0.8, ls="--")
    for bar, acc in zip(bars, per_mode_acc):
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + 1,
            f"{acc:.1%}", ha="center", va="bottom", fontsize=9,
        )
    ax.grid(True, axis="y", alpha=0.3)
    plt.tight_layout()
    path = OUTPUT_DIR / "result_multimode_accuracy.png"
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


def _plot_timeseries(
    X_test: np.ndarray,
    true_labels: np.ndarray,
    pred_labels: np.ndarray,
    scores: np.ndarray,
    segments: list[tuple[int, int, int]],
) -> None:
    """4 段グラフ（入力 / 正解モード / 予測モード / 信頼度）を保存する。"""
    n = len(X_test)
    t = np.arange(n) / FS

    fig, axes = plt.subplots(4, 1, figsize=(14, 10), sharex=True)
    fig.suptitle(
        "Multi-mode classification: test sequence\n"
        "(colored bands = mode;  confidence = softmax max)",
        fontsize=10,
    )

    # Row 1: 入力波形
    ax1 = axes[0]
    ax1.plot(t, X_test[:, 0], color="steelblue", lw=0.5, alpha=0.85, label="A")
    ax1.plot(t, X_test[:, 1], color="orange", lw=0.5, alpha=0.85, label="B")
    ax1.set_ylabel("Amplitude")
    ax1.legend(loc="upper right", fontsize=8)
    ax1.grid(True, alpha=0.3)

    def _fill_mode_bands(ax: plt.Axes, label_arr: np.ndarray) -> None:
        """ラベル配列を連続するバンドに変換して塗りつぶす。"""
        prev = int(label_arr[0])
        seg_s = 0
        for i in range(1, n):
            cur = int(label_arr[i])
            if cur != prev or i == n - 1:
                end_i = i if cur != prev else i + 1
                ax.axvspan(t[seg_s], t[min(end_i - 1, n - 1)],
                           color=_MODE_COLORS[prev], alpha=0.55)
                seg_s = i
                prev = cur

    # Row 2: 正解モード
    ax2 = axes[1]
    _fill_mode_bands(ax2, true_labels)
    # 凡例パッチ
    patches = [
        plt.Rectangle((0, 0), 1, 1, color=_MODE_COLORS[i], alpha=0.55)
        for i in range(N_MODES)
    ]
    ax2.legend(patches, MODE_NAMES, loc="upper right", fontsize=7,
               ncol=3, framealpha=0.8)
    ax2.set_yticks([])
    ax2.set_ylabel("True mode")

    # Row 3: 予測モード
    ax3 = axes[2]
    _fill_mode_bands(ax3, pred_labels)
    ax3.set_yticks([])
    ax3.set_ylabel("Predicted mode")

    # Row 4: 信頼度 (softmax max)
    ax4 = axes[3]
    prob = _softmax(scores)
    confidence = prob.max(axis=1)
    ax4.plot(t, confidence, color="purple", lw=0.6, alpha=0.85)
    ax4.fill_between(t, 0, confidence, color="purple", alpha=0.2)
    ax4.axhline(1.0 / N_MODES, color="gray", ls="--", lw=0.8,
                label=f"random ({1/N_MODES:.2f})")
    ax4.set_ylim(0, 1.05)
    ax4.set_ylabel("Confidence")
    ax4.set_xlabel("Time [s]")
    ax4.legend(loc="upper right", fontsize=7)
    ax4.grid(True, alpha=0.3)

    plt.tight_layout()
    path = OUTPUT_DIR / "result_multimode_timeseries.png"
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── 未知モード検出 ──────────────────────────────────────────────────────────
def train_prediction_head(
    reservoir: Reservoir,
    segments: list[np.ndarray],
) -> Ridge:
    """全既知モードのデータで次ステップ予測 Readout を学習する。

    分類 Readout と同じリザーバを共有し、追加の Ridge 出力層のみ訓練する。
    u(t) → reservoir states(t) → 予測 u(t+1)
    学習後にリザーバをリセットする。

    Args:
        reservoir: 分類学習済みのリザーバ（状態共有・重み固定）
        segments:  generate_train_dataset() が返す各モードの信号セグメントリスト

    Returns:
        学習済み予測 Readout（Ridge）
    """
    readout_pred = Ridge(ridge=RIDGE_COEF)
    all_states: list[np.ndarray] = []
    all_targets: list[np.ndarray] = []
    for seg in segments:
        reservoir.reset()
        states = reservoir.run(seg[:-1])          # u(t) → states(t)
        all_states.append(states[WARMUP:])
        all_targets.append(seg[1:][WARMUP:])      # y(t) = u(t+1)
    readout_pred.fit(np.vstack(all_states), np.vstack(all_targets))
    reservoir.reset()
    return readout_pred


def compute_prediction_threshold(
    reservoir: Reservoir,
    readout_pred: Ridge,
    segments: list[np.ndarray],
    sigma: float = 3.0,
) -> float:
    """既知モードの B チャンネル予測残差分布から検出閾値を求める（全モード共通）。"""
    all_residuals: list[float] = []
    for seg in segments:
        reservoir.reset()
        states = reservoir.run(seg[:-1])
        pred = readout_pred.run(states)
        e_b = np.abs(seg[1:] - pred)[:, 1]
        all_residuals.extend(e_b[WARMUP:].tolist())
        reservoir.reset()
    arr = np.array(all_residuals)
    return float(arr.mean() + sigma * arr.std())


def compute_per_mode_pred_thresholds(
    reservoir: Reservoir,
    readout_pred: Ridge,
    segments: list[np.ndarray],
    sigma: float = 3.0,
) -> dict[int, float]:
    """モードごとの B チャンネル予測残差閾値を求める。

    矩形波を含むモードは残差が大きいため、全モード共通閾値では未知モード検出に
    支障が出る。分類器の予測結果に対応するモード別閾値を使うことで
    各モードの正常残差範囲内に収まる閾値を設定できる。

    Args:
        reservoir:    リザーバ
        readout_pred: 予測 Readout
        segments:     訓練セグメントリスト（generate_train_dataset の戻り値）
        sigma:        閾値の標準偏差倍率

    Returns:
        {mode_idx: threshold} の辞書
    """
    thresholds: dict[int, float] = {}
    for mode_idx, seg in enumerate(segments):
        reservoir.reset()
        states = reservoir.run(seg[:-1])
        pred = readout_pred.run(states)
        e_b = np.abs(seg[1:] - pred)[WARMUP:, 1]
        reservoir.reset()
        thresholds[mode_idx] = float(e_b.mean() + sigma * e_b.std())
    return thresholds


# _UnkSegInfo: (kind, mode_idx_or_-1, start, end)
_UnkSegInfo = tuple[str, int, int, int]


def run_unknown_detection(
    reservoir: Reservoir,
    readout_class: Ridge,
    train_segments: list[np.ndarray],
    fft_window: int = 60,
) -> None:
    """未知モード（A=sin, B=ノイズのみ）を B チャンネルのスペクトルピーク比で検出する。

    既知モードは B に 2Hz 信号あり → 目標周波数ビンのパワー比が大きい。
    未知モードは B がノイズのみ → パワーが全周波数に均等分布、比が小さい。
    この方式はノイズ振幅が信号振幅に匹敵する高ノイズ環境（noise_std=0.3）でも有効。

    noise_std=0.01（標準）と noise_std=0.3（高ノイズ）の両方でテストする。
    各ノイズレベルに対応した訓練データから閾値を再計算して使用する。
    """
    from esn_anomaly.detector import spectral_peak_ratio

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    for noise_label, noise_std_test in [("標準 (0.01)", NOISE_STD), ("高ノイズ (0.30)", 0.30)]:
        # テストと同じノイズレベルで訓練閾値を計算
        rng_thr = np.random.default_rng(SEED)
        thr_segs, _ = generate_train_dataset(
            TRAIN_STEPS_PER_MODE, noise_std=noise_std_test, rng=rng_thr
        )
        train_ratios = [
            spectral_peak_ratio(seg[:, 1], FREQ, FS, fft_window)
            for seg in thr_segs
        ]
        thr = float(np.percentile(
            np.concatenate([r[WARMUP:] for r in train_ratios]), 1
        ))

        print(f"\n  --- noise_std = {noise_label} ---")
        print(f"  SpectralPeak 閾値 (訓練データ 1 パーセンタイル): {thr:.5f}")

        rng = np.random.default_rng(SEED + 99)
        n = TEST_STEPS_PER_MODE
        known_ids = [0, 1, 4]   # sin-sin, sin-saw, saw-squ

        x_parts: list[np.ndarray] = []
        seg_info: list[_UnkSegInfo] = []
        pos = 0
        for mode_idx in known_ids:
            x_parts.append(
                generate_mode_segment(mode_idx, n, noise_std=noise_std_test, rng=rng)
            )
            seg_info.append(("known", mode_idx, pos, pos + n))
            pos += n
            x_parts.append(generate_unknown_segment(n, noise_std=noise_std_test, rng=rng))
            seg_info.append(("unknown", -1, pos, pos + n))
            pos += n

        X_seq = np.concatenate(x_parts, axis=0)
        peak_ratio = spectral_peak_ratio(X_seq[:, 1], FREQ, FS, fft_window)
        is_unknown = peak_ratio < thr

        for kind, mode_idx, start, end in seg_info:
            eval_start = start + WARMUP
            if eval_start >= end:
                continue
            seg_flags = is_unknown[eval_start:end]
            label = UNKNOWN_MODE_NAME if kind == "unknown" else MODE_NAMES[mode_idx]
            if kind == "unknown":
                rate = seg_flags.mean()
                ok = "✓" if rate >= 0.9 else "✗"
                print(f"  [unknown] {label:12s}  検出率={rate:.1%}  {ok}")
            else:
                fa = seg_flags.mean()
                ok = "✓" if fa <= 0.05 else "✗"
                print(f"  [known  ] {label:12s}  誤検知率={fa:.1%}  {ok}")

        _plot_unknown_detection(
            X_seq, peak_ratio, is_unknown, thr, seg_info, noise_label,
        )


def _plot_unknown_detection(
    X_seq: np.ndarray,
    peak_ratio: np.ndarray,
    is_unknown: np.ndarray,
    threshold: float,
    seg_info: list[_UnkSegInfo],
    noise_label: str,
) -> None:
    """未知モード検出結果を 3 段グラフで保存する。

    上段: 入力波形（A / B）＋既知/未知の背景色
    中段: B チャンネル 2Hz スペクトルピーク比＋閾値ライン
    下段: 未知フラグ（1=未知と判定、0=既知と判定）
    """
    n = len(X_seq)
    t = np.arange(n) / FS

    fig, axes = plt.subplots(3, 1, figsize=(14, 8), sharex=True)
    known_names = ", ".join(MODE_NAMES[s[1]] for s in seg_info if s[0] == "known")
    fig.suptitle(
        f"Unknown mode detection via B-channel spectral peak ratio (2 Hz)  [{noise_label}]\n"
        f"Known: {known_names}   Unknown: {UNKNOWN_MODE_NAME} (A=sin, B=noise only)\n"
        f"Threshold = 1st percentile of training peak ratio = {threshold:.4f}",
        fontsize=9,
    )

    def _bg(ax: plt.Axes) -> None:
        for kind, _, start, end in seg_info:
            color = "#d4edda" if kind == "known" else "#f8d7da"
            ax.axvspan(t[max(start, 0)], t[min(end, n) - 1],
                       color=color, alpha=0.4, zorder=0)

    # 上段: 入力波形
    ax1 = axes[0]
    _bg(ax1)
    ax1.plot(t, X_seq[:, 0], color="steelblue", lw=0.5, alpha=0.85, label="A (sin)")
    ax1.plot(t, X_seq[:, 1], color="orange", lw=0.5, alpha=0.85, label="B")
    kp = plt.Rectangle((0, 0), 1, 1, color="#d4edda", alpha=0.6)
    up = plt.Rectangle((0, 0), 1, 1, color="#f8d7da", alpha=0.6)
    handles, labels_leg = ax1.get_legend_handles_labels()
    ax1.legend(handles + [kp, up], labels_leg + ["known", "unknown"],
               loc="upper right", fontsize=7, ncol=2)
    ax1.set_ylabel("Amplitude")
    ax1.grid(True, alpha=0.2)

    # 中段: 2Hz スペクトルピーク比 + 閾値
    ax2 = axes[1]
    _bg(ax2)
    ax2.plot(t, peak_ratio, color="purple", lw=0.7, alpha=0.9,
             label="B 2Hz spectral peak ratio")
    ax2.fill_between(t, 0, peak_ratio, color="purple", alpha=0.15)
    ax2.axhline(threshold, color="crimson", ls="--", lw=1.2,
                label=f"Threshold = {threshold:.4f}")
    ax2.set_ylabel("Peak ratio")
    ax2.legend(loc="upper right", fontsize=8)
    ax2.grid(True, alpha=0.2)

    # 下段: 未知フラグ
    ax3 = axes[2]
    _bg(ax3)
    ax3.fill_between(t, 0, is_unknown.astype(float),
                     color="crimson", alpha=0.7, step="post", label="Flagged as UNKNOWN")
    ax3.set_ylim(-0.05, 1.15)
    ax3.set_yticks([0, 1])
    ax3.set_yticklabels(["Known", "Unknown"], fontsize=8)
    ax3.set_ylabel("Detection")
    ax3.set_xlabel("Time [s]")
    ax3.legend(loc="upper right", fontsize=8)
    ax3.grid(True, alpha=0.2)

    plt.tight_layout()
    suffix = noise_label.split()[0].replace("(", "").replace(")", "").replace(".", "")
    path = OUTPUT_DIR / f"result_multimode_unknown_{suffix}.png"
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── メイン ──────────────────────────────────────────────────────────────────
def run() -> None:
    print("=" * 60)
    print("ESN 6 モード 2ch 波形 分類実験")
    print("=" * 60)
    print(f"\nモード定義: {MODE_NAMES}")

    print("\n[1] ESN 学習（各モード × 2000 step、モード間リセット）")
    reservoir, readout = train()

    print("\n[2] テスト系列生成（各モード × 1000 step、シャッフル順）")
    rng_test = np.random.default_rng(SEED + 1)
    X_test, true_labels, segments = generate_test_sequence(
        TEST_STEPS_PER_MODE, shuffle_order=True, noise_std=NOISE_STD, rng=rng_test,
    )
    order_str = " → ".join(MODE_NAMES[s[0]] for s in segments)
    print(f"  モード順: {order_str}")

    print("\n[3] 推論（連続系列、リザーバリセットなし）")
    pred_labels, scores = classify(reservoir, readout, X_test)

    print("\n[4] 評価")
    overall_acc, per_mode_acc, confusion = evaluate(
        true_labels, pred_labels, segments
    )

    print(f"\n  全体精度: {overall_acc:.1%}")
    print("  モード別精度:")
    for i, (name, acc) in enumerate(zip(MODE_NAMES, per_mode_acc)):
        bar = "█" * int(acc * 20)
        print(f"    [{i}] {name:8s}: {acc:6.1%}  {bar}")

    print("\n  混同行列（行=正解、列=予測）:")
    header = "         " + "  ".join(f"{n:8s}" for n in MODE_NAMES)
    print(header)
    for i, row in enumerate(confusion):
        vals = "  ".join(f"{v:8d}" for v in row)
        print(f"  [{i}] {MODE_NAMES[i]:8s}  {vals}")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    print("\n[5] プロット保存")
    _plot_confusion(confusion, overall_acc)
    _plot_accuracy(per_mode_acc)
    _plot_timeseries(X_test, true_labels, pred_labels, scores, segments)

    print("\n[6] 未知モード検出テスト（A=sin, B=ノイズのみ）")
    rng_pred = np.random.default_rng(SEED)
    train_segs, _ = generate_train_dataset(
        TRAIN_STEPS_PER_MODE, noise_std=NOISE_STD, rng=rng_pred
    )
    run_unknown_detection(reservoir, readout, train_segs)

    print("\n" + "=" * 60)


if __name__ == "__main__":
    run()
