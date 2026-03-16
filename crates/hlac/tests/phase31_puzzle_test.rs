use std::path::PathBuf;

use hlac::{u8_to_binary_array, GrayHlacFeature, HlacExtractor, HlacFeature};
use image::{Rgba, RgbaImage};

const BINARY_THRESHOLD: u8 = 127;

#[derive(Debug, Clone)]
struct TestImage {
    width: usize,
    height: usize,
    luma: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedRelation {
    Identical,
    Different,
}

#[derive(Debug, Clone, Copy)]
struct RectPatch {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

#[derive(Debug, Clone, Copy)]
struct ScoredRectPatch {
    rect: RectPatch,
    // 0.0..=1.0 に正規化した差分強度
    score: f32,
}

#[inline]
fn assert_case(label: &str, condition: bool, message: &str) {
    assert!(condition, "[{label}] {message}");
}

fn load_test_image(file_name: &str) -> TestImage {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(file_name);
    let image = image::open(&path)
        .unwrap_or_else(|err| panic!("testdata 読み込み失敗: path={}, err={err}", path.display()));
    let gray = image.to_luma8();

    TestImage {
        width: gray.width() as usize,
        height: gray.height() as usize,
        luma: gray.into_raw(),
    }
}

fn assert_same_shape(label: &str, lhs: &TestImage, rhs: &TestImage) {
    assert_case(
        label,
        lhs.width == rhs.width && lhs.height == rhs.height,
        "画像サイズが一致しない",
    );
}

fn pixel_diff_count(lhs: &TestImage, rhs: &TestImage) -> usize {
    lhs.luma
        .iter()
        .zip(rhs.luma.iter())
        .filter(|(l, r)| l != r)
        .count()
}

fn extract_binary_feature(extractor: &HlacExtractor, image: &TestImage) -> HlacFeature {
    let binary = u8_to_binary_array(&image.luma, image.width, image.height, BINARY_THRESHOLD)
        .expect("2値化に失敗");
    extractor
        .extract_binary_bool(&binary, image.width, image.height)
        .expect("2値HLAC抽出に失敗")
}

fn extract_gray_feature(extractor: &HlacExtractor, image: &TestImage) -> GrayHlacFeature {
    extractor
        .extract_gray_u8(&image.luma, image.width, image.height)
        .expect("グレースケールHLAC抽出に失敗")
}

fn binary_feature_l1(lhs: &HlacFeature, rhs: &HlacFeature) -> u64 {
    lhs.counts
        .iter()
        .zip(rhs.counts.iter())
        .map(|(l, r)| l.abs_diff(*r))
        .sum()
}

fn gray_feature_l1(lhs: &GrayHlacFeature, rhs: &GrayHlacFeature) -> f64 {
    lhs.sums
        .iter()
        .zip(rhs.sums.iter())
        .map(|(l, r)| (l - r).abs())
        .sum()
}

fn detect_diff_tile_patches(
    lhs: &TestImage,
    rhs: &TestImage,
    tile_size: usize,
    diff_threshold: u8,
) -> Vec<RectPatch> {
    assert!(tile_size > 0, "tile_size must be > 0");

    let tiles_x = lhs.width.div_ceil(tile_size);
    let tiles_y = lhs.height.div_ceil(tile_size);
    let mut patches = Vec::new();

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let x0 = tx * tile_size;
            let y0 = ty * tile_size;
            let x1 = (x0 + tile_size).min(lhs.width);
            let y1 = (y0 + tile_size).min(lhs.height);

            let mut changed = false;
            'scan: for y in y0..y1 {
                let row_start = y * lhs.width;
                for x in x0..x1 {
                    let idx = row_start + x;
                    if lhs.luma[idx].abs_diff(rhs.luma[idx]) >= diff_threshold {
                        changed = true;
                        break 'scan;
                    }
                }
            }

            if changed {
                patches.push(RectPatch { x0, y0, x1, y1 });
            }
        }
    }

    patches
}

