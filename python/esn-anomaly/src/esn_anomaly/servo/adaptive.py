"""エラー率フィードバックによる自動しきい値調整（Phase 16）。

更新則（自己適応型制御）::

    is_error=True  → threshold += alpha          （鈍感化）
    is_error=False → threshold -= alpha × target  （敏感化）

平衡点において P(is_error) ≈ target_rate が成立する。

使い方::

    from esn_anomaly.servo.adaptive import AdaptiveThreshold

    at = AdaptiveThreshold(
        initial=0.05,
        target_rate=0.03,
        hard_min=0.005,
        hard_max=0.25,
    )
    for residual in residual_stream:
        is_error = residual > at.threshold
        at.update(is_error)
"""

from __future__ import annotations


class AdaptiveThreshold:
    """エラー率フィードバックによる自動しきい値調整。

    更新則:
        is_error=True  → threshold += alpha
        is_error=False → threshold -= alpha × target_rate

    平衡点: P(is_error) ≈ target_rate

    Args:
        initial:     初期しきい値（ベースラインモデルの値を使用）
        target_rate: 目標誤警報率（例: 0.03 = 3%）
        alpha:       更新ステップ幅（None なら initial × 0.01）
        hard_min:    物理下限（これ以下には下がらない; None なら initial × 0.05）
        hard_max:    物理上限（これ以上には上がらない; None なら制限なし）
    """

    def __init__(
        self,
        initial: float,
        target_rate: float = 0.03,
        alpha: float | None = None,
        hard_min: float | None = None,
        hard_max: float | None = None,
    ) -> None:
        self.threshold = initial
        self._initial = initial
        self.target_rate = target_rate
        self.alpha = alpha if alpha is not None else initial * 0.01
        # hard_min のデフォルト: initial の 5%（ゼロへの縮退防止）
        self.hard_min = hard_min if hard_min is not None else initial * 0.05
        self.hard_max = hard_max
        self._n_errors: int = 0
        self._n_total: int = 0

    def update(self, is_error: bool) -> float:
        """1 ステップ処理してしきい値を返す。

        Args:
            is_error: 現在しきい値でエラー判定された場合 True

        Returns:
            更新後のしきい値
        """
        self._n_total += 1
        if is_error:
            self._n_errors += 1
            self.threshold += self.alpha
        else:
            self.threshold -= self.alpha * self.target_rate

        self.threshold = max(self.threshold, self.hard_min)
        if self.hard_max is not None:
            self.threshold = min(self.threshold, self.hard_max)
        return self.threshold

    @property
    def error_rate(self) -> float:
        """現在の累積エラー率（サンプルがない場合は 0.0）。"""
        return self._n_errors / self._n_total if self._n_total > 0 else 0.0

    def reset(self) -> None:
        """しきい値とカウンタを初期状態に戻す。"""
        self.threshold = self._initial
        self._n_errors = 0
        self._n_total = 0

    def __repr__(self) -> str:
        return (
            f"AdaptiveThreshold(threshold={self.threshold:.6f}, "
            f"target={self.target_rate:.3f}, "
            f"error_rate={self.error_rate:.3f}, "
            f"n={self._n_total})"
        )
