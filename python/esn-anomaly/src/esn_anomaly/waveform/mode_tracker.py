"""オンラインモード追従（Phase 17）。

波形モードが動的に切り替わる系に対して、ESN モデルのライブラリを使い
リアルタイムでモードを追跡する。未知モードの検出にも対応する。

実行方法:
    uv run python -m esn_anomaly.waveform.mode_tracker
    make run-mode-tracker
"""

from __future__ import annotations

import argparse
from collections import deque
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from esn_anomaly.model import ESNConfig, ESNModel

# 移動平均ウィンドウ幅（閾値計算用）
_MOVING_AVG_WINDOW = 200


@dataclass
class ModeTrackerState:
    """ModeTracker の 1 ステップ出力。"""

    mode: str | None
    residual: float
    is_anomaly: bool
    mode_changed: bool
    steps_since_switch: int
    trigger_type: str = ""  # "fast" | "slow" | ""


class ModeLibrary:
    """既知モードの ESN モデルライブラリ。

    各モードに対応した ESN を学習し、トーナメント照合で現在のモードを推定する。

    Attributes:
        models: モード名 → ESNModel のマップ
        thresholds: select_mode 用の閾値（moving200 の 5× 上限）
        mode_thresholds: ModeTracker 用のモード変化検知閾値（moving200 の 3× 上限）
    """

    def __init__(self) -> None:
        self.models: dict[str, ESNModel] = {}
        self.thresholds: dict[str, float] = {}
        self.mode_thresholds: dict[str, float] = {}

    def add_mode(
        self,
        name: str,
        train_data: NDArray[np.float64],
        config: ESNConfig | None = None,
        select_multiplier: float = 5.0,
        change_multiplier: float = 3.0,
    ) -> None:
        """モードを学習してライブラリに追加する。

        Args:
            name: モード名（例: "sine", "sawtooth"）
            train_data: 1D または shape (n, 1) の訓練波形データ
            config: ESNConfig（省略時はデフォルト）
            select_multiplier: select_mode 閾値 = moving200_max × select_multiplier
            change_multiplier: thr_mode_change = moving200_max × change_multiplier
        """
        cfg = config or ESNConfig()
        data = np.asarray(train_data, dtype=np.float64).reshape(-1, 1)
        X = data[:-1]
        y = data[1:]

        esn = ESNModel(cfg)
        esn.fit(X, y)

        # ウォームアップ後の残差を使って閾値計算
        pred = esn.predict(X)
        raw_res = np.abs(y - pred).ravel()[cfg.warmup:]

        # 移動平均残差の最大値でロバストに閾値設定
        if len(raw_res) >= _MOVING_AVG_WINDOW:
            moving = np.convolve(
                raw_res, np.ones(_MOVING_AVG_WINDOW) / _MOVING_AVG_WINDOW, mode="valid"
            )
        else:
            moving = raw_res

        moving_max = float(np.max(moving))
        self.models[name] = esn
        self.thresholds[name] = moving_max * select_multiplier
        self.mode_thresholds[name] = moving_max * change_multiplier

    def select_mode(
        self,
        window: NDArray[np.float64],
        n_trial: int = 100,
    ) -> tuple[str | None, float]:
        """ライブラリ照合（トーナメント）で最適モードを選択する。

        window の末尾から n_trial ステップを各モデルで評価する。
        window の先頭部分はウォームアップデータとして使い、
        リザーバを安定化した上で残差を計算する。

        Args:
            window: 直近のシグナルウィンドウ（n_trial+1 ステップ以上推奨）
            n_trial: 照合に使用するステップ数

        Returns:
            (mode_name, min_residual): mode_name=None → 未知モード
        """
        data = np.asarray(window, dtype=np.float64).reshape(-1, 1)
        if len(data) < 2:
            return None, float("inf")

        if not self.models:
            return None, float("inf")

        # 末尾 n_trial+1 を評価区間、先頭をウォームアップ
        if len(data) > n_trial + 1:
            warmup_data = data[:-n_trial - 1]
            X = data[-n_trial - 1 : -1]
            y_true = data[-n_trial:]
        else:
            warmup_data = None
            X = data[:-1]
            y_true = data[1:]

        residuals: dict[str, float] = {}
        for name, model in self.models.items():
            y_pred = model.predict(X, warmup_data=warmup_data)
            residuals[name] = float(np.mean(np.abs(y_true - y_pred)))

        best = min(residuals, key=residuals.__getitem__)
        min_res = residuals[best]

        if min_res <= self.thresholds[best]:
            return best, min_res
        return None, min_res