fn apply_rect_patches(base: &TestImage, target: &TestImage, patches: &[RectPatch]) -> TestImage {
    let mut patched = base.clone();

    for patch in patches {
        for y in patch.y0..patch.y1 {
            let start = y * base.width + patch.x0;
            let end = y * base.width + patch.x1;
            patched.luma[start..end].copy_from_slice(&target.luma[start..end]);
        }
    }

    patched
}

fn score_diff_patch(lhs: &TestImage, rhs: &TestImage, patch: RectPatch) -> ScoredRectPatch {
    let mut diff_sum = 0_u64;
    let mut pixels = 0_u64;

    for y in patch.y0..patch.y1 {
        let row_start = y * lhs.width;
        for x in patch.x0..patch.x1 {
            let idx = row_start + x;
            diff_sum += u64::from(lhs.luma[idx].abs_diff(rhs.luma[idx]));
            pixels += 1;
        }
    }

    let score = if pixels == 0 {
        0.0
    } else {
        (diff_sum as f32 / pixels as f32) / 255.0
    }
    .clamp(0.0, 1.0);

    ScoredRectPatch { rect: patch, score }
}

fn score_diff_patches(
    lhs: &TestImage,
    rhs: &TestImage,
    patches: &[RectPatch],
) -> Vec<ScoredRectPatch> {
    patches
        .iter()
        .copied()
        .map(|patch| score_diff_patch(lhs, rhs, patch))
        .collect()
}

fn normalize_heat_score(score: f32, min_score: f32) -> f32 {
    let threshold = min_score.clamp(0.0, 1.0);
    if score <= threshold {
        return 0.0;
    }

    let denom = (1.0 - threshold).max(f32::EPSILON);
    ((score - threshold) / denom).clamp(0.0, 1.0)
}

fn heat_color(score: f32, min_score: f32) -> [u8; 4] {
    let t = normalize_heat_score(score, min_score);
    [255, 0, 0, (255.0 * t) as u8]
}

fn blend_over_opaque(dst: &mut [u8; 4], src: [u8; 4]) {
    let alpha = u16::from(src[3]);
    if alpha == 0 {
        return;
    }

    let inv = 255_u16 - alpha;
    for i in 0..3 {
        let blended = (u16::from(src[i]) * alpha + u16::from(dst[i]) * inv + 127) / 255;
        dst[i] = blended as u8;
    }
    dst[3] = 255;
}

fn build_diff_heatmap(
    width: usize,
    height: usize,
    patches: &[ScoredRectPatch],
    min_score: f32,
) -> RgbaImage {
    let mut heatmap = RgbaImage::new(width as u32, height as u32);

    for patch in patches {
        let color = heat_color(patch.score, min_score);
        if color[3] == 0 {
            continue;
        }
        for y in patch.rect.y0..patch.rect.y1 {
            for x in patch.rect.x0..patch.rect.x1 {
                heatmap.put_pixel(x as u32, y as u32, Rgba(color));
            }
        }
    }

    heatmap
}

fn overlay_heatmap_on_image(base: &TestImage, heatmap: &RgbaImage) -> RgbaImage {
    assert_eq!(heatmap.width() as usize, base.width);
    assert_eq!(heatmap.height() as usize, base.height);

    let mut overlay = RgbaImage::new(base.width as u32, base.height as u32);

    for y in 0..base.height {
        for x in 0..base.width {
            let idx = y * base.width + x;
            let gray = base.luma[idx];
            let mut out = [gray, gray, gray, 255];
            let src = heatmap.get_pixel(x as u32, y as u32).0;
            blend_over_opaque(&mut out, src);
            overlay.put_pixel(x as u32, y as u32, Rgba(out));
        }
    }

    overlay
}

fn phase31_output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("phase31_output")
}

/// 差分可視化画像の出力を有効化するかどうかを返す。
///
/// `ENABLE_OUTPUT_IMAGE=1` のときのみ保存処理を実行し、
/// それ以外（未設定含む）は保存しない。
fn output_image_enabled() -> bool {
    std::env::var("ENABLE_OUTPUT_IMAGE")
        .map(|value| value == "1")
        .unwrap_or(false)
}

