//! ESN (Echo State Network) リザーバコンピューティングによる時系列分類クレート。
//!
//! Phase 10: 6 モード 2ch 波形分類 + SpectralPeak による未知モード検出。

pub mod data;
pub mod detector;
pub mod readout;
pub mod reservoir;

/// サンプリング周波数 [Hz]
pub const FS: f64 = 30.0;
/// 信号周波数 [Hz]
pub const FREQ: f64 = 2.0;
/// 分類モード数
pub const N_MODES: usize = 6;
/// ウォームアップステップ数
pub const WARMUP: usize = 100;

/// ESN ハイパーパラメータ設定
#[derive(Debug, Clone)]
pub struct EsnConfig {
    /// リザーバユニット数
    pub units: usize,
    /// 目標スペクトル半径
    pub sr: f64,
    /// リーク率
    pub lr: f64,
    /// Ridge 正則化係数
    pub ridge: f64,
    /// 入力次元数
    pub input_dim: usize,
    /// 分類クラス数
    pub n_modes: usize,
    /// 乱数シード
    pub seed: u64,
}

impl Default for EsnConfig {
    fn default() -> Self {
        Self {
            units: 200,
            sr: 0.9,
            lr: 0.3,
            ridge: 1e-4,
            input_dim: 2,
            n_modes: N_MODES,
            seed: 42,
        }
    }
}

/// ESN 処理エラー
#[derive(Debug, thiserror::Error)]
pub enum EsnError {
    #[error("Readout がまだ学習されていません")]
    NotFitted,
    #[error("行列演算エラー: {0}")]
    LinearAlgebra(String),
    #[error("入力サイズエラー: {0}")]
    InvalidSize(String),
}
