"""コマンドプロファイル制御 + 外力・関節抵抗シナリオ評価（Phase 13）。

3 チャンネル ESN (pos, load, cmd) を訓練し、
外力・クーロン摩擦が時変する 6 シナリオで挙動を評価する。

シナリオ:
  SA — 正常定期往復（3ch ベースライン）
  SB — コマンドスイープ（多目標ステップ変化、誤警報抑制効果を確認）
  SC — 定常外力（home 保持中に ext_torque=0.3）
  SD — 外力ステップ変化（往復中に突然 ext_torque=0.4 が加わる）
  SE — 摩擦増大（往復中に coulomb_friction=0.1 に変化）
  SF — 複合外乱（コマンドスイープ + 時変外力 + 時変摩擦）

実行:
    uv run python -m esn_anomaly.servo.command_test
    make run-servo-cmd
"""

from __future__ import annotations

from pathlib import Path

import matplotlib.pyplot as plt
import matplotlib.patches as mpatches
import numpy as np

from esn_anomaly.detector import compute_threshold, smooth
from esn_anomaly.model import ESNConfig, ESNModel
from esn_anomaly.servo.data import (
    CommandProfile,
    CommandSegment,
    DisturbanceSegment,
    MeasurementNoise,
    MotionPattern,
    POS_MRAD_MIN,
    POS_MRAD_MAX,
    generate_servo_with_profile,
    normalize_cmd,
    normalize_servo_obs,
)

# ──── 定数 ────────────────────────────────────────────────────────────────────
TRAIN_CYCLES: int = 20
SCENARIO_CYCLES: int = 10
WARMUP: int = 100
SMOOTH_WIN: int = 15
SEED: int = 42
OUTPUT_DIR = Path("output")

NOISE = MeasurementNoise(pos_std=2.0, load_std=3.0)


# ──── ヘルパー ────────────────────────────────────────────────────────────────

def _make_periodic_profile(n_cycles: int, pattern: MotionPattern | None = None) -> CommandProfile:
    """MotionPattern から CommandProfile を生成する（generate_servo_periodic と等価）。"""
    pat = pattern or MotionPattern()
    h, m = pat.hold_steps, pat.move_steps
    one_cycle = [
        CommandSegment("hold", pat.home_mrad,  h),
        CommandSegment("hold", pat.up_mrad,    m + h),
        CommandSegment("hold", pat.home_mrad,  m + h),
        CommandSegment("hold", pat.down_mrad,  m + h),
        CommandSegment("hold", pat.home_mrad,  m + h),
    ]
    return CommandProfile(one_cycle * n_cycles)


# ──── 訓練 ───────────────────────────────────────────────────────────────────

def _train() -> tuple[ESNModel, np.ndarray, float, float]:
    """3ch ESN を正常定期往復データで訓練し (model, warmup_X, thr_pos, thr_load) を返す。"""
    rng = np.random.default_rng(SEED)
    profile = _make_periodic_profile(TRAIN_CYCLES)
    u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)

    u = normalize_servo_obs(u_raw)
    cmd = normalize_cmd(cmd_raw)

    # 3ch 入力: (pos_t, load_t, cmd_t) → 2ch 出力: (pos_{t+1}, load_{t+1})
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

    rmse_pos  = float(np.sqrt(np.mean(e_pos[WARMUP:] ** 2)))
    rmse_load = float(np.sqrt(np.mean(e_load[WARMUP:] ** 2)))
    print(f"    訓練 RMSE  pos: {rmse_pos:.5f}  load: {rmse_load:.5f}")
    print(f"    閾値       pos: {thr_pos:.5f}  load: {thr_load:.5f}")

    return model, X[-WARMUP:], thr_pos, thr_load


# ──── シナリオ評価 ────────────────────────────────────────────────────────────

