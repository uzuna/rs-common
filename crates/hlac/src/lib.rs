//! HLAC (Higher-order Local Auto-Correlation) を扱うクレート。
//!
//! Phase1では2値HLAC（3x3, 25パターン）を提供する。

pub mod binary;
pub mod mask;

pub use binary::{HlacError, HlacExtractor, HlacFeature};
pub use mask::{HLAC25_OFFSETS, HLAC_DIM};
