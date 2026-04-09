//! AprilTag 検出クレート
//!
//! # 概要
//! - `convert`: ピクセルフォーマット変換 (YUYV/NV12/RGB → Luma8)
//! - `detector`: AprilTag 検出コア
//!
//! # SIMD 戦略
//! `wide` クレートで x86_64 SSE/AVX と aarch64 NEON を統一的に扱う。
//! `RUSTFLAGS="-C target-cpu=native"` を指定することで自動的に最適な命令セットが選択される。

pub mod convert;
pub mod detector;

pub use detector::{AprilTagDetector, DetectorConfig, TagDetection};