def _run_scenario(
    model: ESNModel,
    warmup_data: np.ndarray,
    thr_pos: float,
    thr_load: float,
    name: str,
    u_raw: np.ndarray,
    cmd_raw: np.ndarray,
    disturbances: list[DisturbanceSegment],
) -> dict:
    """1 シナリオを評価し結果辞書を返す。"""
    u = normalize_servo_obs(u_raw)
    cmd = normalize_cmd(cmd_raw)
    X = np.column_stack([u[:-1, 0], u[:-1, 1], cmd[:-1]])
    y = u[1:]

    pred = model.predict(X, warmup_data=warmup_data)

    e_pos  = np.abs(y - pred)[:, 0]
    e_load = np.abs(y - pred)[:, 1]
    s_pos  = smooth(e_pos,  SMOOTH_WIN)
    s_load = smooth(e_load, SMOOTH_WIN)

    # ウォームアップ後の閾値超過率（外乱区間含む全体）
    exc_mask = (s_pos[WARMUP:] > thr_pos) | (s_load[WARMUP:] > thr_load)
    exc_rate = float(exc_mask.mean() * 100)

    # 外乱区間のみでの超過率
    n = len(s_pos)
    dist_exc = []
    for d in disturbances:
        lo = max(WARMUP, d.start)
        hi = min(n, d.end)
        if lo < hi:
            seg_exc = (s_pos[lo:hi] > thr_pos) | (s_load[lo:hi] > thr_load)
            dist_exc.append(float(seg_exc.mean() * 100))
    avg_dist_exc = float(np.mean(dist_exc)) if dist_exc else float("nan")

    return dict(
        name=name,
        exc_rate=exc_rate,
        avg_dist_exc=avg_dist_exc,
        s_pos=s_pos, s_load=s_load,
        u_norm=y, pred=pred,
        cmd_norm=cmd[1:],
        disturbances=disturbances,
        thr_pos=thr_pos, thr_load=thr_load,
    )


# ──── シナリオ定義 ────────────────────────────────────────────────────────────

def _build_sa(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray, list[DisturbanceSegment]]:
    """SA: 正常定期往復（3ch ベースライン検証）"""
    profile = _make_periodic_profile(SCENARIO_CYCLES)
    u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)
    return u_raw, cmd_raw, []


def _build_sb(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray, list[DisturbanceSegment]]:
    """SB: コマンドスイープ（多目標ステップ変化で誤警報抑制効果を確認）"""
    pat = MotionPattern()
    hold = 300
    targets = [
        pat.home_mrad, pat.up_mrad, pat.home_mrad,
        pat.down_mrad, 0.0, -500.0, 200.0,
        pat.down_mrad, pat.home_mrad, pat.up_mrad,
        -200.0, pat.home_mrad,
    ]
    segs = [CommandSegment("hold", t, hold) for t in targets]
    profile = CommandProfile(segs * 3)
    u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)
    return u_raw, cmd_raw, []


def _build_sc(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray, list[DisturbanceSegment]]:
    """SC: 定常外力（home 保持中に ext_torque=0.3 を印加）"""
    pat = MotionPattern()
    profile = CommandProfile([
        CommandSegment("hold", pat.home_mrad,  500),   # 正常区間
        CommandSegment("hold", pat.home_mrad, 2000),   # 外力区間
        CommandSegment("hold", pat.home_mrad,  500),   # 回復区間
    ])
    dist = [DisturbanceSegment(500, 2500, ext_torque=0.3, coulomb_friction=0.0)]
    u_raw, cmd_raw = generate_servo_with_profile(profile, dist, noise=NOISE, rng=rng)
    return u_raw, cmd_raw, dist