class ModeTracker:
    """オンラインモード追従器。

    ESN モデルライブラリを使い、ストリーミングデータのモードをオンラインで追跡する。
    各ステップで現在のモデルの残差（1 ステップ先予測誤差）を監視し、
    移動平均残差が thr_mode_change を k_consecutive ステップ連続超過したとき
    ライブラリ照合を起動して既知モードなら切替、未知モードなら UNKNOWN をフラグする。

    残差計算: step(x[t]) では前ステップの予測 run_step(x[t-1]) と x[t] の差を使う。
    これにより 1 ステップ先予測誤差 |x[t] - predicted_x[t]| が正確に得られる。

    Usage::
        library = ModeLibrary()
        library.add_mode("sine", sine_train_data)
        library.add_mode("sawtooth", sawtooth_train_data)

        tracker = ModeTracker(library, initial_mode="sine")
        for x in stream:
            state = tracker.step(x)
            if state.mode_changed:
                print(f"Mode -> {state.mode}")
    """

    def __init__(
        self,
        library: ModeLibrary,
        initial_mode: str,
        thr_mode_change: float | None = None,
        k_consecutive: int = 300,
        min_stable_steps: int = 500,
        moving_avg_window: int = 200,
        n_trial: int = 100,
        k_consecutive_fast: int = 50,
        thr_fast_multiplier: float = 10.0,
        k_consecutive_ratio: int = 0,
        ratio_threshold: float = 0.3,
        ratio_eval_interval: int = 5,
        n_trial_ratio: int = 30,
        warmup_ratio: int = 30,
    ) -> None:
        """
        Args:
            library: ModeLibrary
            initial_mode: 最初のモード名（library に存在すること）
            thr_mode_change: 移動平均残差のモード変化検知閾値。
                             省略時は library.mode_thresholds[initial_mode] を使用。
            k_consecutive: 閾値超過が何ステップ連続したらライブラリ照合を起動するか（低速）
            min_stable_steps: 前回切替から何ステップ経過しないと再検知しないか
            moving_avg_window: 移動平均ウィンドウ幅
            n_trial: ライブラリ照合に使うウィンドウ幅
            k_consecutive_fast: 高速トリガーの連続超過ステップ数（thr × multiplier を k_fast 継続）
            thr_fast_multiplier: 高速トリガーの閾値倍率（thr_mode_change × multiplier）
            k_consecutive_ratio: 比率トリガーの連続成立回数（各評価は ratio_eval_interval ステップ間隔）
            ratio_threshold: 比率トリガーの閾値（best_alt_res / current_res < ratio で成立）
            ratio_eval_interval: 比率評価をスキップするステップ間隔（コスト低減）
            n_trial_ratio: 比率評価の評価窓幅（短いほど早く検知、デフォルト 30）
            warmup_ratio: 比率評価後の現在モデル状態復元に使う短いウォームアップ長
                          lr=0.3 では 30 ステップで初期状態の影響が 0.5% 未満に収束
        """
        if initial_mode not in library.models:
            raise ValueError(f"initial_mode '{initial_mode}' が library にありません")

        self._library = library
        self._current_mode: str | None = initial_mode
        self._current_model: ESNModel = library.models[initial_mode]
        self._current_model.reset()

        if thr_mode_change is None:
            thr_mode_change = library.mode_thresholds.get(initial_mode, 0.1)
        self._thr_mode_change = thr_mode_change
        self._k_consecutive = k_consecutive
        self._min_stable_steps = min_stable_steps
        self._moving_avg_window = moving_avg_window
        self._n_trial = n_trial
        self._k_consecutive_fast = k_consecutive_fast
        self._thr_fast_multiplier = thr_fast_multiplier
        self._k_consecutive_ratio = k_consecutive_ratio
        self._ratio_threshold = ratio_threshold
        self._ratio_eval_interval = ratio_eval_interval
        self._n_trial_ratio = n_trial_ratio
        self._warmup_ratio = warmup_ratio

        buf_size = n_trial + moving_avg_window + 10
        self._res_buf: deque[float] = deque(maxlen=moving_avg_window)
        self._data_buf: deque[float] = deque(maxlen=buf_size)
        self._consecutive_high: int = 0
        self._consecutive_fast: int = 0
        self._consecutive_ratio: int = 0
        self._steps_since_eval: int = 0
        # 0 から始めてカウントアップ: 最初の min_stable_steps は検知しない (warmup 保護)
        self._steps_since_switch: int = 0
        # 1 ステップ先予測を保持（次ステップの残差計算に使用）
        self._prev_pred: NDArray[np.float64] | None = None

    def step(self, x: float | NDArray[np.float64]) -> ModeTrackerState:
        """1 ステップ処理。

        現在のモデルのみ run_step で追跡し、定期バッチ評価で代替モデルとの
        比率を算出する 3 段階トリガー（ratio / fast / slow）でモード変化を検知する。

        Args:
            x: スカラー値または shape (1,) / (1, 1) の入力サンプル

        Returns:
            ModeTrackerState
        """
        x_scalar = float(np.asarray(x).ravel()[0])
        self._data_buf.append(x_scalar)

        # ── 現在モデルの残差（前ステップの予測 vs 現在の観測値）──
        if self._prev_pred is not None:
            residual = float(abs(x_scalar - float(self._prev_pred.ravel()[0])))
        else:
            residual = 0.0

        # 現在モデルのみ 1 ステップ前進
        x_in = np.array([x_scalar], dtype=np.float64)
        self._prev_pred = self._current_model.run_step(x_in)

        # ── 移動平均残差 + 高速/低速カウンタ ──
        self._res_buf.append(residual)
        moving_avg = float(np.mean(self._res_buf))
        is_high_slow = moving_avg > self._thr_mode_change
        is_high_fast = moving_avg > self._thr_mode_change * self._thr_fast_multiplier

        if is_high_fast:
            self._consecutive_fast += 1
        else:
            self._consecutive_fast = 0

        if is_high_slow:
            self._consecutive_high += 1
        else:
            self._consecutive_high = 0

        # ── 比率トリガー: 定期バッチ評価（k_consecutive_ratio > 0 のときのみ実行）──
        if self._k_consecutive_ratio > 0:
            self._steps_since_eval += 1
            if self._steps_since_eval >= self._ratio_eval_interval:
                self._consecutive_ratio = self._eval_ratio_counter()
                self._steps_since_eval = 0

        # ── トリガー判定 ──
        mode_changed = False
        trigger_type = ""
        eligible = self._steps_since_switch >= self._min_stable_steps

        ratio_fire = (
            self._k_consecutive_ratio > 0
            and eligible
            and self._consecutive_ratio >= self._k_consecutive_ratio
        )
        fast_fire = eligible and self._consecutive_fast >= self._k_consecutive_fast
        slow_fire = eligible and self._consecutive_high >= self._k_consecutive

        if ratio_fire or fast_fire or slow_fire:
            trigger_type = "ratio" if ratio_fire else ("fast" if fast_fire else "slow")
            window = np.array(list(self._data_buf), dtype=np.float64)

            if ratio_fire:
                # ratio トリガーは短い窓で評価済みなのでそのまま short_window を使う
                # (長い n_trial 窓は混合データで None を返しやすいため)
                _total_needed = self._warmup_ratio + self._n_trial_ratio + 1
                _sw = window[-_total_needed:] if len(window) >= _total_needed else window
                new_mode, _ = self._library.select_mode(_sw, n_trial=self._n_trial_ratio)
            else:
                new_mode, _ = self._library.select_mode(window, n_trial=self._n_trial)

            if new_mode != self._current_mode:
                self._current_mode = new_mode
                if new_mode is not None:
                    self._current_model = self._library.models[new_mode]
                    warmup = window.reshape(-1, 1)
                    self._current_model.reset(warmup_data=warmup)
                    self._thr_mode_change = self._library.mode_thresholds[new_mode]
                # UNKNOWN: 前モデルをそのまま使い続け閾値は維持
                self._prev_pred = None
                mode_changed = True
                self._steps_since_switch = 0  # 実際にモードが変わったときだけリセット

            # カウンタはトリガー発火時に常にリセット（偽アラームによる連続発火を防ぐ）
            self._consecutive_ratio = 0
            self._consecutive_fast = 0
            self._consecutive_high = 0

        self._steps_since_switch += 1

        is_anomaly = is_high_slow and not mode_changed

        return ModeTrackerState(
            mode=self._current_mode,
            residual=residual,
            is_anomaly=is_anomaly,
            mode_changed=mode_changed,
            steps_since_switch=self._steps_since_switch,
            trigger_type=trigger_type,
        )

    def _eval_ratio_counter(self) -> int:
        """定期バッチ評価で比率カウンタを更新して新しい値を返す。

        最近の warmup_ratio + n_trial_ratio + 1 ステップのみで select_mode を呼ぶ。
        長い全バッファを使わず短い窓を使うことで、新モードのデータが warmup に
        すぐ反映され、検知遅延を warmup_ratio + n_trial_ratio ≈ 60 ステップに短縮できる。

        設計根拠:
          lr=0.3 のリーキー積分器は (1-lr)^k = 0.7^k であり、15 ステップで
          0.7^15 ≈ 0.005 → 初期状態の影響 0.5% 未満。
          短い窓の warmup が新モードのデータで満たされれば select_mode が正確な
          判定を返す（旧モードデータの汚染なし）。

        評価後は現在モデルの状態を短いウォームアップで復元する。
        """
        if self._current_mode is None:
            return 0

        window = np.asarray(list(self._data_buf), dtype=np.float64)
        # 短い窓: warmup_ratio + n_trial_ratio + 1 ステップだけを使う
        total_needed = self._warmup_ratio + self._n_trial_ratio + 1
        if len(window) < total_needed:
            return 0
        short_window = window[-total_needed:]

        proposed, _ = self._library.select_mode(short_window, n_trial=self._n_trial_ratio)

        # select_mode が全モデルのリザーバをリセットするため現在モデルを復元する
        w = self._warmup_ratio
        if len(window) > w + 1:
            restore_warmup = window[-w - 1:-1].reshape(-1, 1)
        else:
            restore_warmup = window[:-1].reshape(-1, 1)
        self._current_model.reset(warmup_data=restore_warmup)
        self._prev_pred = self._current_model.run_step(
            np.array([window[-1]], dtype=np.float64)
        )

        if proposed is not None and proposed != self._current_mode:
            return self._consecutive_ratio + 1
        return 0

    def reset(self, mode: str | None = None) -> None:
        """状態をリセットする。

        Args:
            mode: リセット後のモード名（省略時は現在のモードを維持）
        """
        if mode is not None:
            if mode not in self._library.models:
                raise ValueError(f"mode '{mode}' が library にありません")
            self._current_mode = mode
            self._current_model = self._library.models[mode]
            self._thr_mode_change = self._library.mode_thresholds.get(
                mode, self._thr_mode_change
            )
        self._current_model.reset()
        self._res_buf.clear()
        self._data_buf.clear()
        self._consecutive_high = 0
        self._consecutive_fast = 0
        self._consecutive_ratio = 0
        self._steps_since_eval = 0
        self._steps_since_switch = 0
        self._prev_pred = None

    @property
    def current_mode(self) -> str | None:
        return self._current_mode

    @property
    def thr_mode_change(self) -> float:
        return self._thr_mode_change

    @property
    def thr_fast(self) -> float:
        """高速トリガーの閾値（thr_mode_change × thr_fast_multiplier）。"""
        return self._thr_mode_change * self._thr_fast_multiplier

    @property
    def ratio_threshold(self) -> float:
        """並列比較トリガーの比率閾値。"""
        return self._ratio_threshold


