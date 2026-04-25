"""個体差モデル定義・適応実験（Phase 16）。

ベースラインモデル（IND-0 の正常データで訓練）を他個体に適用し、
個体差によって誤警報率が増大することを確認する（Phase 16-B）。
続けて AdaptiveThreshold によるしきい値適応を実施し、
各個体で誤警報率が目標値に収束することを検証する（Phase 16-D）。

使い方::

    # デモ実行（テキスト出力のみ）
    uv run python -m esn_anomaly.servo.individual

    # 可視化あり（output/ に PNG を保存）
    uv run python -m esn_anomaly.servo.individual --plot

    # 保存先を指定
    uv run python -m esn_anomaly.servo.individual --plot --output-dir results/phase16

    make run-individual          # テキスト出力のみ
    make run-individual-plot     # 可視化あり

    # ライブラリとして使用
    from esn_anomaly.servo.individual import INDIVIDUALS, generate_individual_servo
    u_raw, cmd_raw = generate_individual_servo(INDIVIDUALS["IND-1"], profile)
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import numpy as np
from numpy.typing import NDArray

from esn_anomaly.model import ESNConfig
from esn_anomaly.servo.adaptive import AdaptiveThreshold
from esn_anomaly.servo.adapter import ServoStreamAdapter, StepResult
from esn_anomaly.servo.command_test import NOISE, SEED, WARMUP, _make_periodic_profile
from esn_anomaly.servo.data import (
    CommandProfile,
    CommandSegment,
    DisturbanceSegment,
    MeasurementNoise,
    MotionPattern,
    ServoParams,
    generate_servo_with_profile,
)

# ------------------------------------------------------------------
# 物理定数（ServoParams デフォルトと一致）
# ------------------------------------------------------------------

_THETA_EQ: float = -45.0
_REACH_NEG: float = 35.0   # theta_eq - theta_reach_neg
_REACH_POS: float = 65.0   # theta_reach_pos - theta_eq


# ------------------------------------------------------------------
# 個体差パラメータ
# ------------------------------------------------------------------

@dataclass
class IndividualServoParams:
    """個体差を表すパラメータセット。

    Attributes:
        pos_noise_scale:  位置センサノイズ倍率（基準 1.0 = std 2.0 mrad）
        load_noise_scale: 電流センサノイズ倍率（基準 1.0 = std 3.0 mA）
        load_offset:      電流系統オフセット [mA]（モータ差・摩耗）
        spring_scale:     ばね定数倍率（K_neg, K_pos に乗じる）
        friction:         背景クーロン摩擦係数（常時作用）
    """

    pos_noise_scale: float = 1.0
    load_noise_scale: float = 1.0
    load_offset: float = 0.0
    spring_scale: float = 1.0
    friction: float = 0.0


#: 評価に使う 5 個体（IND-0 がベースライン訓練元）
INDIVIDUALS: dict[str, IndividualServoParams] = {
    "IND-0": IndividualServoParams(),
    "IND-1": IndividualServoParams(pos_noise_scale=2.0),
    "IND-2": IndividualServoParams(load_noise_scale=2.0, load_offset=10.0),
    "IND-3": IndividualServoParams(spring_scale=0.85, friction=0.05),
    "IND-4": IndividualServoParams(
        pos_noise_scale=2.0, load_noise_scale=2.0, load_offset=-8.0,
        spring_scale=1.15, friction=0.03,
    ),
}


# ------------------------------------------------------------------
# 個体差パラメータ → ServoParams / MeasurementNoise 変換
# ------------------------------------------------------------------

def make_servo_params(ind: IndividualServoParams) -> ServoParams:
    """IndividualServoParams から ServoParams を生成する。

    spring_scale を theta_reach に変換:
        reach_new = reach_base / spring_scale
    """
    reach_neg = _REACH_NEG / ind.spring_scale
    reach_pos = _REACH_POS / ind.spring_scale
    return ServoParams(
        theta_reach_neg=_THETA_EQ - reach_neg,
        theta_reach_pos=_THETA_EQ + reach_pos,
        coulomb_friction=ind.friction,
    )


def make_noise(ind: IndividualServoParams) -> MeasurementNoise:
    """IndividualServoParams から MeasurementNoise を生成する。"""
    return MeasurementNoise(
        pos_std=NOISE.pos_std * ind.pos_noise_scale,
        load_std=NOISE.load_std * ind.load_noise_scale,
    )


# ------------------------------------------------------------------
# 個体差付きデータ生成
# ------------------------------------------------------------------

def generate_individual_servo(
    ind: IndividualServoParams,
    profile: CommandProfile,
    disturbances: list[DisturbanceSegment] | None = None,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.float64]]:
    """IndividualServoParams を反映したサーボ観測時系列を生成する。

    Args:
        ind:          個体差パラメータ
        profile:      コマンドプロファイル
        disturbances: 外乱区間リスト（None なら正常のみ）
        rng:          乱数ジェネレータ

    Returns:
        (u_raw, cmd_raw):
          u_raw    shape (n, 2): [pos_mrad, load_mA]
          cmd_raw  shape (n,):   目標値 [mrad]
    """
    servo_params = make_servo_params(ind)
    noise = make_noise(ind)

    u_raw, cmd_raw = generate_servo_with_profile(
        profile,
        disturbances=disturbances,
        noise=noise,
        base_params=servo_params,
        rng=rng,
    )

    # 電流系統オフセット（モータ差・配線バイアス）
    if ind.load_offset != 0.0:
        u_raw = u_raw.copy()
        u_raw[:, 1] += ind.load_offset

    return u_raw, cmd_raw


# ------------------------------------------------------------------
# AdaptiveThreshold セット構築
# ------------------------------------------------------------------

def build_adaptive_thresholds(
    adapter: ServoStreamAdapter,
    target_rate: float = 0.03,
) -> tuple[AdaptiveThreshold, AdaptiveThreshold, AdaptiveThreshold]:
    """学習済みアダプタからベースライン閾値を取得して AdaptiveThreshold を構築する。

    Args:
        adapter:     学習済み ServoStreamAdapter
        target_rate: 目標誤警報率

    Returns:
        (at_esn_pos, at_esn_load, at_phys) の 3 つの AdaptiveThreshold
    """
    det = adapter._detector
    thr_pos = det._thr_pos
    thr_load = det._thr_load
    thr_phys = det._est_ff._threshold if det._est_ff is not None else 1.0

    at_esn_pos = AdaptiveThreshold(
        initial=thr_pos,
        target_rate=target_rate,
        hard_min=thr_pos * 0.1,
        hard_max=thr_pos * 5.0,
    )
    at_esn_load = AdaptiveThreshold(
        initial=thr_load,
        target_rate=target_rate,
        hard_min=thr_load * 0.1,
        hard_max=thr_load * 5.0,
    )
    at_phys = AdaptiveThreshold(
        initial=thr_phys,
        target_rate=target_rate,
        hard_min=thr_phys * 0.2,
        hard_max=thr_phys * 8.0,
    )
    return at_esn_pos, at_esn_load, at_phys


# ------------------------------------------------------------------
# 適応フェーズ実行（軌跡収集あり）
# ------------------------------------------------------------------

def _run_adaptation_phase(
    history: list[StepResult],
    at_esn_pos: AdaptiveThreshold,
    at_esn_load: AdaptiveThreshold,
    at_phys: AdaptiveThreshold,
    warmup: int,
) -> dict:
    """適応フェーズを実行してしきい値軌跡を収集する。

    Args:
        history:    StepResult のリスト
        at_esn_pos: ESN pos 用 AdaptiveThreshold（更新される）
        at_esn_load: ESN load 用 AdaptiveThreshold（更新される）
        at_phys:    物理推定用 AdaptiveThreshold（更新される）
        warmup:     最初の warmup ステップはスキップ（更新なし）

    Returns:
        軌跡辞書:
          "thr_esn_pos", "thr_esn_load", "thr_phys" — 各ステップでの閾値
          "esn_flag", "phys_flag"                    — 各ステップの異常フラグ
          "step_idx"                                 — 元データ中のインデックス
    """
    traj: dict[str, list] = {
        "thr_esn_pos": [], "thr_esn_load": [], "thr_phys": [],
        "esn_flag": [], "phys_flag": [], "step_idx": [],
    }

    for i, r in enumerate(history):
        if i < warmup:
            continue

        is_esn_pos = r.esn_res_pos > at_esn_pos.threshold
        is_esn_load = r.esn_res_load > at_esn_load.threshold
        is_phys = r.phys_is_steady and (r.phys_residual > at_phys.threshold)

        traj["thr_esn_pos"].append(at_esn_pos.threshold)
        traj["thr_esn_load"].append(at_esn_load.threshold)
        traj["thr_phys"].append(at_phys.threshold)
        traj["esn_flag"].append(is_esn_pos or is_esn_load)
        traj["phys_flag"].append(is_phys)
        traj["step_idx"].append(i)

        at_esn_pos.update(is_esn_pos)
        at_esn_load.update(is_esn_load)
        at_phys.update(is_phys)

    return traj


def _compute_fpr_adaptive(
    history: list[StepResult],
    at_esn_pos: AdaptiveThreshold,
    at_esn_load: AdaptiveThreshold,
    at_phys: AdaptiveThreshold,
    warmup: int,
    adapt: bool,
) -> tuple[float, float]:
    """AdaptiveThreshold を使って ESN/Phys FPR を計算する。

    Args:
        history:     StepResult のリスト
        at_esn_pos:  ESN pos 用 AdaptiveThreshold
        at_esn_load: ESN load 用 AdaptiveThreshold
        at_phys:     物理推定用 AdaptiveThreshold
        warmup:      最初の warmup ステップはスキップ
        adapt:       True のとき各ステップで threshold を更新する

    Returns:
        (esn_fpr, phys_fpr) [%]
    """
    esn_flags: list[bool] = []
    phys_flags: list[bool] = []

    for i, r in enumerate(history):
        if i < warmup:
            continue

        is_esn_pos = r.esn_res_pos > at_esn_pos.threshold
        is_esn_load = r.esn_res_load > at_esn_load.threshold
        is_phys = r.phys_is_steady and (r.phys_residual > at_phys.threshold)

        esn_flags.append(is_esn_pos or is_esn_load)
        phys_flags.append(is_phys)

        if adapt:
            at_esn_pos.update(is_esn_pos)
            at_esn_load.update(is_esn_load)
            at_phys.update(is_phys)

    esn_fpr = float(np.mean(esn_flags)) * 100 if esn_flags else 0.0
    phys_fpr = float(np.mean(phys_flags)) * 100 if phys_flags else 0.0
    return esn_fpr, phys_fpr


def _compute_detect_adaptive(
    history: list[StepResult],
    at_phys: AdaptiveThreshold,
    dist_start: int,
    dist_end: int,
) -> float:
    """外乱区間での検知率を AdaptiveThreshold で評価する（threshold 更新なし）。

    Args:
        history:    StepResult のリスト
        at_phys:    適応済み AdaptiveThreshold（更新しない）
        dist_start: 外乱区間開始インデックス
        dist_end:   外乱区間終了インデックス（exclusive）

    Returns:
        検知率 [%]
    """
    n = len(history)
    lo = max(0, dist_start)
    hi = min(n, dist_end)
    if lo >= hi:
        return 0.0

    flags = [
        history[i].phys_is_steady and (history[i].phys_residual > at_phys.threshold)
        for i in range(lo, hi)
    ]
    return float(np.mean(flags)) * 100 if flags else 0.0


# ------------------------------------------------------------------
# 可視化
# ------------------------------------------------------------------

def _moving_average(arr: list[bool], window: int) -> NDArray[np.float64]:
    """因果的移動平均でエラー率を計算する（numpy cumsum ベース）。"""
    x = np.array(arr, dtype=float)
    cs = np.cumsum(x)
    cs[window:] = cs[window:] - cs[:-window]
    counts = np.minimum(np.arange(1, len(x) + 1), window)
    return cs / counts


def plot_adaptation(
    results: dict[str, dict],
    output_dir: str | Path = "output",
) -> None:
    """適応実験の結果を可視化して PNG ファイルに保存する。

    2 種類の図を出力する:

    1. **adaptation_trajectories.png** — 個体ごとのしきい値推移
       - 行: 個体
       - 左列: 物理推定器しきい値 (thr_phys) の時系列
         - 黒実線: 実際のしきい値、灰破線: 初期値（ベースライン）
         - オレンジ帯: hard_min〜hard_max
       - 右列: 物理異常フラグの移動平均 FPR（窓 200 step）
         - 青線: FPR、赤破線: target_rate

    2. **adaptation_summary.png** — 個体間の FPR / 検知率比較
       - グループ棒グラフ: Baseline / Adapted / SC 検知率

    Args:
        results:    run_adaptation() の戻り値
        output_dir: 保存先ディレクトリ
    """
    import matplotlib.pyplot as plt
    import matplotlib.patches as mpatches

    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)

    ind_names = list(results.keys())
    n = len(ind_names)

    # ------------------------------------------------------------------
    # Figure 1: しきい値軌跡
    # ------------------------------------------------------------------
    fig, axes = plt.subplots(n, 2, figsize=(14, 3 * n), squeeze=False)
    fig.suptitle("Phase 16: AdaptiveThreshold Convergence", fontsize=11)

    for row, ind_name in enumerate(ind_names):
        r = results[ind_name]
        traj = r.get("trajectory")

        ax_thr = axes[row, 0]
        ax_fpr = axes[row, 1]

        ax_thr.set_title(f"{ind_name}: phys threshold", fontsize=9)
        ax_fpr.set_title(f"{ind_name}: phys FPR (moving avg)", fontsize=9)

        if traj is None or len(traj["thr_phys"]) == 0:
            ax_thr.text(0.5, 0.5, "no trajectory data", ha="center", va="center",
                        transform=ax_thr.transAxes)
            ax_fpr.text(0.5, 0.5, "no trajectory data", ha="center", va="center",
                        transform=ax_fpr.transAxes)
            continue

        steps = np.array(traj["step_idx"]) * 0.01  # → 秒
        thr_phys = np.array(traj["thr_phys"])
        phys_flag = traj["phys_flag"]

        # ── 左: しきい値推移 ──
        initial_phys = r["final_thr_phys"] * 1.0  # 最終値は残る; 初期は results から別途記録
        # hard_min / hard_max を近似で表示（initial × 0.2 〜 × 8.0）
        # 実際の hard_min/max は build_adaptive_thresholds から来るが、
        # results に保存していないため initial * 倍率で再現
        init_from_traj = float(traj["thr_phys"][0])
        hard_min_approx = init_from_traj * 0.2
        hard_max_approx = init_from_traj * 8.0

        ax_thr.fill_between(steps, hard_min_approx, hard_max_approx,
                            alpha=0.08, color="orange", label="hard bounds")
        ax_thr.axhline(init_from_traj, color="gray", ls="--", lw=1.0,
                       alpha=0.8, label="initial (baseline)")
        ax_thr.plot(steps, thr_phys, color="steelblue", lw=1.0, label="thr_phys")
        ax_thr.set_ylabel("threshold [mA]")
        ax_thr.legend(loc="upper right", fontsize=7)
        ax_thr.grid(True, alpha=0.3)

        # ── 右: 移動平均 FPR ──
        ma_fpr = _moving_average(phys_flag, window=200) * 100
        target_rate = r.get("target_rate", 3.0)

        ax_fpr.plot(steps, ma_fpr, color="darkorange", lw=0.9, label="FPR (win=200)")
        ax_fpr.axhline(target_rate, color="crimson", ls="--", lw=1.2,
                       label=f"target {target_rate:.1f}%")
        ax_fpr.set_ylim(0, max(50, float(np.max(ma_fpr)) * 1.1))
        ax_fpr.set_ylabel("FPR [%]")
        ax_fpr.legend(loc="upper right", fontsize=7)
        ax_fpr.grid(True, alpha=0.3)

    for ax in axes[-1, :]:
        ax.set_xlabel("Time [s]")

    plt.tight_layout()
    path1 = out / "adaptation_trajectories.png"
    plt.savefig(path1, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path1}")

    # ------------------------------------------------------------------
    # Figure 2: FPR / 検知率 サマリー棒グラフ
    # ------------------------------------------------------------------
    fig2, (ax_fpr2, ax_det) = plt.subplots(1, 2, figsize=(12, 5))
    fig2.suptitle("Phase 16: Individual Adaptation — FPR and SC Detection Comparison", fontsize=11)

    x = np.arange(n)
    w = 0.28

    baseline_phys = [results[k]["baseline_phys_fpr"] for k in ind_names]
    adapted_phys  = [results[k]["adapted_phys_fpr"]  for k in ind_names]
    baseline_esn  = [results[k]["baseline_esn_fpr"]  for k in ind_names]
    adapted_esn   = [results[k]["adapted_esn_fpr"]   for k in ind_names]

    # 誤警報率: Baseline Phys / Adapted Phys / Baseline ESN
    ax_fpr2.bar(x - w, baseline_phys, w, label="Baseline Phys", color="steelblue", alpha=0.8)
    ax_fpr2.bar(x,     adapted_phys,  w, label="Adapted Phys",  color="limegreen", alpha=0.8)
    ax_fpr2.bar(x + w, baseline_esn,  w, label="Baseline ESN",  color="darkorange", alpha=0.6)
    target = results[ind_names[0]].get("target_rate", 3.0)
    ax_fpr2.axhline(target, color="crimson", ls="--", lw=1.2, label=f"target {target:.1f}%")
    ax_fpr2.set_xticks(x)
    ax_fpr2.set_xticklabels(ind_names)
    ax_fpr2.set_ylabel("FPR [%]")
    ax_fpr2.set_title("False Positive Rate (normal data)")
    ax_fpr2.legend(fontsize=8)
    ax_fpr2.grid(True, axis="y", alpha=0.3)

    # SC 検知率: Baseline / Adapted
    base_detect  = [results[k]["base_sc_detect"]    for k in ind_names]
    adpt_detect  = [results[k]["adapted_sc_detect"] for k in ind_names]

    ax_det.bar(x - w / 2, base_detect, w, label="Baseline (SC)", color="steelblue", alpha=0.8)
    ax_det.bar(x + w / 2, adpt_detect, w, label="Adapted (SC)",  color="limegreen", alpha=0.8)
    ax_det.set_xticks(x)
    ax_det.set_xticklabels(ind_names)
    ax_det.set_ylabel("Detection Rate [%]")
    ax_det.set_ylim(0, 105)
    ax_det.set_title("SC Detection Rate (ext_torque=0.3)")
    ax_det.legend(fontsize=8)
    ax_det.grid(True, axis="y", alpha=0.3)

    plt.tight_layout()
    path2 = out / "adaptation_summary.png"
    plt.savefig(path2, dpi=150)
    plt.close(fig2)
    print(f"  Saved: {path2}")


# ------------------------------------------------------------------
# 主実験関数
# ------------------------------------------------------------------

def run_adaptation(
    n_train_cycles: int = 10,
    n_adapt_cycles: int = 15,
    n_warmup_cycles: int = 3,
    target_rate: float = 0.03,
    seed: int = SEED,
    verbose: bool = True,
    plot: bool = False,
    output_dir: str | Path = "output",
) -> dict[str, dict]:
    """個体差適応実験を実行して結果辞書を返す（Phase 16）。

    手順:
        1. IND-0 の正常データでベースラインモデルを訓練
        2. 各個体の正常データでベースライン FPR を測定（Phase 16-B）
        3. AdaptiveThreshold でしきい値を適応（Phase 16-D）
        4. SC 外乱シナリオで検知率をベースラインと比較（Phase 16-E）

    Args:
        n_train_cycles:  IND-0 訓練サイクル数
        n_adapt_cycles:  適応フェーズのサイクル数（正常データ）
        n_warmup_cycles: AdaptiveThreshold 更新を開始するまでのサイクル数
        target_rate:     目標誤警報率（例: 0.03 = 3%）
        seed:            乱数シード
        verbose:         True のとき結果を標準出力に表示
        plot:            True のとき可視化 PNG を output_dir に保存
        output_dir:      PNG 保存先ディレクトリ

    Returns:
        個体 ID をキーとする結果辞書。trajectory キーに適応フェーズの軌跡を含む。
    """
    # ------------------------------------------------------------------
    # 1. ベースラインモデル訓練（IND-0）
    # ------------------------------------------------------------------
    rng_train = np.random.default_rng(seed)
    train_profile = _make_periodic_profile(n_train_cycles)
    u_train, cmd_train = generate_individual_servo(
        INDIVIDUALS["IND-0"], train_profile, rng=rng_train
    )

    baseline_adapter = ServoStreamAdapter.from_training_data(
        u_train,
        cmd_train,
        esn_config=ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=seed),
        warmup=WARMUP,
    )

    if verbose:
        print("=" * 62)
        print("Phase 16: 個体差適応実験")
        print("=" * 62)
        det = baseline_adapter._detector
        print(f"\n[1] ベースラインモデル訓練完了 (IND-0, {n_train_cycles} cycles)")
        print(f"    thr_pos={det._thr_pos:.5f}  thr_load={det._thr_load:.5f}  "
              f"thr_phys={det._est_ff._threshold:.2f}")

    pat = MotionPattern()
    steps_per_cycle = pat.steps_per_cycle
    warmup_steps = max(WARMUP, n_warmup_cycles * steps_per_cycle)

    results: dict[str, dict] = {}

    for ind_name, ind in INDIVIDUALS.items():
        rng_ind = np.random.default_rng(seed + abs(hash(ind_name)) % 1000)

        # ------------------------------------------------------------------
        # 2. 正常データ生成・ストリーミング推論
        # ------------------------------------------------------------------
        adapt_profile = _make_periodic_profile(n_adapt_cycles)
        u_normal, cmd_normal = generate_individual_servo(ind, adapt_profile, rng=rng_ind)

        baseline_adapter.reset(clear_history=True)
        for i in range(len(u_normal)):
            baseline_adapter.update_cmd(float(cmd_normal[i]))
            baseline_adapter.on_measurement(i * 0.01, u_normal[i, 0], u_normal[i, 1])

        history_normal = baseline_adapter.history

        # ------------------------------------------------------------------
        # ベースライン FPR（固定しきい値）
        # ------------------------------------------------------------------
        post_warmup = history_normal[WARMUP:]
        baseline_esn_fpr = float(np.mean([r.esn_anomaly for r in post_warmup])) * 100
        baseline_phys_fpr = float(np.mean([r.phys_anomaly for r in post_warmup])) * 100

        # ------------------------------------------------------------------
        # 3. AdaptiveThreshold 適応（軌跡収集）
        # ------------------------------------------------------------------
        at_esn_pos, at_esn_load, at_phys = build_adaptive_thresholds(
            baseline_adapter, target_rate=target_rate
        )
        trajectory = _run_adaptation_phase(
            history_normal, at_esn_pos, at_esn_load, at_phys,
            warmup=warmup_steps,
        )

        # 最後の n_warmup_cycles 相当区間で定常 FPR を評価（threshold 更新なし）
        eval_start = max(WARMUP, len(history_normal) - n_warmup_cycles * steps_per_cycle)
        eval_history = history_normal[eval_start:]
        adapted_esn_fpr, adapted_phys_fpr = _compute_fpr_adaptive(
            eval_history, at_esn_pos, at_esn_load, at_phys,
            warmup=0, adapt=False,
        )

        # ------------------------------------------------------------------
        # 4. SC 外乱シナリオ検知率
        # ------------------------------------------------------------------
        sc_profile = CommandProfile([
            CommandSegment("hold", pat.home_mrad, 500),
            CommandSegment("hold", pat.home_mrad, 2000),
            CommandSegment("hold", pat.home_mrad, 500),
        ])
        sc_dist = [DisturbanceSegment(500, 2500, ext_torque=0.3)]
        rng_sc = np.random.default_rng(seed + 777)
        u_sc, cmd_sc = generate_individual_servo(
            ind, sc_profile, disturbances=sc_dist, rng=rng_sc
        )

        baseline_adapter.reset(clear_history=True)
        for i in range(len(u_sc)):
            baseline_adapter.update_cmd(float(cmd_sc[i]))
            baseline_adapter.on_measurement(i * 0.01, u_sc[i, 0], u_sc[i, 1])

        history_sc = baseline_adapter.history

        base_sc_detect = float(
            np.mean([r.phys_anomaly for r in history_sc[500:2500]])
        ) * 100
        adapted_sc_detect = _compute_detect_adaptive(
            history_sc, at_phys, dist_start=500, dist_end=2500
        )

        results[ind_name] = {
            "baseline_esn_fpr": baseline_esn_fpr,
            "baseline_phys_fpr": baseline_phys_fpr,
            "adapted_esn_fpr": adapted_esn_fpr,
            "adapted_phys_fpr": adapted_phys_fpr,
            "base_sc_detect": base_sc_detect,
            "adapted_sc_detect": adapted_sc_detect,
            "final_thr_esn_pos": at_esn_pos.threshold,
            "final_thr_esn_load": at_esn_load.threshold,
            "final_thr_phys": at_phys.threshold,
            "target_rate": target_rate * 100,  # % 表示用
            "trajectory": trajectory,
        }

        if verbose:
            desc = (f"noise×({ind.pos_noise_scale:.1f},{ind.load_noise_scale:.1f})"
                    f" offset={ind.load_offset:+.0f}mA"
                    f" spring×{ind.spring_scale:.2f}"
                    f" fric={ind.friction:.2f}")
            print(f"\n  {ind_name}  {desc}")
            print(f"    Baseline FPR: ESN={baseline_esn_fpr:.1f}%  "
                  f"Phys={baseline_phys_fpr:.1f}%")
            print(f"    Adapted  FPR: ESN={adapted_esn_fpr:.1f}%  "
                  f"Phys={adapted_phys_fpr:.1f}%")
            print(f"    SC 検知率:    Base={base_sc_detect:.1f}%  "
                  f"Adapted={adapted_sc_detect:.1f}%")
            print(f"    最終 thr:     esn_pos={at_esn_pos.threshold:.5f}  "
                  f"phys={at_phys.threshold:.2f}")

    if verbose:
        print("\n" + "-" * 62)
        print(f"{'個体':<8}  {'Base Phys FPR':>14}  {'Adpt Phys FPR':>14}  "
              f"{'Base SC':>8}  {'Adpt SC':>8}")
        print("-" * 62)
        for ind_name, r in results.items():
            print(f"  {ind_name:<6}  {r['baseline_phys_fpr']:>13.1f}%  "
                  f"{r['adapted_phys_fpr']:>13.1f}%  "
                  f"{r['base_sc_detect']:>7.1f}%  "
                  f"{r['adapted_sc_detect']:>7.1f}%")
        print("=" * 62)

    if plot:
        print(f"\n[可視化] {output_dir} に保存中 ...")
        plot_adaptation(results, output_dir=output_dir)

    return results


# ------------------------------------------------------------------
# エントリポイント
# ------------------------------------------------------------------

if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(
        description="Phase 16: 個体差適応実験"
    )
    parser.add_argument(
        "--plot", action="store_true",
        help="可視化 PNG を output-dir に保存する",
    )
    parser.add_argument(
        "--output-dir", default="output", metavar="DIR",
        help="PNG 保存先ディレクトリ（デフォルト: output）",
    )
    parser.add_argument(
        "--train-cycles", type=int, default=10, metavar="N",
        help="ベースライン訓練サイクル数（デフォルト: 10）",
    )
    parser.add_argument(
        "--adapt-cycles", type=int, default=15, metavar="N",
        help="適応フェーズのサイクル数（デフォルト: 15）",
    )
    parser.add_argument(
        "--target-rate", type=float, default=0.03, metavar="R",
        help="目標誤警報率 0〜1（デフォルト: 0.03）",
    )
    args = parser.parse_args()

    run_adaptation(
        n_train_cycles=args.train_cycles,
        n_adapt_cycles=args.adapt_cycles,
        target_rate=args.target_rate,
        plot=args.plot,
        output_dir=args.output_dir,
    )
