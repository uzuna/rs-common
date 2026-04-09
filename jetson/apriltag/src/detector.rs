//! AprilTag 検出コアモジュール
//!
//! # 注意
//! `apriltag::Detector` は `!Send + !Sync` のため、同一スレッド内でのみ使用すること。
//! マルチスレッド化が必要な場合は、スレッドごとに `AprilTagDetector` インスタンスを作成する。

use apriltag::{Detector, DetectorBuilder, Family, Image};
use image::{GrayImage, ImageBuffer, Luma};
use thiserror::Error;

/// 検出エラー型
#[derive(Error, Debug)]
pub enum DetectorError {
    #[error("Detector のビルドに失敗: {0}")]
    BuildFailed(String),
    #[error("画像バッファの生成に失敗 (width={width}, height={height}, data_len={data_len})")]
    InvalidImageBuffer {
        width: u32,
        height: u32,
        data_len: usize,
    },
}

/// Detector の設定パラメータ
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    /// デシメーション比 (1.0=フルスケール, 2.0=解像度半分で検出)
    pub decimation: f32,
    /// ガウスブラーのシグマ (0.0=なし)
    pub sigma: f32,
    /// エッジ精度向上フラグ
    pub refine_edges: bool,
    /// 検出スレッド数
    pub threads: u8,
    /// タグファミリー
    pub family: TagFamily,
    /// 許容ハミング距離 (0=完全一致のみ, 1=1ビット誤り許容)
    pub hamming: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            decimation: 1.0,
            sigma: 0.0,
            refine_edges: true,
            threads: 1,
            family: TagFamily::Tag36h11,
            hamming: 1,
        }
    }
}

/// サポートするタグファミリー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagFamily {
    Tag36h11,
    Tag25h9,
    Tag16h5,
    TagCircle21h7,
    TagCircle49h12,
    TagCustom48h12,
    TagStandard41h12,
    TagStandard52h13,
}

/// AprilTag 検出結果
#[derive(Debug, Clone)]
pub struct TagDetection {
    /// タグ ID
    pub id: usize,
    /// ビット誤り数 (0=完全一致)
    pub hamming: usize,
    /// 品質指標 (高いほど良い検出)
    pub decision_margin: f32,
    /// タグ中心の画像座標 [x, y]
    pub center: [f64; 2],
    /// 4隅の画像座標 [[x,y]; 4] (左上から時計回り)
    pub corners: [[f64; 2]; 4],
}

/// AprilTag 検出器ラッパー
///
/// `apriltag::Detector` は `!Send + !Sync` のため、このラッパーも同様。
pub struct AprilTagDetector {
    inner: Detector,
}

// Detector が !Send なので AprilTagDetector も !Send を引き継ぐ (自動的に !Send になる)

impl AprilTagDetector {
    /// 設定から検出器を構築する
    pub fn new(config: &DetectorConfig) -> Result<Self, DetectorError> {
        let family = match config.family {
            TagFamily::Tag36h11 => Family::tag_36h11(),
            TagFamily::Tag25h9 => Family::tag_25h9(),
            TagFamily::Tag16h5 => Family::tag_16h5(),
            TagFamily::TagCircle21h7 => Family::tag_circle_21h7(),
            TagFamily::TagCircle49h12 => Family::tag_circle_49h12(),
            TagFamily::TagCustom48h12 => Family::tag_custom_48h12(),
            TagFamily::TagStandard41h12 => Family::tag_standard_41h12(),
            TagFamily::TagStandard52h13 => Family::tag_standard_52h13(),
        };

        let mut detector = DetectorBuilder::new()
            .add_family_bits(family, config.hamming)
            .build()
            .map_err(|e| DetectorError::BuildFailed(e.to_string()))?;

        detector.set_thread_number(config.threads);
        detector.set_decimation(config.decimation);
        detector.set_sigma(config.sigma);
        detector.set_refine_edges(config.refine_edges);

        Ok(Self { inner: detector })
    }