# ---------------------------------------------------------------------------
# デモ実行
# ---------------------------------------------------------------------------

_FS = 30.0
_FREQ = 2.0
_NOISE_STD = 0.01
_TRAIN_STEPS = 2000
_SEG_STEPS = 1000
_K_CONSECUTIVE = 300
_K_CONSECUTIVE_FAST = 50
_K_CONSECUTIVE_RATIO = 1  # ratio トリガー: 1 回成立で即発火 → 遅延 ~34 ステップ (1.1s)
_THR_FAST_MULTIPLIER = 10.0
_RATIO_THRESHOLD = 0.3
_MIN_STABLE = 500
_SEED = 42


def _build_library(config: ESNConfig, verbose: bool = True) -> ModeLibrary:
    """sine / sawtooth モデルを学習してライブラリを構築する。"""
    from esn_anomaly.waveform.data import generate_sawtooth, generate_sine

    rng = np.random.default_rng(_SEED)
    library = ModeLibrary()

    for name, gen in [("sine", generate_sine), ("sawtooth", generate_sawtooth)]:
        data = gen(
            _TRAIN_STEPS, fs=_FS, freq=_FREQ, noise_std=_NOISE_STD, rng=rng
        ).ravel()
        library.add_mode(name, data, config=config)
        if verbose:
            print(
                f"    {name}: select_thr={library.thresholds[name]:.4f},"
                f" mode_thr={library.mode_thresholds[name]:.4f}"
            )

    return library


