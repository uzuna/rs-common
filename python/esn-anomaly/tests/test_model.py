"""model.py のユニットテスト。"""

import numpy as np
import pytest

from esn_anomaly.waveform.data import generate_train_data
from esn_anomaly.model import ESNConfig, ESNModel


@pytest.fixture
def trained_model():
    """訓練済みの ESNModel を返すフィクスチャ。"""
    rng = np.random.default_rng(0)
    X, y = generate_train_data(n_steps=2000, rng=rng)
    model = ESNModel(ESNConfig(units=100, seed=0))
    model.fit(X, y)
    return model, X, y


class TestESNModel:
    def test_fit_returns_self(self):
        X, y = generate_train_data(500)
        model = ESNModel(ESNConfig(units=50, seed=0))
        result = model.fit(X, y)
        assert result is model

    def test_is_fitted(self):
        X, y = generate_train_data(500)
        model = ESNModel(ESNConfig(units=50, seed=0))
        assert not model.is_fitted
        model.fit(X, y)
        assert model.is_fitted

    def test_predict_output_shape(self, trained_model):
        model, X, _ = trained_model
        pred = model.predict(X)
        assert pred.shape == X.shape

    def test_predict_rmse_on_train(self, trained_model):
        """訓練データの RMSE が 0.1 以下であること。"""
        model, X, y = trained_model
        pred = model.predict(X)
        rmse = float(np.sqrt(np.mean((y - pred) ** 2)))
        assert rmse < 0.1, f"訓練 RMSE {rmse:.4f} が 0.1 を超えています"

    def test_predict_deterministic(self, trained_model):
        """リセット後の推論が冪等であること。"""
        model, X, _ = trained_model
        pred1 = model.predict(X[:50])
        pred2 = model.predict(X[:50])
        np.testing.assert_array_equal(pred1, pred2)

    def test_predict_before_fit_raises(self):
        model = ESNModel()
        X = np.random.randn(10, 1)
        with pytest.raises(RuntimeError):
            model.predict(X)
