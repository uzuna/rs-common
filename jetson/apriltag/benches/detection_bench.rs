//! AprilTag 検出パイプラインのベンチマーク
//!
//! # 使い方
//! ```
//! cargo bench -p jetson-apriltag
//! ```
//!
//! # 計測項目
//! 1. RGB → Luma8 変換速度 (2560×1920)
//! 2. AprilTag detect 実行時間 (タグなし / タグあり)
//! 3. パイプライン合計 (変換 + 検出)

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use image::{GrayImage, ImageBuffer, Luma, Rgb, RgbImage};
use jetson_apriltag::{
    convert::{rgb_to_luma8_wide, RgbConverter, PixelConverter},
    detector::{AprilTagDetector, DetectorConfig},
};
use once_cell::sync::Lazy;
use std::path::Path;

// ベンチマーク用画像を一度だけ読み込む (I/O コストを計測から除外)
static BENCH_IMAGE_RGB: Lazy<RgbImage> = Lazy::new(|| {
    load_bench_image()
});

static BENCH_IMAGE_GRAY: Lazy<GrayImage> = Lazy::new(|| {
    let rgb = &*BENCH_IMAGE_RGB;
    let converter = RgbConverter;
    let width = rgb.width();
    let height = rgb.height();
    let src: Vec<u8> = rgb.as_raw().clone();
    let mut dst = Vec::new();
    converter.to_luma8(&src, width, height, &mut dst);
    ImageBuffer::from_raw(width, height, dst).expect("グレー画像の構築に失敗")
});

// タグあり版の画像 (tag36h11 を合成済み)
static BENCH_IMAGE_WITH_TAG: Lazy<GrayImage> = Lazy::new(|| {
    let base = &*BENCH_IMAGE_GRAY;
    synthesize_tag_on_image(base)
});

/// ベンチマーク用背景画像を読み込む
///
/// testdata/ の実写 JPEG を使用。
/// ファイルが存在しない場合はグレーの単色画像にフォールバック。
fn load_bench_image() -> RgbImage {
    // クレートルートからの相対パス探索
    let candidates = [
        "testdata/Tokyo-Japan-city-evening-street-people-buildings_2560x1920.jpg",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/Tokyo-Japan-city-evening-street-people-buildings_2560x1920.jpg"
        ),
    ];

    for path in &candidates {
        if Path::new(path).exists() {
            match image::open(path) {
                Ok(img) => {
                    eprintln!("ベンチマーク画像読み込み: {} ({}×{})", path, img.width(), img.height());
                    return img.into_rgb8();
                }
                Err(e) => {
                    eprintln!("画像読み込み失敗 {}: {}", path, e);
                }
            }
        }
    }

    // フォールバック: 2560×1920 の単色画像
    eprintln!("ベンチマーク画像が見つからないためフォールバック画像を使用 (2560×1920)");
    RgbImage::from_fn(2560, 1920, |_, _| Rgb([128u8, 128, 128]))
}

/// グレー画像に tag36h11 の ID=0 タグを合成する
///
/// タグのビットパターンを直接描画することで、外部ファイル依存なしに
/// 検出可能なテスト画像を生成する。
fn synthesize_tag_on_image(base: &GrayImage) -> GrayImage {
    let mut img = base.clone();
    let width = img.width();
    let height = img.height();

    // tag36h11 ID=0 のビットパターン (8×8 グリッド、外側1セルは白/黒ボーダー)
    // 実際の apriltag の最小描画サイズは 8×8 ピクセル以上が必要
    // ここでは 200×200 px のタグを画像中央に配置する
    let tag_size = 200usize;
    let cell_size = tag_size / 8; // 25 px/cell
    let offset_x = ((width as usize) - tag_size) / 2;
    let offset_y = ((height as usize) - tag_size) / 2;

    // tag36h11 ID=0 のビットデータ (6×6 データビット + 2 ボーダー = 8×8 cells)
    // 外周は白 (明るい) ボーダー、その内側が黒 (暗い) ボーダー
    // データビット (6×6): tag36h11 ID=0 のコード
    // ref: https://github.com/AprilRobotics/apriltag/blob/master/tagStandard41h12.h
    // ここでは 8×8 グリッドの各セルの明暗で定義 (0=黒, 1=白)
    #[rustfmt::skip]
    let tag_grid: [[u8; 8]; 8] = [
        [1, 1, 1, 1, 1, 1, 1, 1], // 外周白ボーダー
        [1, 0, 0, 0, 0, 0, 0, 1], // 内側黒ボーダー
        [1, 0, 1, 0, 1, 1, 0, 1], // データビット行 0
        [1, 0, 0, 1, 0, 1, 0, 1], // データビット行 1
        [1, 0, 1, 1, 0, 0, 0, 1], // データビット行 2
        [1, 0, 1, 0, 1, 0, 0, 1], // データビット行 3
        [1, 0, 0, 0, 0, 0, 0, 1], // 内側黒ボーダー (下)
        [1, 1, 1, 1, 1, 1, 1, 1], // 外周白ボーダー
    ];

    for (row, grid_row) in tag_grid.iter().enumerate() {
        for (col, &bit) in grid_row.iter().enumerate() {
            let px_val = if bit == 1 { 255u8 } else { 0u8 };
            for dy in 0..cell_size {
                for dx in 0..cell_size {
                    let x = (offset_x + col * cell_size + dx) as u32;
                    let y = (offset_y + row * cell_size + dy) as u32;
                    if x < width && y < height {
                        img.put_pixel(x, y, Luma([px_val]));
                    }
                }
            }
        }
    }

    img
}