def _run_scenario(
    tracker: ModeTracker,
    segments: list[tuple[str, NDArray[np.float64]]],
    spike_segments: list[tuple[int, int, float]] | None = None,
    verbose: bool = True,
) -> dict:
    """シナリオを実行して結果を返す。

    Returns:
        dict with keys:
            mode_changes: [(step, mode), ...]
            anomalies: [step, ...]
            steps: total step count
            signal: per-step input values (including spikes)
            residuals: per-step prediction residual
            thr_history: per-step thr_mode_change value
            thr_fast_history: per-step thr_fast value
            mode_history: per-step current mode label (str | None)
            true_boundaries: segment boundary step indices
            true_modes: mode label for each segment
            trigger_type_map: {step: "fast" | "slow"} for each mode_changed step
    """
    spike_amp: dict[int, float] = {}
    if spike_segments:
        for s_start, s_end, amp in spike_segments:
            for i in range(s_start, s_end):
                spike_amp[i] = amp

    mode_changes: list[tuple[int, str | None]] = []
    anomaly_steps: list[int] = []
    signal_hist: list[float] = []
    residual_hist: list[float] = []
    thr_hist: list[float] = []
    thr_fast_hist: list[float] = []
    mode_hist: list[str | None] = []
    trigger_type_map: dict[int, str] = {}

    all_data = np.concatenate([d for _, d in segments], axis=0).ravel()
    for i, x_val in enumerate(all_data):
        x_in = x_val + spike_amp.get(i, 0.0)
        state = tracker.step(x_in)
        signal_hist.append(x_in)
        residual_hist.append(state.residual)
        thr_hist.append(tracker.thr_mode_change)
        thr_fast_hist.append(tracker.thr_fast)
        mode_hist.append(state.mode)
        if state.mode_changed:
            mode_changes.append((i, state.mode))
            trigger_type_map[i] = state.trigger_type
            if verbose:
                label = state.mode if state.mode is not None else "UNKNOWN"
                print(f"    step {i:4d}: mode -> {label} [{state.trigger_type}]")
        if state.is_anomaly:
            anomaly_steps.append(i)

    # 真のセグメント境界
    boundaries: list[int] = []
    cum = 0
    for _, seg in segments[:-1]:
        cum += len(seg)
        boundaries.append(cum)
    true_modes = [label for label, _ in segments]

    return {
        "mode_changes": mode_changes,
        "anomalies": anomaly_steps,
        "steps": len(all_data),
        "signal": np.array(signal_hist),
        "residuals": np.array(residual_hist),
        "thr_history": np.array(thr_hist),
        "thr_fast_history": np.array(thr_fast_hist),
        "mode_history": mode_hist,
        "true_boundaries": boundaries,
        "true_modes": true_modes,
        "trigger_type_map": trigger_type_map,
    }