def _build_sd(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray, list[DisturbanceSegment]]:
    """SD: 外力ステップ変化（往復動作中に突然 ext_torque=0.4 が加わる）"""
    pat = MotionPattern()
    profile = _make_periodic_profile(SCENARIO_CYCLES, pat)
    n_normal = 5 * pat.steps_per_cycle
    n_total  = SCENARIO_CYCLES * pat.steps_per_cycle
    dist = [DisturbanceSegment(n_normal, n_total, ext_torque=0.4, coulomb_friction=0.0)]
    u_raw, cmd_raw = generate_servo_with_profile(profile, dist, noise=NOISE, rng=rng)
    return u_raw, cmd_raw, dist


def _build_se(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray, list[DisturbanceSegment]]:
    """SE: 摩擦増大（往復動作中に coulomb_friction=0.1 に変化）"""
    pat = MotionPattern()
    profile = _make_periodic_profile(SCENARIO_CYCLES, pat)
    n_normal = 5 * pat.steps_per_cycle
    n_total  = SCENARIO_CYCLES * pat.steps_per_cycle
    dist = [DisturbanceSegment(n_normal, n_total, ext_torque=0.0, coulomb_friction=0.1)]
    u_raw, cmd_raw = generate_servo_with_profile(profile, dist, noise=NOISE, rng=rng)
    return u_raw, cmd_raw, dist


def _build_sf(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray, list[DisturbanceSegment]]:
    """SF: 複合外乱（コマンドスイープ + 時変外力 + 時変摩擦）"""
    pat = MotionPattern()
    profile = _make_periodic_profile(SCENARIO_CYCLES, pat)
    n_total = SCENARIO_CYCLES * pat.steps_per_cycle
    t1, t2, t3 = n_total // 4, n_total // 2, 3 * n_total // 4
    dist = [
        DisturbanceSegment(t1, t2, ext_torque=0.25, coulomb_friction=0.0),
        DisturbanceSegment(t2, t3, ext_torque=0.0,  coulomb_friction=0.08),
        DisturbanceSegment(t3, n_total, ext_torque=0.2, coulomb_friction=0.06),
    ]
    u_raw, cmd_raw = generate_servo_with_profile(profile, dist, noise=NOISE, rng=rng)
    return u_raw, cmd_raw, dist


# ──── プロット ────────────────────────────────────────────────────────────────

