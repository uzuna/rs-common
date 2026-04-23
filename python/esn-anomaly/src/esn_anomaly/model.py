"""ESNモデルの構築・学習・推論。

reservoirpy 0.4.x の Reservoir >> Ridge パイプラインを用いる。
"""

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray
from reservoirpy.nodes import Reservoir, Ridge


@dataclass
class ESNConfig:
    """ESN のハイパーパラメータ。"""

    units: int = 200
    sr: float = 0.9
    lr: float = 0.3
    ridge: float = 1e-6
    warmup: int = 100
    seed: int | None = 42


class ESNModel:
    """Reservoir >> Ridge 構成の ESN ラッパー。

    Usage:
        model = ESNModel()
        model.fit(X_train, y_train)
        y_pred = model.predict(X_test)
    """

    def __init__(self, config: ESNConfig | None = None) -> None:
        self.config = config or ESNConfig()
        self._reservoir = Reservoir(
            units=self.config.units,
            sr=self.config.sr,
            lr=self.config.lr,
            seed=self.config.seed,
        )
        self._readout = Ridge(ridge=self.config.ridge)
        self._esn = self._reservoir >> self._readout
        self._fitted = False

    def fit(
        self,
        X_train: NDArray[np.float64],
        y_train: NDArray[np.float64],
    ) -> "ESNModel":
        """Readout 層をリッジ回帰で学習する。

        Args:
            X_train: 入力系列 shape (n, 1)
            y_train: 教師系列 shape (n, 1)

        Returns:
            self（メソッドチェーン用）
        """
        self._esn.fit(X_train, y_train, warmup=self.config.warmup)
        self._reservoir.reset()
        self._fitted = True
        return self

    def predict(
        self,
        X: NDArray[np.float64],
        warmup_data: NDArray[np.float64] | None = None,
    ) -> NDArray[np.float64]:
        """リザーバ状態をリセットしてから逐次推論する。

        Args:
            X: 入力系列 shape (n, 1)
            warmup_data: 事前にリザーバを安定させるためのデータ shape (m, 1)。
                         推論結果には含まれない。

        Returns:
            予測系列 shape (n, 1)
        """
        if not self._fitted:
            raise RuntimeError("モデルが未学習です。fit() を先に呼び出してください。")
        self._reservoir.reset()
        if warmup_data is not None and len(warmup_data) > 0:
            self._esn.run(warmup_data)
        return self._esn.run(X)

    @property
    def is_fitted(self) -> bool:
        return self._fitted