# モードごとの背景色
_MODE_COLORS: dict[str | None, str] = {
    "sine": "#A5D6A7",       # 淡緑
    "sawtooth": "#FFCC80",   # 淡橙
    "triangle": "#CE93D8",   # 淡紫
    None: "#EF9A9A",         # 淡赤（UNKNOWN）
}
_MODE_EDGE_COLORS: dict[str | None, str] = {
    "sine": "#2E7D32",
    "sawtooth": "#E65100",
    "triangle": "#6A1B9A",
    None: "#B71C1C",
}


def plot_mode_tracker(
    results: dict[str, dict],
    output_dir: str = "output",
) -> None:
    """モード追従結果を可視化して PNG ファイルに保存する。

    2 種類の図を出力する:

    1. **mode_tracker_scenarios.png** — 3 行 (S1/S2/S3) × 2 列
       - 左列: 信号波形 + 真のモード区間（背景色）+ 検知モード変化（縦線）+ 異常フラグ（点）
       - 右列: ステップ残差 + 移動平均 + thr_mode_change（ステップ状変化）

    2. **mode_tracker_summary.png** — 検知遅延・正解率・未知モード検知率の棒グラフ

    Args:
        results: {"S1": result_dict, "S2": ..., "S3": ...}
        output_dir: 保存先ディレクトリ
    """
    import matplotlib.pyplot as plt
    import matplotlib.patches as mpatches
    from pathlib import Path

    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)

    scenario_names = list(results.keys())
    n = len(scenario_names)

    # ------------------------------------------------------------------
    # Figure 1: シナリオ詳細
    # ------------------------------------------------------------------
    fig, axes = plt.subplots(n, 2, figsize=(14, 4 * n), squeeze=False)
    fig.suptitle("Phase 17: Online Mode Tracker – Scenario Results", fontsize=11)

    for row, name in enumerate(scenario_names):
        r = results[name]
        signal = r["signal"]
        residuals = r["residuals"]
        thr_hist = r["thr_history"]
        thr_fast_hist = r.get("thr_fast_history", thr_hist * 10)
        mode_hist = r["mode_history"]
        mode_changes = r["mode_changes"]
        anomalies = r["anomalies"]
        true_boundaries = r["true_boundaries"]
        true_modes = r["true_modes"]
        trigger_type_map = r.get("trigger_type_map", {})

        n_steps = len(signal)
        steps = np.arange(n_steps) / _FS  # → 秒

        # 移動平均残差
        win = 200
        moving_avg = np.convolve(residuals, np.ones(win) / win, mode="full")[:n_steps]

        ax_sig = axes[row, 0]
        ax_res = axes[row, 1]

        # ── 左: 信号 + 真のモード背景 ──────────────────────────────
        ax_sig.set_title(f"[{name}] Signal & Mode Tracking", fontsize=9)

        # 真のモード区間を背景色で塗る
        boundaries_ext = [0] + true_boundaries + [n_steps]
        for i_seg, (seg_start, seg_end) in enumerate(zip(boundaries_ext[:-1], boundaries_ext[1:])):
            seg_mode = true_modes[i_seg] if i_seg < len(true_modes) else None
            color = _MODE_COLORS.get(seg_mode, "#E0E0E0")
            ax_sig.axvspan(seg_start / _FS, seg_end / _FS, alpha=0.25, color=color, linewidth=0)

        # 真の境界を縦破線で示す
        for b in true_boundaries:
            ax_sig.axvline(b / _FS, color="dimgray", ls="--", lw=1.0, alpha=0.7)

        # 信号波形
        ax_sig.plot(steps, signal, color="#546E7A", lw=0.5, alpha=0.7, label="signal")

        # 検知モード変化を縦線で示す（fast=実線/太, slow=破線/細）
        for chg_step, chg_mode in mode_changes:
            ec = _MODE_EDGE_COLORS.get(chg_mode, "#000000")
            ttype = trigger_type_map.get(chg_step, "slow")
            lw = 2.2 if ttype == "fast" else 1.3
            ls = "-" if ttype == "fast" else "--"
            ax_sig.axvline(chg_step / _FS, color=ec, ls=ls, lw=lw, alpha=0.9)
            label = chg_mode if chg_mode is not None else "UNKNOWN"
            suffix = " [F]" if ttype == "fast" else ""
            ax_sig.text(
                chg_step / _FS + 0.05, 1.05, label + suffix,
                transform=ax_sig.get_xaxis_transform(),
                fontsize=6.5, color=ec, rotation=45, ha="left", va="bottom",
            )

        # 異常フラグを点で示す
        if anomalies:
            anom_t = np.array(anomalies) / _FS
            anom_y = signal[anomalies]
            ax_sig.scatter(anom_t, anom_y, s=4, color="crimson", zorder=5, label="anomaly")

        ax_sig.set_ylabel("Amplitude")
        ax_sig.set_ylim(-1.6, 1.6)
        ax_sig.grid(True, alpha=0.25)

        # 凡例: 真のモード区間
        handles = []
        for seg_mode in dict.fromkeys(true_modes):  # 順序保持で重複除去
            c = _MODE_COLORS.get(seg_mode, "#E0E0E0")
            label = seg_mode if seg_mode is not None else "UNKNOWN"
            handles.append(mpatches.Patch(facecolor=c, alpha=0.5, label=f"true: {label}"))
        if anomalies:
            handles.append(plt.Line2D([0], [0], marker="o", color="crimson",
                                      ls="none", markersize=4, label="anomaly"))
        ax_sig.legend(handles=handles, loc="lower right", fontsize=7, ncol=2)

        # ── 右: 残差 + 移動平均 + 閾値 ──────────────────────────
        ax_res.set_title(f"[{name}] Residual & Moving Average (w=200)", fontsize=9)

        # 真のモード背景（同じ）
        for i_seg, (seg_start, seg_end) in enumerate(zip(boundaries_ext[:-1], boundaries_ext[1:])):
            seg_mode = true_modes[i_seg] if i_seg < len(true_modes) else None
            color = _MODE_COLORS.get(seg_mode, "#E0E0E0")
            ax_res.axvspan(seg_start / _FS, seg_end / _FS, alpha=0.15, color=color, linewidth=0)

        # ステップ残差（薄く）
        ax_res.plot(steps, residuals, color="#90CAF9", lw=0.5, alpha=0.6, label="residual")

        # 移動平均
        ax_res.plot(steps, moving_avg, color="#1565C0", lw=1.2, label=f"moving avg (w={win})")

        # thr_mode_change（ステップ状に変化）
        ax_res.step(steps, thr_hist, where="post", color="crimson",
                    ls="--", lw=1.2, label="thr_slow (mode_change)")

        # thr_fast（10× 高速トリガー閾値）
        ax_res.step(steps, thr_fast_hist, where="post", color="darkorange",
                    ls=":", lw=1.2, label="thr_fast (×10)")

        # 検知モード変化の縦線（fast/slow 区別）
        for chg_step, chg_mode in mode_changes:
            ec = _MODE_EDGE_COLORS.get(chg_mode, "#000000")
            ttype = trigger_type_map.get(chg_step, "slow")
            lw = 2.2 if ttype == "fast" else 1.3
            ls = "-" if ttype == "fast" else "--"
            ax_res.axvline(chg_step / _FS, color=ec, ls=ls, lw=lw, alpha=0.9)

        # 真の境界
        for b in true_boundaries:
            ax_res.axvline(b / _FS, color="dimgray", ls="--", lw=1.0, alpha=0.7)

        ax_res.set_ylabel("Residual")
        ax_res.set_yscale("log")
        ax_res.set_ylim(bottom=1e-4)
        ax_res.legend(loc="upper right", fontsize=7)
        ax_res.grid(True, alpha=0.25, which="both")

    for ax in axes[-1, :]:
        ax.set_xlabel("Time [s]")

    plt.tight_layout()
    path1 = out / "mode_tracker_scenarios.png"
    plt.savefig(path1, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  Saved: {path1}")

    # ------------------------------------------------------------------
    # Figure 2: サマリ（検知遅延 / 正解率 / 未知検出率）
    # ------------------------------------------------------------------
    fig2, axes2 = plt.subplots(1, 3, figsize=(13, 4))
    fig2.suptitle("Phase 17: Mode Tracker Summary", fontsize=11)

    # 検知遅延 [steps]
    ax_delay = axes2[0]
    ax_delay.set_title("Detection Delay [steps]", fontsize=9)
    delays: list[float] = []
    delay_labels: list[str] = []
    delay_colors: list[str] = []

    for sc_name in ["S1", "S2"]:
        if sc_name not in results:
            continue
        r = results[sc_name]
        boundaries = r["true_boundaries"]
        changes = r["mode_changes"]
        if boundaries and changes:
            first_change_step = changes[0][0]
            delay = first_change_step - boundaries[0]
            delays.append(max(delay, 0))
        else:
            delays.append(float("nan"))
        delay_labels.append(sc_name)
        delay_colors.append("#1565C0" if sc_name == "S1" else "#6A1B9A")

    if delays:
        bars = ax_delay.bar(delay_labels, delays, color=delay_colors, alpha=0.8, width=0.5)
        for bar, val in zip(bars, delays):
            if not np.isnan(val):
                ax_delay.text(bar.get_x() + bar.get_width() / 2, bar.get_height() + 5,
                              f"{int(val)}", ha="center", va="bottom", fontsize=9)
        ax_delay.axhline(_K_CONSECUTIVE, color="crimson", ls="--", lw=1.2,
                         label=f"k_slow={_K_CONSECUTIVE}")
        ax_delay.axhline(_K_CONSECUTIVE_FAST, color="darkorange", ls=":", lw=1.2,
                         label=f"k_fast={_K_CONSECUTIVE_FAST}")
        ax_delay.axhline(_K_CONSECUTIVE_RATIO, color="green", ls="-.", lw=1.2,
                         label=f"k_ratio={_K_CONSECUTIVE_RATIO}")
        ax_delay.set_ylabel("Steps")
        ax_delay.legend(fontsize=7)
        ax_delay.grid(True, axis="y", alpha=0.3)

    # モード選択正解率（既知モードのみ）
    ax_acc = axes2[1]
    ax_acc.set_title("Mode Selection Accuracy (known modes)", fontsize=9)
    accuracies: list[float] = []
    acc_labels: list[str] = []

    for sc_name in scenario_names:
        r = results[sc_name]
        changes = r["mode_changes"]
        true_segs = r["true_modes"]
        boundaries_list = [0] + r["true_boundaries"] + [r["steps"]]

        if not changes:
            continue

        correct = 0
        total = 0
        for chg_step, det_mode in changes:
            if det_mode is None:
                continue  # UNKNOWN は正解率計算から除外
            # 真のモードを探す
            true_seg_mode = None
            for i_s, (s_start, s_end) in enumerate(
                zip(boundaries_list[:-1], boundaries_list[1:])
            ):
                if s_start <= chg_step < s_end:
                    true_seg_mode = true_segs[i_s] if i_s < len(true_segs) else None
                    break
            if true_seg_mode is not None:
                total += 1
                if det_mode == true_seg_mode:
                    correct += 1

        if total > 0:
            accuracies.append(correct / total * 100)
            acc_labels.append(sc_name)

    if accuracies:
        bars = ax_acc.bar(acc_labels, accuracies, color="#2E7D32", alpha=0.8, width=0.5)
        for bar, val in zip(bars, accuracies):
            ax_acc.text(bar.get_x() + bar.get_width() / 2, bar.get_height() + 1,
                        f"{val:.0f}%", ha="center", va="bottom", fontsize=9)
        ax_acc.axhline(90, color="crimson", ls="--", lw=1.2, label="target 90%")
        ax_acc.set_ylim(0, 110)
        ax_acc.set_ylabel("Accuracy [%]")
        ax_acc.legend(fontsize=7)
        ax_acc.grid(True, axis="y", alpha=0.3)

    # 未知モード検出率
    ax_unk = axes2[2]
    ax_unk.set_title("Unknown Mode Detection Rate", fontsize=9)
    unk_rates: list[float] = []
    unk_labels: list[str] = []

    for sc_name in scenario_names:
        r = results[sc_name]
        true_modes_list = r["true_modes"]
        # unknown_true_segment が存在するか
        unknown_segs = [m for m in true_modes_list if m not in ("sine", "sawtooth")]
        if not unknown_segs:
            continue

        boundaries_list = [0] + r["true_boundaries"] + [r["steps"]]
        changes = r["mode_changes"]

        # unknown セグメント内で None が検知された割合（セグメント単位）
        unknown_detected = 0
        unknown_total = 0
        for i_s, seg_mode in enumerate(true_modes_list):
            if seg_mode in ("sine", "sawtooth"):
                continue
            seg_start = boundaries_list[i_s]
            seg_end = boundaries_list[i_s + 1]
            unknown_total += 1
            found = any(seg_start <= step < seg_end and mode is None
                        for step, mode in changes)
            if found:
                unknown_detected += 1

        if unknown_total > 0:
            unk_rates.append(unknown_detected / unknown_total * 100)
            unk_labels.append(sc_name)

    if unk_rates:
        bars = ax_unk.bar(unk_labels, unk_rates, color="#6A1B9A", alpha=0.8, width=0.5)
        for bar, val in zip(bars, unk_rates):
            ax_unk.text(bar.get_x() + bar.get_width() / 2, bar.get_height() + 1,
                        f"{val:.0f}%", ha="center", va="bottom", fontsize=9)
        ax_unk.axhline(80, color="crimson", ls="--", lw=1.2, label="target 80%")
        ax_unk.set_ylim(0, 110)
        ax_unk.set_ylabel("Rate [%]")
        ax_unk.legend(fontsize=7)
        ax_unk.grid(True, axis="y", alpha=0.3)
    else:
        ax_unk.text(0.5, 0.5, "No unknown mode segments",
                    ha="center", va="center", transform=ax_unk.transAxes)

    plt.tight_layout()
    path2 = out / "mode_tracker_summary.png"
    plt.savefig(path2, dpi=150, bbox_inches="tight")
    plt.close(fig2)
    print(f"  Saved: {path2}")


def run_demo(verbose: bool = True, plot: bool = False, output_dir: str = "output") -> None:
    from esn_anomaly.waveform.data import generate_sawtooth, generate_sine, generate_triangle

    print("=" * 60)
    print("Mode Tracker Demo (Phase 17)")
    print("=" * 60)

    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=100, seed=_SEED)

    print("\n[1] Building ModeLibrary (sine, sawtooth) ...")
    library = _build_library(config, verbose=verbose)

    rng = np.random.default_rng(_SEED + 1)

    def mk_sine() -> NDArray[np.float64]:
        return generate_sine(
            _SEG_STEPS, fs=_FS, freq=_FREQ, noise_std=_NOISE_STD, rng=rng
        ).ravel()

    def mk_saw() -> NDArray[np.float64]:
        return generate_sawtooth(
            _SEG_STEPS, fs=_FS, freq=_FREQ, noise_std=_NOISE_STD, rng=rng
        ).ravel()

    def mk_tri() -> NDArray[np.float64]:
        return generate_triangle(
            _SEG_STEPS, fs=_FS, freq=_FREQ, noise_std=_NOISE_STD, rng=rng
        ).ravel()

    def make_tracker() -> ModeTracker:
        return ModeTracker(
            library,
            initial_mode="sine",
            k_consecutive=_K_CONSECUTIVE,
            min_stable_steps=_MIN_STABLE,
            k_consecutive_fast=_K_CONSECUTIVE_FAST,
            thr_fast_multiplier=_THR_FAST_MULTIPLIER,
            k_consecutive_ratio=_K_CONSECUTIVE_RATIO,
            ratio_threshold=_RATIO_THRESHOLD,
        )

    # S1: sine -> sawtooth -> sine
    print("\n[S1] sine -> sawtooth -> sine (known mode transitions)")
    result_s1 = _run_scenario(
        make_tracker(),
        [("sine", mk_sine()), ("sawtooth", mk_saw()), ("sine", mk_sine())],
        verbose=verbose,
    )
    changes_s1 = result_s1["mode_changes"]
    if changes_s1:
        first = changes_s1[0]
        delay = first[0] - _SEG_STEPS
        print(
            f"    First switch: step {first[0]} (delay {delay} steps),"
            f" mode={first[1]!r}"
        )
    else:
        print("    No mode switch detected")

    # S2: sine -> triangle -> sine
    print("\n[S2] sine -> triangle -> sine (unknown mode)")
    result_s2 = _run_scenario(
        make_tracker(),
        [("sine", mk_sine()), ("triangle", mk_tri()), ("sine", mk_sine())],
        verbose=verbose,
    )
    unknown_detected = any(m is None for _, m in result_s2["mode_changes"])
    print(f"    Unknown mode detected: {'YES' if unknown_detected else 'NO'}")

    # S3: sine with 50-step spike
    print("\n[S3] sine with 50-step spike injection (no mode switch expected)")
    result_s3 = _run_scenario(
        make_tracker(),
        [("sine", mk_sine())],
        spike_segments=[(300, 350, 0.8)],
        verbose=verbose,
    )
    n_mode_changes = len(result_s3["mode_changes"])
    spike_anomalies = sum(300 <= s < 350 for s in result_s3["anomalies"])
    print(f"    Mode changes during spike: {n_mode_changes} (expected 0)")
    print(f"    Anomaly flags during spike window: {spike_anomalies}")

    # Summary
    print("\n" + "=" * 60)
    print("Summary")
    print("=" * 60)

    s1_ok = len(changes_s1) >= 1
    s1_correct = s1_ok and changes_s1[0][1] == "sawtooth"
    s2_ok = unknown_detected
    s3_ok = n_mode_changes == 0

    print(f"  S1 mode switch detected  : {'OK' if s1_ok else 'FAIL'}")
    print(f"  S1 correct model selected: {'OK' if s1_correct else 'FAIL'}")
    print(f"  S2 unknown mode detected : {'OK' if s2_ok else 'FAIL'}")
    print(f"  S3 no false mode switch  : {'OK' if s3_ok else 'FAIL'}")

    if plot:
        print(f"\n[Plot] Saving figures to {output_dir}/ ...")
        plot_mode_tracker(
            {"S1": result_s1, "S2": result_s2, "S3": result_s3},
            output_dir=output_dir,
        )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Phase 17: Online Mode Tracker Demo")
    parser.add_argument("--quiet", action="store_true", help="詳細出力を抑制する")
    parser.add_argument("--plot", action="store_true", help="結果を可視化して PNG に保存する")
    parser.add_argument("--output-dir", default="output", help="PNG 保存先ディレクトリ (default: output)")
    args = parser.parse_args()
    run_demo(verbose=not args.quiet, plot=args.plot, output_dir=args.output_dir)