def _plot_scenario(result: dict, path: Path) -> None:
    """4 段グラフを保存する: pos 波形 / load 波形 / pos 残差 / load 残差。

    外乱区間は赤シェード、コマンド軌跡は点線でオーバーレイする。
    """
    path.parent.mkdir(parents=True, exist_ok=True)

    u = result["u_norm"]
    pred = result["pred"]
    s_pos = result["s_pos"]
    s_load = result["s_load"]
    cmd_norm = result["cmd_norm"]
    thr_pos = result["thr_pos"]
    thr_load = result["thr_load"]
    disturbances = result["disturbances"]
    name = result["name"]
    n = len(s_pos)
    t = np.arange(n) * 0.01

    fig, axes = plt.subplots(4, 1, figsize=(14, 14), sharex=True)
    fig.suptitle(f"ESN Servo Command Test: {name}", fontsize=11)

    def _shade(ax: plt.Axes) -> None:
        for d in disturbances:
            ax.axvspan(d.start * 0.01, (d.end - 1) * 0.01,
                       alpha=0.12, color="red", label="_nolegend_")

    def _shade_warmup(ax: plt.Axes) -> None:
        ax.axvspan(0, WARMUP * 0.01, alpha=0.1, color="gray", label="_nolegend_")

    # ── pos 波形 ──
    ax = axes[0]
    _shade_warmup(ax)
    _shade(ax)
    ax.plot(t, u[:, 0], color="steelblue", lw=0.6, alpha=0.8, label="pos actual")
    ax.plot(t, pred[:, 0], color="limegreen", lw=0.8, ls="--", alpha=0.9,
            label="pos predicted")
    ax.plot(t, cmd_norm, color="gray", lw=0.7, ls=":", alpha=0.7, label="cmd (target)")
    ax.set_ylabel("pos (normalized)")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── load 波形 ──
    ax = axes[1]
    _shade_warmup(ax)
    _shade(ax)
    ax.plot(t, u[:, 1], color="darkorange", lw=0.6, alpha=0.8, label="load actual")
    ax.plot(t, pred[:, 1], color="limegreen", lw=0.8, ls="--", alpha=0.9,
            label="load predicted")
    ax.set_ylabel("load (normalized)")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── pos 残差 ──
    ax = axes[2]
    _shade_warmup(ax)
    _shade(ax)
    ax.plot(t, s_pos, color="navy", lw=0.9, label="smoothed pos residual")
    ax.axhline(thr_pos, color="crimson", ls="--", lw=1.2,
               label=f"threshold {thr_pos:.4f}")
    ax.set_ylabel("pos residual")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # ── load 残差 ──
    ax = axes[3]
    _shade_warmup(ax)
    _shade(ax)
    ax.plot(t, s_load, color="saddlebrown", lw=0.9, label="smoothed load residual")
    ax.axhline(thr_load, color="crimson", ls="--", lw=1.2,
               label=f"threshold {thr_load:.4f}")
    ax.set_xlabel("Time [s]")
    ax.set_ylabel("load residual")
    ax.legend(loc="upper right", fontsize=8)
    ax.grid(True, alpha=0.3)

    # 外乱凡例を追加
    if disturbances:
        red_patch = mpatches.Patch(color="red", alpha=0.25, label="disturbance")
        axes[0].legend(
            handles=list(axes[0].get_legend_handles_labels()[0]) + [red_patch],
            labels=list(axes[0].get_legend_handles_labels()[1]) + ["disturbance"],
            loc="upper right", fontsize=8,
        )

    plt.tight_layout()
    plt.savefig(path, dpi=150)
    plt.close(fig)
    print(f"  Saved: {path}")


# ──── メイン ──────────────────────────────────────────────────────────────────

def run() -> None:
    print("=" * 68)
    print("ESN サーボ コマンドプロファイル制御 / 外力・摩擦シナリオ評価 (Phase 13)")
    print("=" * 68)

    print("\n[1] 3ch ESN 訓練 (pos, load, cmd 入力)")
    model, warmup_data, thr_pos, thr_load = _train()

    scenarios_builders = [
        ("SA_Normal",      _build_sa),
        ("SB_CmdSweep",    _build_sb),
        ("SC_ConstForce",  _build_sc),
        ("SD_ForceStep",   _build_sd),
        ("SE_FrictionUp",  _build_se),
        ("SF_Combined",    _build_sf),
    ]

    print(f"\n[2] シナリオ評価 ({SCENARIO_CYCLES} cycles / scenario)\n")
    print(f"{'Scenario':<18}  {'Exc.Rate':>10}  {'Dist.Exc':>10}  {'Disturbances'}")
    print("-" * 68)

    for i, (name, builder) in enumerate(scenarios_builders):
        rng = np.random.default_rng(SEED + 200 + i)
        u_raw, cmd_raw, dist = builder(rng)

        result = _run_scenario(
            model, warmup_data, thr_pos, thr_load,
            name, u_raw, cmd_raw, dist,
        )

        exc_str  = f"{result['exc_rate']:.1f}%"
        dist_str = (f"{result['avg_dist_exc']:.1f}%"
                    if not np.isnan(result["avg_dist_exc"]) else "-")
        dist_desc = ", ".join(
            f"[{d.start}:{d.end}] F={d.ext_torque:.2f} μ={d.coulomb_friction:.3f}"
            for d in dist
        ) or "none"
        print(f"  {name:<16}  {exc_str:>10}  {dist_str:>10}  {dist_desc}")

        _plot_scenario(result, OUTPUT_DIR / f"result_servo_cmd_{name.lower()}.png")

    print("\n" + "=" * 68)


if __name__ == "__main__":
    run()
