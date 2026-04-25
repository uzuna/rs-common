//! Ridge 読み出しモジュール。
//!
//! faer の Cholesky 分解を使って Ridge 回帰を解く。
//!
//! 解法: A = X^T X + λI, b = X^T Y → chol(A)^{-1} b = W_out^T

use faer::linalg::solvers::{Llt, Solve};
use faer::{Mat, Side};

use crate::EsnError;

/// Ridge 読み出し層
pub struct RidgeReadout {
    /// 学習済み重み行列 shape (n_out, units)、`None` は未学習
    w_out: Option<Mat<f64>>,
    ridge: f64,
}

impl RidgeReadout {
    pub fn new(ridge: f64) -> Self {
        Self { w_out: None, ridge }
    }

    /// Ridge 回帰を解いて重みを学習する。
    ///
    /// # 引数
    /// - `states`:   shape `(n_samples, units)` の flat Vec（row-major）
    /// - `n_samples`: サンプル数
    /// - `targets`:  shape `(n_samples, n_out)` の flat Vec（row-major）
    /// - `n_out`:    出力次元数
    /// - `units`:    リザーバユニット数
    pub fn fit(
        &mut self,
        states: &[f64],
        n_samples: usize,
        targets: &[f64],
        n_out: usize,
        units: usize,
    ) -> Result<(), EsnError> {
        assert_eq!(states.len(), n_samples * units);
        assert_eq!(targets.len(), n_samples * n_out);

        // A = X^T X + λI (units × units) — 手動累積（n_samples が大きくても units が小さい）
        let mut a_data = vec![0.0_f64; units * units];
        let mut b_data = vec![0.0_f64; units * n_out];

        for k in 0..n_samples {
            let x_k = &states[k * units..(k + 1) * units];
            let y_k = &targets[k * n_out..(k + 1) * n_out];
            for i in 0..units {
                for j in 0..units {
                    a_data[i * units + j] += x_k[i] * x_k[j];
                }
                for j in 0..n_out {
                    b_data[i * n_out + j] += x_k[i] * y_k[j];
                }
            }
        }

        // Ridge 正則化: 対角に ridge を加算
        for i in 0..units {
            a_data[i * units + i] += self.ridge;
        }

        // faer Mat に変換（row-major → col-major 変換に注意: from_fn は (row, col) 順）
        let a = Mat::<f64>::from_fn(units, units, |i, j| a_data[i * units + j]);
        let b = Mat::<f64>::from_fn(units, n_out, |i, j| b_data[i * n_out + j]);

        // Cholesky 分解: A は SPD (X^T X + λI) が保証されている
        let llt = Llt::<f64>::new(a.as_ref(), Side::Lower)
            .map_err(|e| EsnError::LinearAlgebra(format!("{e:?}")))?;

        // W_out^T = A^{-1} b  shape (units, n_out)
        let w_out_t = llt.solve(&b);

        // W_out shape (n_out, units) = W_out^T ^T
        self.w_out = Some(w_out_t.transpose().to_owned());
        Ok(())
    }

    /// 状態系列に読み出しを適用する。
    ///
    /// # 引数
    /// - `states`: shape `(n, units)` の flat Vec（row-major）
    /// - `n`:      タイムステップ数
    /// - `units`:  リザーバユニット数
    ///
    /// # 戻り値
    /// shape `(n, n_out)` の flat Vec（row-major）
    pub fn run(
        &self,
        states: &[f64],
        n: usize,
        units: usize,
    ) -> Result<Vec<f64>, EsnError> {
        let w_out = self.w_out.as_ref().ok_or(EsnError::NotFitted)?;
        let n_out = w_out.nrows();

        assert_eq!(states.len(), n * units);

        // scores[t, k] = sum_j states[t, j] * w_out[k, j]
        let mut out = vec![0.0_f64; n * n_out];
        for t in 0..n {
            let s_t = &states[t * units..(t + 1) * units];
            for k in 0..n_out {
                let mut acc = 0.0_f64;
                for j in 0..units {
                    acc += s_t[j] * w_out[(k, j)];
                }
                out[t * n_out + k] = acc;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 線形分離可能なダミーデータで 100% 精度が出ることを確認
    #[test]
    fn test_fit_run_separable() {
        let n_samples = 100;
        let units = 4;
        let n_out = 2;

        let mut states = Vec::with_capacity(n_samples * units);
        let mut targets = Vec::with_capacity(n_samples * n_out);
        for i in 0..n_samples {
            let sign = if i < n_samples / 2 { 1.0_f64 } else { -1.0 };
            for _ in 0..units {
                states.push(sign);
            }
            if i < n_samples / 2 {
                targets.extend_from_slice(&[1.0, 0.0]);
            } else {
                targets.extend_from_slice(&[0.0, 1.0]);
            }
        }

        let mut readout = RidgeReadout::new(1e-4);
        readout
            .fit(&states, n_samples, &targets, n_out, units)
            .unwrap();

        let scores = readout.run(&states, n_samples, units).unwrap();
        let mut correct = 0;
        for i in 0..n_samples {
            let s0 = scores[i * n_out];
            let s1 = scores[i * n_out + 1];
            let pred = if s0 > s1 { 0_usize } else { 1 };
            let true_class = if i < n_samples / 2 { 0 } else { 1 };
            if pred == true_class {
                correct += 1;
            }
        }
        assert_eq!(correct, n_samples, "線形分離可能データで精度が 100% でない");
    }

    #[test]
    fn test_run_before_fit_error() {
        let readout = RidgeReadout::new(1e-4);
        let result = readout.run(&[1.0, 2.0], 1, 2);
        assert!(matches!(result, Err(EsnError::NotFitted)));
    }
}