    /// Luma8 バイト列から AprilTag を検出する
    ///
    /// * `data`   - グレースケール画像データ (width × height バイト)
    /// * `width`  - 画像幅 [px]
    /// * `height` - 画像高 [px]
    pub fn detect_luma8(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<TagDetection>, DetectorError> {
        let n = (width * height) as usize;
        if data.len() < n {
            return Err(DetectorError::InvalidImageBuffer {
                width,
                height,
                data_len: data.len(),
            });
        }

        // apriltag::Image を zeros_with_stride で確保してデータをコピー
        // apriltag-image クレートを経由しないため image クレートのバージョン衝突を回避する
        let at_image = luma8_to_apriltag(data, width, height);
        let detections = self.inner.detect(&at_image);

        Ok(map_detections(detections))
    }

    /// `image::ImageBuffer<Luma<u8>>` から直接検出する
    pub fn detect_image(&mut self, img: &ImageBuffer<Luma<u8>, Vec<u8>>) -> Vec<TagDetection> {
        let at_image = luma8_to_apriltag(img.as_raw(), img.width(), img.height());
        let detections = self.inner.detect(&at_image);
        map_detections(detections)
    }

    /// `image::GrayImage` の参照から直接検出する
    pub fn detect_gray(&mut self, img: &GrayImage) -> Vec<TagDetection> {
        let at_image = luma8_to_apriltag(img.as_raw(), img.width(), img.height());
        let detections = self.inner.detect(&at_image);
        map_detections(detections)
    }
}

/// Luma8 スライスを `apriltag::Image` に変換する
///
/// `apriltag-image` クレートを使わず直接 `Image::zeros_with_stride` → コピーで変換する。
/// これにより `image` クレートのバージョン衝突を回避できる。
fn luma8_to_apriltag(data: &[u8], width: u32, height: u32) -> Image {
    // stride = width (パディングなし)
    let mut img = Image::zeros_with_stride(width as usize, height as usize, width as usize)
        .expect("apriltag::Image の確保に失敗");
    let n = (width * height) as usize;
    img.as_slice_mut()[..n].copy_from_slice(&data[..n]);
    img
}

/// `Vec<apriltag::Detection>` を `Vec<TagDetection>` に変換する
fn map_detections(detections: Vec<apriltag::Detection>) -> Vec<TagDetection> {
    detections
        .into_iter()
        .map(|det| TagDetection {
            id: det.id(),
            hamming: det.hamming(),
            decision_margin: det.decision_margin(),
            center: det.center(),
            corners: det.corners(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- 正常系: 検出器の構築 ---

    #[test]
    fn test_detector_build_default() {
        let config = DetectorConfig::default();
        let detector = AprilTagDetector::new(&config);
        assert!(detector.is_ok(), "デフォルト設定での Detector 構築に失敗");
    }

    #[test]
    fn test_detector_build_all_families() {
        // 全タグファミリーでビルドできることを確認
        let families = [
            TagFamily::Tag36h11,
            TagFamily::Tag25h9,
            TagFamily::Tag16h5,
            TagFamily::TagCircle21h7,
            TagFamily::TagCircle49h12,
            TagFamily::TagCustom48h12,
            TagFamily::TagStandard41h12,
            TagFamily::TagStandard52h13,
        ];
        for family in families {
            let config = DetectorConfig {
                family,
                ..DetectorConfig::default()
            };
            assert!(
                AprilTagDetector::new(&config).is_ok(),
                "{family:?} ファミリーの Detector 構築に失敗"
            );
        }
    }

    // --- 正常系: 空画像で検出ゼロを返す ---

    #[test]
    fn test_detect_empty_image_no_detections() {
        let mut detector = AprilTagDetector::new(&DetectorConfig::default()).unwrap();
        // 何もない 64×64 グレー画像 → 検出なし
        let data = vec![128u8; 64 * 64];
        let detections = detector.detect_luma8(&data, 64, 64).unwrap();
        assert_eq!(detections.len(), 0, "空画像でタグが誤検出された");
    }

    // --- 異常系: 不正なバッファサイズ ---

    #[test]
    fn test_detect_invalid_buffer_size() {
        let mut detector = AprilTagDetector::new(&DetectorConfig::default()).unwrap();
        // width*height に満たないバッファ
        let data = vec![0u8; 10]; // 64×64 には 4096 バイト必要
        let result = detector.detect_luma8(&data, 64, 64);
        assert!(result.is_err(), "不正なバッファサイズでエラーにならなかった");
    }
}
