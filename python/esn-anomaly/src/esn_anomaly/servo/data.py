"""サーボモータ物理モデルとデータ生成。

仕様:
  - 動作範囲: -90〜+90 度
  - 安定平衡点: -45 度（重力＋ギア抵抗）
  - 最大電流: 100 mA
  - 最大到達角度: 負方向 -80 度（平衡点から 35 度）、正方向 +20 度（平衡点から 65 度）

物理モデル:
  非対称ばね＋粘性ダンピング。
    T_restore(θ) = -K_neg × (θ - θ_eq)  (θ < θ_eq)
    T_restore(θ) = -K_pos × (θ - θ_eq)  (θ ≥ θ_eq)
  最大電流での釣り合い条件:
    T_motor_max = K_neg × 35 = K_pos × 65 = 1.0（正規化）
  これにより K_neg = 1/35、K_pos = 1/65。
  正方向の剛性が低い（65 度まで届く）、負方向が高い（35 度まで）。
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np
from numpy.typing import NDArray


@dataclass
class ServoParams:
    """サーボモータのパラメータ。"""

    # 機械的限界 [度]
    theta_min: float = -90.0
    theta_max: float = +90.0
    # 安定平衡点 [度]
    theta_eq: float = -45.0
    # 最大電流で到達可能な限界 [度]
    theta_reach_neg: float = -80.0
    theta_reach_pos: float = +20.0
    # 最大電流 [mA]
    current_max: float = 100.0
    # 慣性モーメント（正規化）
    inertia: float = 0.01
    # 粘性ダンピング係数
    damping: float = 0.08
    # サンプリング間隔 [s] (出力レート)
    dt: float = 0.01
    # 内部積分ステップ数（1出力ステップあたり）
    substeps: int = 10


class ServoModel:
    """サーボモータ物理シミュレーション（非対称ばね＋粘性ダンピング）。

    状態: (θ [度], ω [度/s])
    入力: I [mA]（-current_max 〜 +current_max）

    Usage::

        model = ServoModel()
        theta, omega = model.step(current_mA)
        model.reset()
    """

    def __init__(self, params: ServoParams | None = None) -> None:
        self.p = params or ServoParams()

        # 非対称ばね定数（最大電流トルク = 1.0 に正規化）
        reach_neg = self.p.theta_eq - self.p.theta_reach_neg  # 35 度
        reach_pos = self.p.theta_reach_pos - self.p.theta_eq  # 65 度
        self.k_neg: float = 1.0 / reach_neg
        self.k_pos: float = 1.0 / reach_pos
        # 電流→モータートルク変換 [torque/mA]
        self.motor_gain: float = 1.0 / self.p.current_max

        # 初期状態
        self.theta: float = self.p.theta_eq
        self.omega: float = 0.0

    # ------------------------------------------------------------------
    # 物理計算
    # ------------------------------------------------------------------

    def restoring_torque(self, theta: float) -> float:
        """安定点への復元トルクを返す。

        正の値 = 正方向トルク。
        """
        delta = theta - self.p.theta_eq
        if delta >= 0.0:
            return -self.k_pos * delta
        else:
            return -self.k_neg * delta

    def _derivatives(self, theta: float, omega: float, I: float) -> tuple[float, float]:
        """状態微分 (dθ/dt, dω/dt) を計算する。"""
        T_motor = self.motor_gain * I
        T_restore = self.restoring_torque(theta)
        T_damp = -self.p.damping * omega
        alpha = (T_motor + T_restore + T_damp) / self.p.inertia
        return omega, alpha

    def _rk4_step(self, theta: float, omega: float, I: float, dt: float) -> tuple[float, float]:
        """4次 Runge-Kutta 積分（固定電流入力）。"""
        dth1, dom1 = self._derivatives(theta, omega, I)
        dth2, dom2 = self._derivatives(theta + 0.5 * dt * dth1, omega + 0.5 * dt * dom1, I)
        dth3, dom3 = self._derivatives(theta + 0.5 * dt * dth2, omega + 0.5 * dt * dom2, I)
        dth4, dom4 = self._derivatives(theta + dt * dth3, omega + dt * dom3, I)
        new_theta = theta + dt / 6.0 * (dth1 + 2 * dth2 + 2 * dth3 + dth4)
        new_omega = omega + dt / 6.0 * (dom1 + 2 * dom2 + 2 * dom3 + dom4)
        return new_theta, new_omega

    def _apply_limits(self, theta: float, omega: float) -> tuple[float, float]:
        """機械的限界に当たった場合に速度を 0 にクリップする。"""
        if theta <= self.p.theta_min:
            return self.p.theta_min, max(0.0, omega)
        if theta >= self.p.theta_max:
            return self.p.theta_max, min(0.0, omega)
        return theta, omega

    # ------------------------------------------------------------------
    # 公開インタフェース
    # ------------------------------------------------------------------

    def step(self, current: float) -> tuple[float, float]:
        """1 サンプル（dt 秒）分を進めて (θ, ω) を返す。

        Args:
            current: 入力電流 [mA]（クリッピングあり）

        Returns:
            (theta [度], omega [度/s])
        """
        I = float(np.clip(current, -self.p.current_max, self.p.current_max))
        dt_sub = self.p.dt / self.p.substeps
        th, om = self.theta, self.omega
        for _ in range(self.p.substeps):
            th, om = self._rk4_step(th, om, I, dt_sub)
            th, om = self._apply_limits(th, om)
        self.theta, self.omega = th, om
        return self.theta, self.omega

    def reset(self, theta: float | None = None, omega: float = 0.0) -> None:
        """状態をリセットする。

        Args:
            theta: 初期角度 [度]。None なら安定平衡点。
            omega: 初期角速度 [度/s]。
        """
        self.theta = self.p.theta_eq if theta is None else float(theta)
        self.omega = float(omega)


# ------------------------------------------------------------------
# 単位変換
# ------------------------------------------------------------------

DEG_TO_MRAD: float = np.pi / 180.0 * 1000.0  # ≈ 17.453 mrad/deg


def _deg_to_mrad(deg: float) -> float:
    """度 → ミリラジアン。"""
    return deg * DEG_TO_MRAD


def _mrad_to_deg(mrad: float) -> float:
    """ミリラジアン → 度。"""
    return mrad / DEG_TO_MRAD


# ------------------------------------------------------------------
# 位置制御器
# ------------------------------------------------------------------

@dataclass
class ControllerParams:
    """ServoController の制御ゲイン。"""

    kp: float = 5.0   # 位置比例ゲイン
    kd: float = 0.2   # 速度微分ゲイン


class ServoController:
    """前向き補償付き PD 位置制御器。

    `ServoModel`（度・電流制御）をラップし、**ミリラジアン** の位置指令を受けて
    ``(pos_mrad, load_mA)`` を返す。

    制御則::

        I_ff = -T_restore(θ_target) / motor_gain  # 保持電流（重力補償）
        I_pd = Kp × (θ_target - θ) - Kd × ω
        I_cmd = clip(I_ff + I_pd, -current_max, +current_max)

    ``load_mA = I_cmd`` がモーターの実効負荷電流を表す。

    Usage::

        ctrl = ServoController()
        pos_mrad, load_mA = ctrl.step_with_target(target_mrad)
        ctrl.reset()
    """

    def __init__(
        self,
        servo_params: ServoParams | None = None,
        ctrl_params: ControllerParams | None = None,
    ) -> None:
        self._servo = ServoModel(servo_params or ServoParams())
        self._cp = ctrl_params or ControllerParams()

    @property
    def servo(self) -> ServoModel:
        return self._servo

    def _feedforward(self, target_deg: float) -> float:
        """目標角度の保持に必要な前向き補償電流 [mA]。"""
        return -self._servo.restoring_torque(target_deg) / self._servo.motor_gain

    def step_with_target(self, target_mrad: float) -> tuple[float, float]:
        """1 サンプル分を進めて (pos_mrad, load_mA) を返す。

        Args:
            target_mrad: 目標位置 [mrad]

        Returns:
            (pos_mrad, load_mA):
              pos_mrad — 現在角度 [mrad]（ノイズなし真値）
              load_mA  — モーター電流指令値 [mA]（= 実効負荷）
        """
        target_deg = _mrad_to_deg(target_mrad)
        theta = self._servo.theta
        omega = self._servo.omega

        I_ff = self._feedforward(target_deg)
        I_pd = self._cp.kp * (target_deg - theta) - self._cp.kd * omega
        I_cmd = float(np.clip(I_ff + I_pd, -self._servo.p.current_max, self._servo.p.current_max))

        theta_new, _ = self._servo.step(I_cmd)
        return _deg_to_mrad(theta_new), I_cmd

    def reset(self, target_mrad: float | None = None) -> None:
        """状態をリセットする。

        Args:
            target_mrad: リセット後の初期位置 [mrad]。None なら安定平衡点。
        """
        theta_deg = _mrad_to_deg(target_mrad) if target_mrad is not None else None
        self._servo.reset(theta=theta_deg)


# ------------------------------------------------------------------
# 観測ノイズ・モーションパターン
# ------------------------------------------------------------------

@dataclass
class MeasurementNoise:
    """観測ノイズの標準偏差。"""

    pos_std: float = 2.0    # 位置ノイズ [mrad]  ≈ 0.11°
    load_std: float = 3.0   # 負荷ノイズ [mA]


@dataclass
class MotionPattern:
    """定期往復モーションの目標値と継続ステップ数。

    1サイクル構成（デフォルト 1300 step ≈ 13 s @ 100 Hz）::

        hold_home → move+hold_up → move+hold_home
                  → move+hold_down → move+hold_home
    """

    home_mrad: float = -785.4    # 安定平衡点 -45°
    up_mrad: float = 0.0         # 上位目標   0°
    down_mrad: float = -1221.7   # 下位目標  -70°
    hold_steps: int = 100        # 各保持フェーズのステップ数
    move_steps: int = 200        # 各移動フェーズのステップ数（コントローラが追従）

    @property
    def steps_per_cycle(self) -> int:
        """1 サイクルのステップ数。"""
        # home_hold + (move_up + hold_up) + (move_home + hold_home)
        # + (move_down + hold_down) + (move_home + hold_home)
        return self.hold_steps + 4 * (self.move_steps + self.hold_steps)


def _build_target_profile(pattern: MotionPattern, n_cycles: int) -> NDArray[np.float64]:
    """定期往復モーションの目標値配列を生成する。

    Returns:
        shape (n_cycles × steps_per_cycle,) の目標 mrad 配列
    """
    h = pattern.hold_steps
    m = pattern.move_steps

    one_cycle = np.concatenate([
        np.full(h, pattern.home_mrad),                  # home 保持
        np.full(m + h, pattern.up_mrad),                # up 移動+保持
        np.full(m + h, pattern.home_mrad),              # home 移動+保持
        np.full(m + h, pattern.down_mrad),              # down 移動+保持
        np.full(m + h, pattern.home_mrad),              # home 移動+保持
    ])
    return np.tile(one_cycle, n_cycles)


# ------------------------------------------------------------------
# 正規化
# ------------------------------------------------------------------

#: 位置正規化レンジ [mrad]（ServoParams のデフォルト -90°〜+90° に対応）
POS_MRAD_MIN: float = -90.0 * DEG_TO_MRAD   # ≈ -1570.8
POS_MRAD_MAX: float = +90.0 * DEG_TO_MRAD   # ≈ +1570.8

#: 負荷正規化レンジ [mA]
LOAD_MA_MIN: float = -100.0
LOAD_MA_MAX: float = +100.0


def normalize_servo_obs(u: NDArray[np.float64]) -> NDArray[np.float64]:
    """サーボ観測値 (pos_mrad, load_mA) を [-1, 1] に正規化する。

    Args:
        u: shape (n, 2)  列 0 = pos [mrad], 列 1 = load [mA]

    Returns:
        shape (n, 2)  両列とも [-1, 1] に正規化済み
    """
    out = np.empty_like(u)
    pos_center = (POS_MRAD_MAX + POS_MRAD_MIN) / 2.0
    pos_scale = (POS_MRAD_MAX - POS_MRAD_MIN) / 2.0
    load_center = (LOAD_MA_MAX + LOAD_MA_MIN) / 2.0
    load_scale = (LOAD_MA_MAX - LOAD_MA_MIN) / 2.0
    out[:, 0] = (u[:, 0] - pos_center) / pos_scale
    out[:, 1] = (u[:, 1] - load_center) / load_scale
    return out


# ------------------------------------------------------------------
# 定期往復モーション生成
# ------------------------------------------------------------------

def generate_servo_periodic(
    n_cycles: int = 20,
    pattern: MotionPattern | None = None,
    noise: MeasurementNoise | None = None,
    params: ServoParams | None = None,
    ctrl: ControllerParams | None = None,
    rng: np.random.Generator | None = None,
) -> NDArray[np.float64]:
    """定期往復モーションのサーボ観測時系列を生成する。

    サーボは home → up → home → down → home を ``n_cycles`` 回繰り返す。
    観測値には Gaussian ノイズを付加できる。

    Args:
        n_cycles: 繰り返し回数
        pattern: モーションパターン（None なら既定値）
        noise: 観測ノイズ（None ならノイズなし）
        params: サーボ物理パラメータ
        ctrl: 制御ゲイン
        rng: 乱数ジェネレータ（ノイズ再現性のために指定）

    Returns:
        shape (n_cycles × steps_per_cycle, 2):
          列 0 = pos [mrad]（ノイズあり）
          列 1 = load [mA]（ノイズあり）

    ESN 訓練への接続例::

        u = generate_servo_periodic(n_cycles=20)
        X_train = u[:-1]   # (n-1, 2)
        y_train = u[1:]    # (n-1, 2)
    """
    if rng is None:
        rng = np.random.default_rng()
    pat = pattern or MotionPattern()
    controller = ServoController(params, ctrl)
    controller.reset(target_mrad=pat.home_mrad)

    targets = _build_target_profile(pat, n_cycles)
    n_steps = len(targets)

    pos_arr = np.empty(n_steps)
    load_arr = np.empty(n_steps)

    for i, tgt in enumerate(targets):
        pos_mrad, load_mA = controller.step_with_target(float(tgt))
        pos_arr[i] = pos_mrad
        load_arr[i] = load_mA

    if noise is not None:
        pos_arr += rng.normal(0.0, noise.pos_std, n_steps)
        load_arr += rng.normal(0.0, noise.load_std, n_steps)

    return np.stack([pos_arr, load_arr], axis=1).astype(np.float64)


# ------------------------------------------------------------------
# 異常注入
# ------------------------------------------------------------------

#: 異常種別定数
ANOMALY_FORCE = "force"           # 外力: 負荷チャンネルに一定オフセット
ANOMALY_POS_SPIKE = "pos_spike"   # 位置センサスパイク: 突発的な位置誤差
ANOMALY_POS_DRIFT = "pos_drift"   # 位置センサドリフト: 徐々に増大する位置誤差
ANOMALY_LOAD_STUCK = "load_stuck" # 負荷センサ固着: 区間先頭値で固定


@dataclass(frozen=True)
class ServoAnomalySegment:
    """注入済み異常セグメントのメタデータ。"""

    start: int      # 開始インデックス (inclusive)
    end: int        # 終了インデックス (exclusive)
    kind: str       # ANOMALY_* 定数のいずれか
    magnitude: float  # 異常の大きさ（種別により解釈が異なる）

    @property
    def length(self) -> int:
        return self.end - self.start


def inject_anomaly_segment(
    u_raw: NDArray[np.float64],
    seg: ServoAnomalySegment,
    rng: np.random.Generator,
) -> NDArray[np.float64]:
    """1 つの異常セグメントを生データに注入する。

    Args:
        u_raw: shape (n, 2) のサーボ観測データ（pos_mrad, load_mA）
        seg: 注入する異常セグメント情報
        rng: 乱数ジェネレータ（スパイク位置の決定に使用）

    Returns:
        shape (n, 2): 異常を注入した新しい配列（コピー）
    """
    u = u_raw.copy()
    s, e = seg.start, seg.end
    n = e - s

    if seg.kind == ANOMALY_FORCE:
        # 外力: 負荷チャンネルに一定オフセット
        u[s:e, 1] += seg.magnitude
    elif seg.kind == ANOMALY_POS_SPIKE:
        # 位置センサスパイク: ランダムなタイミングで突発値を追加
        n_spikes = max(1, n // 20)
        idx = rng.choice(n, size=n_spikes, replace=False)
        signs = rng.choice([-1.0, 1.0], size=n_spikes)
        u[s + idx, 0] += seg.magnitude * signs
    elif seg.kind == ANOMALY_POS_DRIFT:
        # 位置センサドリフト: 徐々に増大するオフセット
        u[s:e, 0] += np.linspace(0.0, seg.magnitude, n)
    elif seg.kind == ANOMALY_LOAD_STUCK:
        # 負荷センサ固着: 区間先頭値で全区間を固定
        u[s:e, 1] = u[s, 1]
    else:
        raise ValueError(f"未知の異常種別: {seg.kind!r}")

    return u


#: サーボ異常注入のデフォルト強度
_DEFAULT_MAGNITUDES: dict[str, float] = {
    ANOMALY_FORCE: 30.0,         # 30 mA（最大の 30%）
    ANOMALY_POS_SPIKE: 200.0,    # 200 mrad（約 11.5°）
    ANOMALY_POS_DRIFT: 300.0,    # 300 mrad（約 17°）
    ANOMALY_LOAD_STUCK: 0.0,     # 未使用（固着値はデータから自動決定）
}


def generate_servo_anomaly_test(
    n_cycles: int = 10,
    anomaly_kinds: list[str] | None = None,
    rate_hz: float = 0.05,
    seg_len_lo: int = 50,
    seg_len_hi: int = 200,
    magnitudes: dict[str, float] | None = None,
    pattern: MotionPattern | None = None,
    noise: MeasurementNoise | None = None,
    params: ServoParams | None = None,
    ctrl: ControllerParams | None = None,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], list[ServoAnomalySegment]]:
    """Poisson 間隔で異常セグメントを埋め込んだサーボ観測時系列を生成する。

    Args:
        n_cycles: モーションサイクル繰り返し数
        anomaly_kinds: 注入する異常種別のリスト（ループで使用）。
            None または空リストのとき正常データのみ返す。
        rate_hz: 異常セグメントの平均発生頻度 [Hz]
        seg_len_lo: セグメント長の下限 [step]
        seg_len_hi: セグメント長の上限 [step]
        magnitudes: 種別ごとの強度辞書（None なら _DEFAULT_MAGNITUDES を使用）
        pattern: モーションパターン
        noise: 観測ノイズ
        params: サーボ物理パラメータ
        ctrl: 制御ゲイン
        rng: 乱数ジェネレータ

    Returns:
        (u_anomaly, segments):
          u_anomaly — shape (n, 2) サーボ観測（異常注入後）
          segments  — 注入した ServoAnomalySegment のリスト
    """
    if rng is None:
        rng = np.random.default_rng()

    mags = dict(_DEFAULT_MAGNITUDES)
    if magnitudes is not None:
        mags.update(magnitudes)

    pat = pattern or MotionPattern()
    # 基本正常データを生成（ノイズ込み）
    u_raw = generate_servo_periodic(
        n_cycles=n_cycles,
        pattern=pat,
        noise=noise,
        params=params,
        ctrl=ctrl,
        rng=rng,
    )

    if not anomaly_kinds:
        return u_raw, []

    n_steps = len(u_raw)
    # サンプリング周波数 = 1/dt
    fs = 1.0 / (params.dt if params is not None else ServoParams().dt)
    mean_interval = fs / rate_hz  # Poisson 平均間隔 [step]

    segments: list[ServoAnomalySegment] = []
    kind_idx = 0
    pos = int(rng.exponential(mean_interval))

    while pos < n_steps:
        slen = int(rng.integers(seg_len_lo, seg_len_hi + 1))
        end = min(pos + slen, n_steps)
        kind = anomaly_kinds[kind_idx % len(anomaly_kinds)]
        kind_idx += 1

        seg = ServoAnomalySegment(
            start=pos,
            end=end,
            kind=kind,
            magnitude=mags[kind],
        )
        u_raw = inject_anomaly_segment(u_raw, seg, rng)
        segments.append(seg)

        pos = end + int(rng.exponential(mean_interval))

    return u_raw, segments


# ------------------------------------------------------------------
# データ生成
# ------------------------------------------------------------------

def _normalize_theta(theta: NDArray[np.float64], theta_min: float = -90.0, theta_max: float = 90.0) -> NDArray[np.float64]:
    """角度を [-1, 1] に正規化する。"""
    center = (theta_max + theta_min) / 2.0
    scale = (theta_max - theta_min) / 2.0
    return (theta - center) / scale


def _normalize_current(current: NDArray[np.float64], current_max: float = 100.0) -> NDArray[np.float64]:
    """電流を [-1, 1] に正規化する。"""
    return current / current_max


def generate_servo_step(
    n_steps: int = 1000,
    params: ServoParams | None = None,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.float64]]:
    """ランダムなステップ電流入力を与えたときのサーボ応答を生成する。

    Args:
        n_steps: 生成ステップ数
        params: サーボパラメータ（None なら既定値）
        rng: 乱数ジェネレータ

    Returns:
        (u, y):
          u shape (n_steps, 1) — 正規化電流入力 [-1, 1]
          y shape (n_steps, 1) — 正規化角度出力 [-1, 1]
    """
    if rng is None:
        rng = np.random.default_rng()
    p = params or ServoParams()
    servo = ServoModel(p)

    # ランダムステップ: 80〜200 step ごとに電流値を切り替え
    currents = np.empty(n_steps)
    pos = 0
    while pos < n_steps:
        width = int(rng.integers(80, 201))
        value = float(rng.uniform(-p.current_max, p.current_max))
        end = min(pos + width, n_steps)
        currents[pos:end] = value
        pos = end

    thetas = np.empty(n_steps)
    for i in range(n_steps):
        theta, _ = servo.step(currents[i])
        thetas[i] = theta

    u = _normalize_current(currents, p.current_max).reshape(-1, 1)
    y = _normalize_theta(thetas, p.theta_min, p.theta_max).reshape(-1, 1)
    return u, y


def generate_servo_train(
    n_steps: int = 5000,
    params: ServoParams | None = None,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.float64]]:
    """ESN 学習用の訓練データを生成する。

    正規化電流 u(t) を入力、次ステップの正規化角度 y(t) を出力とする。
    ESN は u(t) → θ(t) のマッピングを学習する（1 ステップ遅延なし）。

    Returns:
        (X_train, y_train):
          X_train shape (n_steps, 2) — [正規化電流, 正規化角度（前ステップ）]
          y_train shape (n_steps, 1) — 正規化角度（現ステップ）
    """
    if rng is None:
        rng = np.random.default_rng()
    u, y = generate_servo_step(n_steps, params, rng)
    # X = [I(t), θ(t-1)], y = θ(t)
    theta_prev = np.roll(y, 1, axis=0)
    theta_prev[0] = _normalize_theta(
        np.array([(params or ServoParams()).theta_eq]),
        (params or ServoParams()).theta_min,
        (params or ServoParams()).theta_max,
    )
    X = np.concatenate([u, theta_prev], axis=1)
    return X, y


def generate_servo_test_scenario(
    scenario: str = "step",
    n_steps: int = 1000,
    params: ServoParams | None = None,
    rng: np.random.Generator | None = None,
) -> dict:
    """テスト用シナリオデータを生成する。

    Args:
        scenario:
          "step"     — ステップ入力（±100 mA を 4 段階）
          "sine"     — 正弦波電流入力
          "max_neg"  — 最大電流を負方向に継続（-80 度付近に収束）
          "max_pos"  — 最大電流を正方向に継続（+20 度付近に収束）
          "free"     — 電流 0（自然に安定点 -45 度へ収束）
        n_steps: 生成ステップ数
        params: サーボパラメータ

    Returns:
        dict with keys:
          "t"       — 時刻配列 shape (n_steps,)
          "theta"   — 角度 [度] shape (n_steps,)
          "omega"   — 角速度 [度/s] shape (n_steps,)
          "current" — 電流 [mA] shape (n_steps,)
    """
    if rng is None:
        rng = np.random.default_rng()
    p = params or ServoParams()
    servo = ServoModel(p)

    t = np.arange(n_steps) * p.dt

    if scenario == "step":
        # 各 1/4 区間で ±100mA と ±50mA を順番に
        quarter = n_steps // 4
        vals = [-p.current_max, p.current_max, -p.current_max * 0.5, p.current_max * 0.5]
        current = np.concatenate([np.full(quarter, v) for v in vals])[:n_steps]
    elif scenario == "sine":
        current = p.current_max * np.sin(2 * np.pi * 0.5 * t)
    elif scenario == "max_neg":
        current = np.full(n_steps, -p.current_max)
    elif scenario == "max_pos":
        current = np.full(n_steps, p.current_max)
    elif scenario == "free":
        # 初期位置を +20 度（最大正方向）に設定し電流 0 で自然減衰
        servo.reset(theta=20.0)
        current = np.zeros(n_steps)
    else:
        raise ValueError(f"未知のシナリオ: {scenario!r}")

    thetas = np.empty(n_steps)
    omegas = np.empty(n_steps)
    for i in range(n_steps):
        theta, omega = servo.step(current[i])
        thetas[i] = theta
        omegas[i] = omega

    return {
        "t": t,
        "theta": thetas,
        "omega": omegas,
        "current": current,
    }
