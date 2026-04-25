//! ESN リザーバモジュール。
//!
//! 固定ランダム重みのリザーバ（スペクトル半径をべき乗法でスケーリング）。
//! 更新式: x(t+1) = (1-lr)*x(t) + lr*tanh(W_res @ x(t) + W_in @ u(t))

use rand::distr::{Distribution, Uniform};
use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::EsnConfig;

/// Echo State Network リザーバ（固定重み）
pub struct Reservoir {
    /// リザーバ重み行列 row-major, shape (units, units)
    w_res: Vec<f64>,
    /// 入力重み行列 row-major, shape (units, input_dim)
    w_in: Vec<f64>,
    /// 現在の状態ベクトル shape (units,)
    state: Vec<f64>,
    units: usize,
    input_dim: usize,
    lr: f64,
}

/// Box-Muller 変換で N(0, std) サンプルを生成する。
fn sample_gaussian(std: f64, rng: &mut impl rand::RngExt) -> f64 {
    let u1: f64 = (rng.random::<f64>()).max(f64::MIN_POSITIVE);
    let u2: f64 = rng.random::<f64>();
    let r: f64 = (-2.0_f64 * u1.ln()).sqrt();
    let theta = 2.0 * std::f64::consts::PI * u2;
    r * theta.cos() * std
}

impl Reservoir {
    /// 設定から新規リザーバを構築する。
    ///
    /// 初期化:
    /// 1. W_in ~ N(0, 0.1)
    /// 2. W_res ~ N(0, 1)
    /// 3. べき乗法でスペクトル半径を推定し `config.sr` にスケーリング
    pub fn new(config: &EsnConfig) -> Self {
        let mut rng = SmallRng::seed_from_u64(config.seed);
        let units = config.units;
        let input_dim = config.input_dim;

        // W_in: shape (units, input_dim), Uniform(-1, 1) — reservoirpy デフォルト (input_scaling=1.0)
        let w_in_dist = Uniform::new(-1.0_f64, 1.0_f64).unwrap();
        let w_in: Vec<f64> = (0..units * input_dim)
            .map(|_| w_in_dist.sample(&mut rng))
            .collect();

        // W_res: shape (units, units) — sparse 10% connectivity (reservoirpy デフォルト)
        let sparse_dist = Uniform::new(0.0_f64, 1.0_f64).unwrap();
        let mut w_res: Vec<f64> = (0..units * units)
            .map(|_| {
                if sparse_dist.sample(&mut rng) < 0.1 {
                    sample_gaussian(1.0, &mut rng)
                } else {
                    0.0
                }
            })
            .collect();

        // スペクトル半径をべき乗法で推定してスケーリング
        let sr_est = power_iteration_sr(&w_res, units, 100, &mut rng);
        if sr_est > 1e-9 {
            let scale = config.sr / sr_est;
            for v in &mut w_res {
                *v *= scale;
            }
        }

        Self {
            w_res,
            w_in,
            state: vec![0.0; units],
            units,
            input_dim,
            lr: config.lr,
        }
    }

    /// 状態をゼロにリセットする。
    pub fn reset(&mut self) {
        self.state.fill(0.0);
    }

    /// 入力系列を逐次処理して状態系列を返す。
    ///
    /// # 引数
    /// - `u`: shape `(n, input_dim)` の flat Vec（row-major）
    /// - `n`: タイムステップ数
    ///
    /// # 戻り値
    /// shape `(n, units)` の flat Vec（row-major）
    pub fn run(&mut self, u: &[f64], n: usize) -> Vec<f64> {
        assert_eq!(u.len(), n * self.input_dim, "入力サイズ不一致");
        let units = self.units;
        let input_dim = self.input_dim;
        let lr = self.lr;

        let mut states = vec![0.0_f64; n * units];

        for t in 0..n {
            let u_t = &u[t * input_dim..(t + 1) * input_dim];

            // pre_act = W_res @ x + W_in @ u[t]
            let mut pre_act = vec![0.0_f64; units];
            for i in 0..units {
                let res: f64 = (0..units)
                    .map(|j| self.w_res[i * units + j] * self.state[j])
                    .sum();
                let inp: f64 = (0..input_dim)
                    .map(|k| self.w_in[i * input_dim + k] * u_t[k])
                    .sum();
                pre_act[i] = res + inp;
            }

            // x(t+1) = (1 - lr) * x + lr * tanh(pre_act)
            for i in 0..units {
                self.state[i] = (1.0 - lr) * self.state[i] + lr * pre_act[i].tanh();
            }
            states[t * units..(t + 1) * units].copy_from_slice(&self.state);
        }
        states
    }

    /// リザーバのユニット数を返す
    pub fn units(&self) -> usize {
        self.units
    }
}

/// べき乗法（power iteration）によるスペクトル半径推定。
fn power_iteration_sr(w: &[f64], units: usize, n_iter: usize, rng: &mut impl rand::RngExt) -> f64 {
    let dist = Uniform::new(-1.0_f64, 1.0_f64).unwrap();

    // ランダム初期ベクトル
    let mut v: Vec<f64> = (0..units).map(|_| dist.sample(rng)).collect();
    let norm = l2_norm(&v);
    for vi in &mut v {
        *vi /= norm;
    }

    for _ in 0..n_iter {
        let wv = mat_vec_mul(w, &v, units);
        let wv_norm = l2_norm(&wv);
        if wv_norm < 1e-15 {
            return 0.0;
        }
        v = wv.iter().map(|x| x / wv_norm).collect();
    }

    l2_norm(&mat_vec_mul(w, &v, units))
}

/// 行列ベクトル積 W @ v, W は (units, units) row-major
fn mat_vec_mul(w: &[f64], v: &[f64], units: usize) -> Vec<f64> {
    (0..units)
        .map(|i| (0..units).map(|j| w[i * units + j] * v[j]).sum())
        .collect()
}

/// L2 ノルム
fn l2_norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> EsnConfig {
        EsnConfig::default()
    }

    #[test]
    fn test_reset_zeroes_state() {
        let mut res = Reservoir::new(&default_config());
        let u: Vec<f64> = vec![0.5_f64; 200]; // 100 steps × 2 ch
        let _ = res.run(&u, 100);
        // 状態が非ゼロのはず
        let nonzero = res.state.iter().any(|&v| v.abs() > 1e-9);
        assert!(nonzero, "run 後に状態がすべてゼロ");

        res.reset();
        assert!(
            res.state.iter().all(|&v| v == 0.0),
            "reset 後に状態がゼロでない"
        );
    }

    #[test]
    fn test_state_bounded() {
        let mut res = Reservoir::new(&default_config());
        let n = 500;
        let u: Vec<f64> = (0..n * 2)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let states = res.run(&u, n);
        // tanh 出力は (-1, 1) → リーク率で混合しても有界
        for &s in &states {
            assert!(s.abs() < 1.0 + 1e-9, "状態値が有界でない: {s}");
        }
    }

    #[test]
    fn test_run_output_shape() {
        let mut res = Reservoir::new(&default_config());
        let n = 50;
        let input_dim = res.input_dim;
        let u = vec![0.1_f64; n * input_dim];
        let states = res.run(&u, n);
        assert_eq!(states.len(), n * res.units());
    }
}