fn count_heatmap_nonzero_alpha_pixels(image: &RgbaImage) -> usize {
    image.pixels().filter(|p| p.0[3] > 0).count()
}

fn count_overlay_colored_pixels(base: &TestImage, overlay: &RgbaImage) -> usize {
    let mut count = 0_usize;
    for y in 0..base.height {
        for x in 0..base.width {
            let idx = y * base.width + x;
            let gray = base.luma[idx];
            let px = overlay.get_pixel(x as u32, y as u32).0;
            if px[0] != gray || px[1] != gray || px[2] != gray {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn phase31_puzzle_feature_comparison_cases() {
    struct Case {
        label: &'static str,
        left: &'static str,
        right: &'static str,
        relation: ExpectedRelation,
    }

    let cases = [
        Case {
            label: "identical_reference",
            left: "img_puzzle_1.webp",
            right: "img_puzzle_1.webp",
            relation: ExpectedRelation::Identical,
        },
        Case {
            label: "spot_the_difference_pair",
            left: "img_puzzle_1.webp",
            right: "img_puzzle_2.webp",
            relation: ExpectedRelation::Different,
        },
    ];

    let extractor = HlacExtractor::new_binary_25();

    for case in &cases {
        let left = load_test_image(case.left);
        let right = load_test_image(case.right);
        assert_same_shape(case.label, &left, &right);

        let pixel_diff = pixel_diff_count(&left, &right);
        let left_binary = extract_binary_feature(&extractor, &left);
        let right_binary = extract_binary_feature(&extractor, &right);
        let left_gray = extract_gray_feature(&extractor, &left);
        let right_gray = extract_gray_feature(&extractor, &right);

        let binary_l1 = binary_feature_l1(&left_binary, &right_binary);
        let gray_l1 = gray_feature_l1(&left_gray, &right_gray);

        match case.relation {
            ExpectedRelation::Identical => {
                assert_case(case.label, pixel_diff == 0, "同一画像なのに画素差分がある");
                assert_case(
                    case.label,
                    binary_l1 == 0,
                    "同一画像なのに2値特徴量が不一致",
                );
                assert_case(
                    case.label,
                    gray_l1 == 0.0,
                    "同一画像なのにグレースケール特徴量が不一致",
                );
            }
            ExpectedRelation::Different => {
                assert_case(case.label, pixel_diff > 0, "異なる画像なのに画素差分がない");
                assert_case(
                    case.label,
                    gray_l1 > 0.0,
                    "異なる画像なのにグレースケール特徴量差分がない",
                );
                assert_case(
                    case.label,
                    binary_l1 > 0 || gray_l1 > 0.0,
                    "異なる画像なのに特徴量差分がない",
                );
            }
        }
    }
}

#[test]
fn phase31_diff_patch_overlay_cases() {
    struct Case {
        label: &'static str,
        tile_size: usize,
        diff_threshold: u8,
    }

    let cases = [
        Case {
            label: "tile_8",
            tile_size: 8,
            diff_threshold: 1,
        },
        Case {
            label: "tile_32",
            tile_size: 32,
            diff_threshold: 1,
        },
    ];

    let extractor = HlacExtractor::new_binary_25();
    let base = load_test_image("img_puzzle_1.webp");
    let target = load_test_image("img_puzzle_2.webp");
    assert_same_shape("phase31_diff_patch_overlay_cases", &base, &target);

    let base_gray = extract_gray_feature(&extractor, &base);
    let target_gray = extract_gray_feature(&extractor, &target);
    let base_binary = extract_binary_feature(&extractor, &base);
    let target_binary = extract_binary_feature(&extractor, &target);

    for case in &cases {
        let patches = detect_diff_tile_patches(&base, &target, case.tile_size, case.diff_threshold);
        assert_case(
            case.label,
            !patches.is_empty(),
            "差分パッチが1つも検出されない",
        );

        let patched = apply_rect_patches(&base, &target, &patches);
        assert_eq!(
            patched.luma, target.luma,
            "[{}] 差分パッチ適用後の画像が目標画像と一致しない",
            case.label
        );

        let patched_gray = extract_gray_feature(&extractor, &patched);
        let patched_binary = extract_binary_feature(&extractor, &patched);

        let before_gray_l1 = gray_feature_l1(&base_gray, &target_gray);
        let after_gray_l1 = gray_feature_l1(&patched_gray, &target_gray);

        assert_case(
            case.label,
            before_gray_l1 > 0.0,
            "差分パッチ適用前のグレースケール特徴量差分が0",
        );
        assert_case(
            case.label,
            after_gray_l1 == 0.0,
            "差分パッチ適用後もグレースケール特徴量差分が残っている",
        );

        assert_case(
            case.label,
            binary_feature_l1(&base_binary, &target_binary) > 0
                || gray_feature_l1(&base_gray, &target_gray) > 0.0,
            "適用前に特徴量差分がない",
        );
        assert_eq!(
            patched_binary, target_binary,
            "[{}] 差分パッチ適用後の2値特徴量が一致しない",
            case.label
        );
    }
}

#[test]
fn phase31_diff_heatmap_overlay_output_cases() {
    struct Case {
        label: &'static str,
        tile_size: usize,
        diff_threshold: u8,
        heatmap_min_score: f32,
    }

    let cases = [
        Case {
            label: "tile_8",
            tile_size: 8,
            diff_threshold: 1,
            heatmap_min_score: 0.2,
        },
        Case {
            label: "tile_32",
            tile_size: 32,
            diff_threshold: 1,
            heatmap_min_score: 0.03,
        },
    ];

    let base = load_test_image("img_puzzle_1.webp");
    let target = load_test_image("img_puzzle_2.webp");
    assert_same_shape("phase31_diff_heatmap_overlay_output_cases", &base, &target);

    // `ENABLE_OUTPUT_IMAGE=1` のときのみ testdata/phase31_output に画像を保存する。
    let output_enabled = output_image_enabled();
    let out_dir = phase31_output_dir();
    if output_enabled {
        std::fs::create_dir_all(&out_dir).expect("可視化出力ディレクトリの作成に失敗");
    }

    for case in &cases {
        let patches = detect_diff_tile_patches(&base, &target, case.tile_size, case.diff_threshold);
        assert_case(
            case.label,
            !patches.is_empty(),
            "差分パッチが1つも検出されない",
        );

        // 既存の差分パッチと同じ領域に対して差分強度を計算する。
        let scored = score_diff_patches(&base, &target, &patches);
        assert_case(
            case.label,
            scored.iter().any(|p| p.score > 0.0),
            "差分強度がすべて0になっている",
        );

        let heatmap_min_score = case.heatmap_min_score;
        assert_case(
            case.label,
            scored.iter().any(|p| p.score > heatmap_min_score),
            "ヒートマップ閾値が高すぎて可視化対象がなくなっている",
        );

        let heatmap = build_diff_heatmap(base.width, base.height, &scored, heatmap_min_score);
        let overlay = overlay_heatmap_on_image(&base, &heatmap);

        if output_enabled {
            let heatmap_file = out_dir.join(format!("{}_heatmap.png", case.label));
            let overlay_file = out_dir.join(format!("{}_overlay.png", case.label));

            heatmap.save(&heatmap_file).unwrap_or_else(|err| {
                panic!(
                    "ヒートマップ保存に失敗: path={}, err={err}",
                    heatmap_file.display()
                )
            });
            overlay.save(&overlay_file).unwrap_or_else(|err| {
                panic!(
                    "重ね合わせ画像保存に失敗: path={}, err={err}",
                    overlay_file.display()
                )
            });

            assert_case(
                case.label,
                heatmap_file.exists(),
                "ヒートマップ画像が出力されていない",
            );
            assert_case(
                case.label,
                overlay_file.exists(),
                "重ね合わせ画像が出力されていない",
            );
        }
        assert_case(
            case.label,
            count_heatmap_nonzero_alpha_pixels(&heatmap) > 0,
            "ヒートマップに有効画素がない",
        );
        assert_case(
            case.label,
            count_overlay_colored_pixels(&base, &overlay) > 0,
            "重ね合わせ画像に色付き画素がない",
        );
    }
}
