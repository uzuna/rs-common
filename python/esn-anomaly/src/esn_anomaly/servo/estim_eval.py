"""物理推定器 vs ESN 残差の比較評価（Phase 14）。

3 つの検出手法を SA〜SF の 6 シナリオで比較する:
  (A) ESN 残差（Phase 13 / command_test.py と同設定の 3ch ESN）
  (B) 物理推定器 "ff" モード     — residual = smooth(load - I_ff(pos))
  (C) 物理推定器 "full" モード   — residual = smooth(load - I_ff(pos) - I_pd_est)

評価指標:
  SA  正常基準       — 誤警報率（低い方が良い）
  SB  コマンドスイープ — 誤警報率（低い方が良い; 外乱なし）
  SC  定常外力        — 外乱区間検出率
  SD  外力ステップ変化 — 外乱区間検出率
  SE  摩擦増大        — 外乱区間検出率
  SF  複合外乱        — 外乱区間検出率

実行:
    uv run python -m esn_anomaly.servo.estim_eval
    make run-servo-estim
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.pyplot as plt
import matplotlib.patches as mpatches
import numpy as np

from esn_anomaly.detector import compute_threshold, smooth
from esn_anomaly.model import ESNConfig, ESNModel
from esn_anomaly.servo.command_test import (
    NOISE,
    SCENARIO_CYCLES,
    SEED,
    SMOOTH_WIN,
    WARMUP,
    _build_sa,
    _build_sb,
    _build_sc,
    _build_sd,
    _build_se,
    _build_sf,
    _make_periodic_profile,
)
from esn_anomaly.servo.data import (
    DEG_TO_MRAD,
    DisturbanceSegment,
    generate_servo_with_profile,
    normalize_cmd,
    normalize_servo_obs,
)
from esn_anomaly.servo.estimator import PhysicalEstimator

# ──── 定数 ────────────────────────────────────────────────────────────────────
TRAIN_CYCLES: int = 20
OUTPUT_DIR = Path("output")

# 物理推定器パラメータ
PHYS_SMOOTH_WIN: int = 20    # 残差スムージング窓幅
VEL_SMOOTH_WIN: int = 50     # 速度推定窓幅（ノイズ抑制: 11.5→1.6 deg/s）
VEL_THRESHOLD: float = 5.0   # 定常状態判定速度閾値 [deg/s]
SAT_FRACTION: float = 0.90   # 飽和判定割合


# ──── 訓練 ───────────────────────────────────────────────────────────────────

def _train_esn() -> tuple[ESNModel, np.ndarray, float, float]:
    """3ch ESN 訓練（command_test.py と同一設定）。"""
    rng = np.random.default_rng(SEED)
    profile = _make_periodic_profile(TRAIN_CYCLES)
    u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)

    u = normalize_servo_obs(u_raw)
    cmd = normalize_cmd(cmd_raw)

    X = np.column_stack([u[:-1, 0], u[:-1, 1], cmd[:-1]])
    y = u[1:]

    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED)
    model = ESNModel(config)
    model.fit(X, y)

    pred_tr = model.predict(X)
    e_pos  = np.abs(y - pred_tr)[:, 0]
    e_load = np.abs(y - pred_tr)[:, 1]
    s_pos  = smooth(e_pos,  SMOOTH_WIN)
    s_load = smooth(e_load, SMOOTH_WIN)

    thr_pos  = compute_threshold(s_pos[WARMUP:],  "3sigma")
    thr_load = compute_threshold(s_load[WARMUP:], "3sigma")

    return model, X[-WARMUP:], thr_pos, thr_load


def _train_estimators(
    u_raw_train: np.ndarray,
    cmd_raw_train: np.ndarray,
) -> tuple[PhysicalEstimator, PhysicalEstimator]:
    """物理推定器 ff / full を正常訓練データで閾値学習する。"""
    pos_train  = u_raw_train[:, 0]
    load_train = u_raw_train[:, 1]

    est_ff   = PhysicalEstimator(mode="ff",   smooth_win=PHYS_SMOOTH_WIN, vel_smooth_win=VEL_SMOOTH_WIN, vel_threshold=VEL_THRESHOLD, sat_fraction=SAT_FRACTION)
    est_full = PhysicalEstimator(mode="full",  smooth_win=PHYS_SMOOTH_WIN, vel_smooth_win=VEL_SMOOTH_WIN, vel_threshold=VEL_THRESHOLD, sat_fraction=SAT_FRACTION)

    est_ff.fit_threshold(pos_train, load_train, cmd_raw_train,  warmup=WARMUP)
    est_full.fit_threshold(pos_train, load_train, cmd_raw_train,  warmup=WARMUP)

    return est_ff, est_full


# ──── シナリオ評価 ────────────────────────────────────────────────────────────

def _exc_rate(mask: np.ndarray, warmup: int) -> float:
    """ウォームアップ後の閾値超過率 [%]。"""
    return float(mask[warmup:].mean() * 100)


def _dist_exc_rate(mask: np.ndarray, disturbances: list[DisturbanceSegment]) -> float:
    """外乱区間内の閾値超過率 [%]。区間がなければ nan。"""
    if not disturbances:
        return float("nan")
    n = len(mask)
    rates = []
    for d in disturbances:
        lo = max(WARMUP, d.start)
        hi = min(n, d.end)
        if lo < hi:
            rates.append(float(mask[lo:hi].mean() * 100))
    return float(np.mean(rates)) if rates else float("nan")


def _eval_scenario(
    name: str,
    u_raw: np.ndarray,
    cmd_raw: np.ndarray,
    disturbances: list[DisturbanceSegment],
    # ESN
    esn_model: ESNModel,
    esn_warmup_data: np.ndarray,
    thr_pos: float,
    thr_load: float,
    # 物理推定器
    est_ff: PhysicalEstimator,
    est_full: PhysicalEstimator,
) -> dict:
    """1 シナリオを 3 手法で評価し結果辞書を返す。"""
    pos  = u_raw[:, 0]
    load = u_raw[:, 1]

    # ── ESN ──
    u_norm = normalize_servo_obs(u_raw)
    cmd_norm = normalize_cmd(cmd_raw)
    X = np.column_stack([u_norm[:-1, 0], u_norm[:-1, 1], cmd_norm[:-1]])
    y = u_norm[1:]
    pred = esn_model.predict(X, warmup_data=esn_warmup_data)

    esn_s_pos  = smooth(np.abs(y - pred)[:, 0], SMOOTH_WIN)
    esn_s_load = smooth(np.abs(y - pred)[:, 1], SMOOTH_WIN)
    esn_mask   = (esn_s_pos > thr_pos) | (esn_s_load > thr_load)

    # ── 物理推定器 ff ──
    r_ff    = est_ff.abs_residual(pos, load)
    ff_mask = est_ff.detect(pos, load, cmd_raw)

    # ── 物理推定器 full ──
    r_full    = est_full.abs_residual(pos, load, cmd_raw)
    full_mask = est_full.detect(pos, load, cmd_raw)

    return dict(
        name=name,
        disturbances=disturbances,
        # ESN
        esn_exc   = _exc_rate(esn_mask,  WARMUP),
        esn_dist  = _dist_exc_rate(esn_mask, disturbances),
        esn_s_pos = esn_s_pos, esn_s_load = esn_s_load,
        thr_pos=thr_pos, thr_load=thr_load,
        # ff
        ff_exc  = _exc_rate(ff_mask,  WARMUP),
        ff_dist = _dist_exc_rate(ff_mask, disturbances),
        r_ff=r_ff, thr_ff=est_ff._threshold,
        # full
        full_exc  = _exc_rate(full_mask,  WARMUP),
        full_dist = _dist_exc_rate(full_mask, disturbances),
        r_full=r_full, thr_full=est_full._threshold,
        # 生データ
        pos=pos, load=load, cmd=cmd_raw,
    )


# ──── プロット ────────────────────────────────────────────────────────────────

def _plot_comparison(result: dict, path: Path) -> None:
    """3 手法の残差を並べた比較プロットを保存する。

    3 段構成:
      上: ESN pos + load 残差（2 本）
      中: 物理推定器 ff 残差
      下: 物理推定器 full 残差
    """
    path.parent.mkdir(parents=True, exist_ok=True)

    n = len(result["pos"])
    t = np.arange(n) * 0.01

    fig, axes = plt.subplots(3, 1, figsize=(14, 10), sharex=True)
    fig.suptitle(f"Estimator Comparison: {result['name']}", fontsize=11)

    def _shade(ax: plt.Axes) -> None:
        for d in result["disturbances"]:
            ax.axvspan(d.start * 0.01, (d.end - 1) * 0.01,
                       alpha=0.12, color="red", label="_nolegend_")

    def _shade_warmup(ax: plt.Axes) -> None:
        ax.axvspan(0, WARMUP * 0.01, alpha=0.10, color="gray", label="_nolegend_")

    # ESN 残差
    ax = axes[0]
    _shade_warmup(ax)
    _shade(ax)
    esn_s_pos  = result["esn_s_pos"]
    esn_s_load = result["esn_s_load"]
    t_esn = np.arange(len(esn_s_pos)) * 0.01
    ax.plot(t_esn, esn_s_pos,  color="navy",       lw=0.9, label="ESN pos residual")
    ax.plot(t_esn, esn_s_load, color="darkorange",  lw=0.9, label="ESN load residual")
    ax.axhline(result["thr_pos"],  color="navy",      ls="--", lw=1.0,
               label=f"pos thr {result['thr_pos']:.4f}")
    ax.axhline(result["thr_load"], color="darkorange", ls="--", lw=1.0,
               label=f"load thr {result['thr_load']:.4f}")
    ax.set_ylabel("ESN residual\n(normalized)")
    ax.legend(loc="upper right", fontsize=7)
    ax.grid(True, alpha=0.3)
    exc_str = f"{result['esn_exc']:.1f}%"
    dist_str = f"  dist:{result['esn_dist']:.1f}%" if not np.isnan(result['esn_dist']) else ""
    ax.set_title(f"(A) ESN — exc {exc_str}{dist_str}", fontsize=9, loc="left")

    # 物理推定器 ff
    ax = axes[1]
    _shade_warmup(ax)
    _shade(ax)
    r_ff = result["r_ff"]
    ax.plot(t[:len(r_ff)], r_ff, color="steelblue", lw=0.9, label="phys ff |residual|")
    ax.axhline(result["thr_ff"], color="steelblue", ls="--", lw=1.2,
               label=f"threshold {result['thr_ff']:.2f}")
    ax.set_ylabel("ff |residual| [mA]")
    ax.legend(loc="upper right", fontsize=7)
    ax.grid(True, alpha=0.3)
    exc_str = f"{result['ff_exc']:.1f}%"
    dist_str = f"  dist:{result['ff_dist']:.1f}%" if not np.isnan(result['ff_dist']) else ""
    ax.set_title(f"(B) Physical ff — exc {exc_str}{dist_str}", fontsize=9, loc="left")

    # 物理推定器 full
    ax = axes[2]
    _shade_warmup(ax)
    _shade(ax)
    r_full = result["r_full"]
    ax.plot(t[:len(r_full)], r_full, color="forestgreen", lw=0.9, label="phys full |residual|")
    ax.axhline(result["thr_full"], color="forestgreen", ls="--", lw=1.2,
               label=f"threshold {result['thr_full']:.2f}")
    ax.set_xlabel("Time [s]")
    ax.set_ylabel("full |residual| [mA]")
    ax.legend(loc="upper right", fontsize=7)
    ax.grid(True, alpha=0.3)
    exc_str = f"{result['full_exc']:.1f}%"
    dist_str = f"  dist:{result['full_dist']:.1f}%" if not np.isnan(result['full_dist']) else ""
    ax.set_title(f"(C) Physical full — exc {exc_str}{dist_str}", fontsize=9, loc="left")

    # 外乱凡例
    if result["disturbances"]:
        red_patch = mpatches.Patch(color="red", alpha=0.25, label="disturbance zone")
        axes[0].legend(
            handles=list(axes[0].get_legend_handles_labels()[0]) + [red_patch],
            labels=list(axes[0].get_legend_handles_labels()[1]) + ["disturbance zone"],
            loc="upper right", fontsize=7,
        )

    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── メイン ──────────────────────────────────────────────────────────────────

def run() -> None:
    print("=" * 72)
    print("Phase 14: 物理推定器 vs ESN 比較評価")
    print("=" * 72)

    # ── 訓練データ生成（物理推定器閾値学習用）──
    print("\n[1] 訓練データ生成 & 閾値学習")
    rng_train = np.random.default_rng(SEED)
    profile_train = _make_periodic_profile(TRAIN_CYCLES)
    u_raw_train, cmd_raw_train = generate_servo_with_profile(
        profile_train, noise=NOISE, rng=rng_train
    )

    # ESN 訓練
    esn_model, esn_warmup_data, thr_pos, thr_load = _train_esn()
    print(f"  ESN 閾値   pos: {thr_pos:.5f}  load: {thr_load:.5f}")

    # 物理推定器訓練
    est_ff, est_full = _train_estimators(u_raw_train, cmd_raw_train)
    print(f"  Phys ff 閾値:   {est_ff._threshold:.3f} mA")
    print(f"  Phys full 閾値: {est_full._threshold:.3f} mA")

    # ── シナリオ評価 ──
    scenarios_builders = [
        ("SA_Normal",      _build_sa),
        ("SB_CmdSweep",    _build_sb),
        ("SC_ConstForce",  _build_sc),
        ("SD_ForceStep",   _build_sd),
        ("SE_FrictionUp",  _build_se),
        ("SF_Combined",    _build_sf),
    ]

    print(f"\n[2] シナリオ評価 ({SCENARIO_CYCLES} cycles / scenario)\n")

    header = (
        f"{'Scenario':<18}  "
        f"{'ESN Exc':>8}  {'ESN Dist':>9}  "
        f"{'ff Exc':>8}  {'ff Dist':>9}  "
        f"{'full Exc':>8}  {'full Dist':>9}"
    )
    print(header)
    print("-" * 82)

    results = []
    for i, (name, builder) in enumerate(scenarios_builders):
        rng = np.random.default_rng(SEED + 200 + i)
        u_raw, cmd_raw, dist = builder(rng)

        result = _eval_scenario(
            name, u_raw, cmd_raw, dist,
            esn_model, esn_warmup_data, thr_pos, thr_load,
            est_ff, est_full,
        )
        results.append(result)

        def _fmt(exc: float, dist_exc: float) -> str:
            d = f"{dist_exc:.1f}%" if not np.isnan(dist_exc) else "    -"
            return f"{exc:>7.1f}%  {d:>9}"

        print(
            f"  {name:<16}  "
            + _fmt(result["esn_exc"],  result["esn_dist"])  + "  "
            + _fmt(result["ff_exc"],   result["ff_dist"])   + "  "
            + _fmt(result["full_exc"], result["full_dist"])
        )

        _plot_comparison(
            result,
            OUTPUT_DIR / f"result_estim_{name.lower()}.png",
        )

    # ── サマリ ──
    print("\n" + "=" * 72)
    print("サマリ（SB 誤警報率 ↓ が改善、SC-SE 外乱検出率 ↑ が良好）")
    print("-" * 72)
    print(f"{'シナリオ':<16}  {'ESN Exc':>9}  {'ff Exc':>9}  {'full Exc':>9}")
    for r in results:
        print(f"  {r['name']:<16}  {r['esn_exc']:>8.1f}%  {r['ff_exc']:>8.1f}%  {r['full_exc']:>8.1f}%")
    print("=" * 72)


if __name__ == "__main__":
    run()