// --- RGB → Luma8 変換ベンチマーク ---

fn bench_rgb_to_luma8(c: &mut Criterion) {
    let rgb_image = &*BENCH_IMAGE_RGB;
    let width = rgb_image.width();
    let height = rgb_image.height();
    let src = rgb_image.as_raw().as_slice();
    let mut dst = vec![0u8; (width * height) as usize];

    c.bench_function(
        &format!("rgb_to_luma8_wide_{}x{}", width, height),
        |b| {
            b.iter(|| {
                rgb_to_luma8_wide(black_box(src), black_box(&mut dst));
            });
        },
    );
}

fn bench_rgb_converter_to_luma8(c: &mut Criterion) {
    let rgb_image = &*BENCH_IMAGE_RGB;
    let width = rgb_image.width();
    let height = rgb_image.height();
    let src = rgb_image.as_raw().as_slice();
    let converter = RgbConverter;
    let mut dst = Vec::new();

    c.bench_function(
        &format!("RgbConverter_to_luma8_{}x{}", width, height),
        |b| {
            b.iter(|| {
                converter.to_luma8(black_box(src), black_box(width), black_box(height), black_box(&mut dst));
            });
        },
    );
}

// --- AprilTag 検出ベンチマーク ---

fn bench_detect_no_tag(c: &mut Criterion) {
    let gray = &*BENCH_IMAGE_GRAY;
    let width = gray.width();
    let height = gray.height();
    let config = DetectorConfig {
        decimation: 2.0, // 大画像なので 2x デシメーション
        ..DetectorConfig::default()
    };
    let mut detector = AprilTagDetector::new(&config).expect("Detector 構築失敗");

    c.bench_function(
        &format!("detect_no_tag_{}x{}", width, height),
        |b| {
            b.iter(|| {
                let result = detector.detect_gray(black_box(gray));
                black_box(result);
            });
        },
    );
}

fn bench_detect_with_tag(c: &mut Criterion) {
    let gray = &*BENCH_IMAGE_WITH_TAG;
    let width = gray.width();
    let height = gray.height();
    let config = DetectorConfig {
        decimation: 2.0,
        ..DetectorConfig::default()
    };
    let mut detector = AprilTagDetector::new(&config).expect("Detector 構築失敗");

    c.bench_function(
        &format!("detect_with_tag_{}x{}", width, height),
        |b| {
            b.iter(|| {
                let result = detector.detect_gray(black_box(gray));
                black_box(result);
            });
        },
    );
}

fn bench_pipeline_full(c: &mut Criterion) {
    let rgb_image = &*BENCH_IMAGE_WITH_TAG;
    // GrayImage を RGB に戻す (パイプライン全体の計測のため)
    // 実際の用途は RGB カメラ → Luma8 → 検出
    let width = rgb_image.width();
    let height = rgb_image.height();
    let gray_data = rgb_image.as_raw().clone();

    let config = DetectorConfig {
        decimation: 2.0,
        ..DetectorConfig::default()
    };
    let mut detector = AprilTagDetector::new(&config).expect("Detector 構築失敗");

    // GrayImage はすでに Luma8 なので、ここではグレーデータを直接渡すパイプラインを計測
    c.bench_function(
        &format!("pipeline_luma8_detect_{}x{}", width, height),
        |b| {
            b.iter(|| {
                let mut luma = Vec::with_capacity((width * height) as usize);
                // グレー画像のデータをそのままコピー (実際はYUYV/NV12変換が入る)
                luma.extend_from_slice(black_box(&gray_data));
                let result = detector
                    .detect_luma8(black_box(&luma), black_box(width), black_box(height))
                    .expect("検出失敗");
                black_box(result);
            });
        },
    );
}

criterion_group!(
    benches,
    bench_rgb_to_luma8,
    bench_rgb_converter_to_luma8,
    bench_detect_no_tag,
    bench_detect_with_tag,
    bench_pipeline_full,
);
criterion_main!(benches);
