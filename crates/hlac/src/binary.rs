use std::fmt;

use crate::mask::{HLAC25_OFFSETS, HLAC_DIM};
use wide::u8x16;

const SIMD_LANES: usize = 16;

/// 2値HLACの25次元特徴量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlacFeature {
    pub counts: [u64; HLAC_DIM],
}

impl HlacFeature {
    pub const fn new(counts: [u64; HLAC_DIM]) -> Self {
        Self { counts }
    }

    pub const fn zeros() -> Self {
        Self {
            counts: [0; HLAC_DIM],
        }
    }

    /// 任意の分母で正規化したf64配列を返す。
    ///
    /// 分母が0以下の場合はゼロ配列を返す。
    pub fn as_f64_normalized(&self, denom: f64) -> [f64; HLAC_DIM] {
        if denom <= 0.0 {
            return [0.0; HLAC_DIM];
        }

        let mut normalized = [0.0; HLAC_DIM];
        for (idx, value) in self.counts.iter().enumerate() {
            normalized[idx] = *value as f64 / denom;
        }
        normalized
    }
}

impl Default for HlacFeature {
    fn default() -> Self {
        Self::zeros()
    }
}

/// グレースケールHLACの25次元特徴量。
#[derive(Debug, Clone, PartialEq)]
pub struct GrayHlacFeature {
    pub sums: [f64; HLAC_DIM],
}

impl GrayHlacFeature {
    pub const fn new(sums: [f64; HLAC_DIM]) -> Self {
        Self { sums }
    }

    pub const fn zeros() -> Self {
        Self {
            sums: [0.0; HLAC_DIM],
        }
    }

    /// 任意の分母で正規化したf64配列を返す。
    ///
    /// 分母が0以下の場合はゼロ配列を返す。
    pub fn as_f64_normalized(&self, denom: f64) -> [f64; HLAC_DIM] {
        if denom <= 0.0 {
            return [0.0; HLAC_DIM];
        }

        let mut normalized = [0.0; HLAC_DIM];
        for (idx, value) in self.sums.iter().enumerate() {
            normalized[idx] = *value / denom;
        }
        normalized
    }
}

impl Default for GrayHlacFeature {
    fn default() -> Self {
        Self::zeros()
    }
}

/// HLAC抽出時のエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlacError {
    ImageTooSmall {
        width: usize,
        height: usize,
    },
    InvalidBufferLength {
        width: usize,
        height: usize,
        len: usize,
    },
    NonBinaryValue {
        x: usize,
        y: usize,
        value: u8,
    },
}

impl fmt::Display for HlacError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HlacError::ImageTooSmall { width, height } => {
                write!(
                    f,
                    "画像サイズが小さすぎます: width={width}, height={height}, 必要最小サイズは3x3"
                )
            }
            HlacError::InvalidBufferLength { width, height, len } => {
                write!(
                    f,
                    "バッファ長が不正です: width={width}, height={height}, len={len}"
                )
            }
            HlacError::NonBinaryValue { x, y, value } => {
                write!(f, "2値以外の値を検出しました: x={x}, y={y}, value={value}")
            }
        }
    }
}

impl std::error::Error for HlacError {}

/// `u8` 輝度配列をしきい値で2値化する。
///
/// `src` は行優先（row-major）で `width * height` 要素を持つ必要がある。
pub fn u8_to_binary_array(
    src: &[u8],
    width: usize,
    height: usize,
    threshold: u8,
) -> Result<Vec<bool>, HlacError> {
    validate_buffer_len(width, height, src.len())?;
    Ok(src.iter().map(|value| *value > threshold).collect())
}

/// 2値HLAC抽出器。
#[derive(Debug, Clone)]
pub struct HlacExtractor {
    masks: [&'static [(isize, isize)]; HLAC_DIM],
}

impl Default for HlacExtractor {
    fn default() -> Self {
        Self::new_binary_25()
    }
}

impl HlacExtractor {
    /// 3x3・25次元の標準HLAC抽出器を生成する。
    pub fn new_binary_25() -> Self {
        Self {
            masks: HLAC25_OFFSETS,
        }
    }

    /// `bool` 2値配列（`true`=1, `false`=0）から2値HLACを抽出する。
    ///
    /// `image` は行優先（row-major）で `width * height` 要素を持つ必要がある。
    pub fn extract_binary_bool(
        &self,
        image: &[bool],
        width: usize,
        height: usize,
    ) -> Result<HlacFeature, HlacError> {
        Self::validate_shape(width, height)?;
        validate_buffer_len(width, height, image.len())?;

        Ok(self.extract_core_unchecked(width, height, |y, x| image[flat_index(width, y, x)]))
    }

    /// `u8` 2値配列（0/1のみ許可）から2値HLACを抽出する。
    ///
    /// `image` は行優先（row-major）で `width * height` 要素を持つ必要がある。
    pub fn extract_binary_u8(
        &self,
        image: &[u8],
        width: usize,
        height: usize,
    ) -> Result<HlacFeature, HlacError> {
        Self::validate_shape(width, height)?;
        validate_buffer_len(width, height, image.len())?;
        self.validate_binary_u8_values(image, width)?;

        Ok(self.extract_core_unchecked(width, height, |y, x| image[flat_index(width, y, x)] == 1))
    }

    /// `u8` 2値配列（0/1のみ許可）から、`wide` を使って2値HLACを抽出する。
    ///
    /// `image` は行優先（row-major）で `width * height` 要素を持つ必要がある。
    pub fn extract_binary_u8_simd(
        &self,
        image: &[u8],
        width: usize,
        height: usize,
    ) -> Result<HlacFeature, HlacError> {
        Self::validate_shape(width, height)?;
        validate_buffer_len(width, height, image.len())?;
        self.validate_binary_u8_values(image, width)?;

        Ok(self.extract_binary_u8_simd_unchecked(image, width, height))
    }

    /// `u8` グレースケール配列から積和HLACを抽出する。
    ///
    /// `image` は行優先（row-major）で `width * height` 要素を持つ必要がある。
    pub fn extract_gray_u8(
        &self,
        image: &[u8],
        width: usize,
        height: usize,
    ) -> Result<GrayHlacFeature, HlacError> {
        Self::validate_shape(width, height)?;
        validate_buffer_len(width, height, image.len())?;

        Ok(self.extract_gray_core_unchecked(width, height, |y, x| image[flat_index(width, y, x)]))
    }

    fn validate_binary_u8_values(&self, image: &[u8], width: usize) -> Result<(), HlacError> {
        for (idx, value) in image.iter().enumerate() {
            if *value > 1 {
                return Err(HlacError::NonBinaryValue {
                    x: idx % width,
                    y: idx / width,
                    value: *value,
                });
            }
        }
        Ok(())
    }

    fn validate_shape(width: usize, height: usize) -> Result<(), HlacError> {
        if width < 3 || height < 3 {
            return Err(HlacError::ImageTooSmall { width, height });
        }
        Ok(())
    }

    fn extract_core_unchecked<F>(&self, width: usize, height: usize, mut is_on: F) -> HlacFeature
    where
        F: FnMut(usize, usize) -> bool,
    {
        let mut counts = [0_u64; HLAC_DIM];

        for y in 1..(height - 1) {
            for x in 1..(width - 1) {
                for (mask_idx, mask) in self.masks.iter().enumerate() {
                    let mut matched = true;

                    for &(dx, dy) in *mask {
                        let yy = (y as isize + dy) as usize;
                        let xx = (x as isize + dx) as usize;
                        if !is_on(yy, xx) {
                            matched = false;
                            break;
                        }
                    }

                    if matched {
                        counts[mask_idx] += 1;
                    }
                }
            }
        }

        HlacFeature::new(counts)
    }

    fn extract_binary_u8_simd_unchecked(
        &self,
        image: &[u8],
        width: usize,
        height: usize,
    ) -> HlacFeature {
        let mut counts = [0_u64; HLAC_DIM];

        for y in 1..(height - 1) {
            let mut x = 1;

            while x + SIMD_LANES <= width - 1 {
                for (mask_idx, mask) in self.masks.iter().enumerate() {
                    let (dx0, dy0) = mask[0];
                    let mut matched = load_u8x16_lane(image, width, y, x, dx0, dy0);

                    for &(dx, dy) in &mask[1..] {
                        matched &= load_u8x16_lane(image, width, y, x, dx, dy);
                    }

                    counts[mask_idx] += count_true_lanes(matched);
                }

                x += SIMD_LANES;
            }

            for xx in x..(width - 1) {
                for (mask_idx, mask) in self.masks.iter().enumerate() {
                    let mut all_on = true;

                    for &(dx, dy) in *mask {
                        let yy = (y as isize + dy) as usize;
                        let xn = (xx as isize + dx) as usize;
                        if image[flat_index(width, yy, xn)] != 1 {
                            all_on = false;
                            break;
                        }
                    }

                    if all_on {
                        counts[mask_idx] += 1;
                    }
                }
            }
        }

        HlacFeature::new(counts)
    }

    fn extract_gray_core_unchecked<F>(
        &self,
        width: usize,
        height: usize,
        mut pixel: F,
    ) -> GrayHlacFeature
    where
        F: FnMut(usize, usize) -> u8,
    {
        let mut sums = [0.0_f64; HLAC_DIM];

        for y in 1..(height - 1) {
            for x in 1..(width - 1) {
                for (mask_idx, mask) in self.masks.iter().enumerate() {
                    let mut acc = 1.0_f64;

                    for &(dx, dy) in *mask {
                        let yy = (y as isize + dy) as usize;
                        let xx = (x as isize + dx) as usize;
                        acc *= f64::from(pixel(yy, xx));
                    }

                    sums[mask_idx] += acc;
                }
            }
        }

        GrayHlacFeature::new(sums)
    }
}

#[inline]
fn flat_index(width: usize, y: usize, x: usize) -> usize {
    y * width + x
}

#[inline]
fn load_u8x16_lane(image: &[u8], width: usize, y: usize, x: usize, dx: isize, dy: isize) -> u8x16 {
    let yy = (y as isize + dy) as usize;
    let xx = (x as isize + dx) as usize;
    let start = flat_index(width, yy, xx);
    let lane: [u8; SIMD_LANES] = image[start..start + SIMD_LANES]
        .try_into()
        .expect("SIMD lane load must have exact width");
    u8x16::from(lane)
}

#[inline]
fn count_true_lanes(v: u8x16) -> u64 {
    v.to_array()
        .iter()
        .map(|value| u64::from(*value == 1))
        .sum()
}

fn validate_buffer_len(width: usize, height: usize, len: usize) -> Result<(), HlacError> {
    let Some(expected) = width.checked_mul(height) else {
        return Err(HlacError::InvalidBufferLength { width, height, len });
    };

    if len != expected {
        return Err(HlacError::InvalidBufferLength { width, height, len });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Luma};
    use ndarray::Array2;

    use crate::mask::HLAC25_OFFSETS;

    use super::{
        u8_to_binary_array, GrayHlacFeature, HlacError, HlacExtractor, HlacFeature, HLAC_DIM,
    };

    fn build_image(height: usize, width: usize, ones: &[(usize, usize)]) -> Array2<bool> {
        let mut image = Array2::from_elem((height, width), false);
        for &(y, x) in ones {
            image[(y, x)] = true;
        }
        image
    }

    fn merge_image(lhs: &Array2<bool>, rhs: &Array2<bool>) -> Array2<bool> {
        assert_eq!(lhs.dim(), rhs.dim());
        let (height, width) = lhs.dim();
        let mut merged = Array2::from_elem((height, width), false);

        for y in 0..height {
            for x in 0..width {
                merged[(y, x)] = lhs[(y, x)] || rhs[(y, x)];
            }
        }

        merged
    }

    fn build_gray_image(height: usize, width: usize, values: &[(usize, usize, u8)]) -> Array2<u8> {
        let mut image = Array2::from_elem((height, width), 0_u8);
        for &(y, x, value) in values {
            image[(y, x)] = value;
        }
        image
    }

    fn merge_gray_image_disjoint(lhs: &Array2<u8>, rhs: &Array2<u8>) -> Array2<u8> {
        assert_eq!(lhs.dim(), rhs.dim());
        let (height, width) = lhs.dim();
        let mut merged = Array2::from_elem((height, width), 0_u8);

        for y in 0..height {
            for x in 0..width {
                assert!(
                    !(lhs[(y, x)] > 0 && rhs[(y, x)] > 0),
                    "gray additivity test requires disjoint supports"
                );
                merged[(y, x)] = lhs[(y, x)].saturating_add(rhs[(y, x)]);
            }
        }

        merged
    }

    fn sum_feature(lhs: &HlacFeature, rhs: &HlacFeature) -> [u64; HLAC_DIM] {
        let mut sum = [0_u64; HLAC_DIM];
        for (idx, item) in sum.iter_mut().enumerate() {
            *item = lhs.counts[idx] + rhs.counts[idx];
        }
        sum
    }

    fn sum_gray_feature(lhs: &GrayHlacFeature, rhs: &GrayHlacFeature) -> [f64; HLAC_DIM] {
        let mut sum = [0.0_f64; HLAC_DIM];
        for (idx, item) in sum.iter_mut().enumerate() {
            *item = lhs.sums[idx] + rhs.sums[idx];
        }
        sum
    }

    fn extract_binary_bool_array(
        extractor: &HlacExtractor,
        image: &Array2<bool>,
    ) -> Result<HlacFeature, HlacError> {
        let (height, width) = image.dim();
        extractor.extract_binary_bool(
            image
                .as_slice()
                .expect("binary bool ndarray must be contiguous"),
            width,
            height,
        )
    }

    fn extract_binary_u8_array(
        extractor: &HlacExtractor,
        image: &Array2<u8>,
    ) -> Result<HlacFeature, HlacError> {
        let (height, width) = image.dim();
        extractor.extract_binary_u8(
            image
                .as_slice()
                .expect("binary u8 ndarray must be contiguous"),
            width,
            height,
        )
    }

    fn extract_binary_u8_simd_array(
        extractor: &HlacExtractor,
        image: &Array2<u8>,
    ) -> Result<HlacFeature, HlacError> {
        let (height, width) = image.dim();
        extractor.extract_binary_u8_simd(
            image
                .as_slice()
                .expect("binary u8 ndarray must be contiguous"),
            width,
            height,
        )
    }

    fn extract_gray_u8_array(
        extractor: &HlacExtractor,
        image: &Array2<u8>,
    ) -> Result<GrayHlacFeature, HlacError> {
        let (height, width) = image.dim();
        extractor.extract_gray_u8(
            image
                .as_slice()
                .expect("gray u8 ndarray must be contiguous"),
            width,
            height,
        )
    }

    fn assert_counts_range(feature: &HlacFeature, valid_positions: u64, case_name: &str) {
        for (idx, value) in feature.counts.iter().enumerate() {
            assert!(
                *value <= valid_positions,
                "case={case_name}, pattern={idx}, value={value}, valid_positions={valid_positions}"
            );
        }
    }

    fn assert_feature_eq(actual: &HlacFeature, expected: &[u64; HLAC_DIM], case_name: &str) {
        assert_eq!(
            &actual.counts, expected,
            "case={case_name}, expected={expected:?}, actual={:?}",
            actual.counts
        );
    }

    fn assert_gray_non_negative_finite(feature: &GrayHlacFeature, case_name: &str) {
        for (idx, value) in feature.sums.iter().enumerate() {
            assert!(
                value.is_finite(),
                "case={case_name}, pattern={idx}, value={value} is not finite"
            );
            assert!(
                *value >= 0.0,
                "case={case_name}, pattern={idx}, value={value} must be >= 0"
            );
        }
    }

    fn assert_gray_feature_close(
        actual: &GrayHlacFeature,
        expected: &[f64; HLAC_DIM],
        eps: f64,
        case_name: &str,
    ) {
        for (idx, (a, e)) in actual.sums.iter().zip(expected.iter()).enumerate() {
            let diff = (a - e).abs();
            assert!(
                diff <= eps,
                "case={case_name}, pattern={idx}, expected={e}, actual={a}, diff={diff}, eps={eps}"
            );
        }
    }

    fn assert_error_eq<T: std::fmt::Debug>(
        result: Result<T, HlacError>,
        expected: &HlacError,
        case_name: &str,
    ) {
        match result {
            Ok(value) => panic!("case={case_name}, expected error={expected:?}, actual={value:?}"),
            Err(actual) => assert_eq!(
                &actual, expected,
                "case={case_name}, expected={expected:?}, actual={actual:?}"
            ),
        }
    }

    #[test]
    fn hlac_binary_value_range_cases() {
        struct Case {
            name: &'static str,
            image: Array2<bool>,
            require_center_dominance: bool,
        }

        let cases = vec![
            Case {
                name: "all_zero_3x3",
                image: Array2::from_elem((3, 3), false),
                require_center_dominance: false,
            },
            Case {
                name: "all_one_3x3",
                image: Array2::from_elem((3, 3), true),
                require_center_dominance: true,
            },
            Case {
                name: "single_one_4x4",
                image: build_image(4, 4, &[(1, 1)]),
                require_center_dominance: false,
            },
        ];

        let extractor = HlacExtractor::new_binary_25();

        for case in cases {
            let feature = extract_binary_bool_array(&extractor, &case.image).unwrap();
            let (height, width) = case.image.dim();
            let valid_positions = ((height - 2) * (width - 2)) as u64;

            assert_counts_range(&feature, valid_positions, case.name);

            if case.require_center_dominance {
                let center = feature.counts[0];
                assert!(
                    feature.counts[1..].iter().all(|value| center >= *value),
                    "case={}, center={}, others={:?}",
                    case.name,
                    center,
                    &feature.counts[1..]
                );
            }
        }
    }

    #[test]
    fn hlac_binary_normal_cases() {
        enum Scenario {
            ShiftInvariant {
                lhs: Array2<bool>,
                rhs: Array2<bool>,
            },
            Additivity {
                lhs: Array2<bool>,
                rhs: Array2<bool>,
                merged: Array2<bool>,
            },
            Golden {
                image: Array2<bool>,
                expected: [u64; HLAC_DIM],
            },
        }

        struct Case {
            name: &'static str,
            scenario: Scenario,
        }

        let shift_base_points = &[(2, 2), (2, 3), (3, 2), (3, 3), (3, 4)];
        let shift_moved_points = &[(3, 4), (3, 5), (4, 4), (4, 5), (4, 6)];

        let add_lhs = build_image(12, 12, &[(2, 2), (2, 3), (3, 2), (3, 3)]);
        let add_rhs = build_image(12, 12, &[(8, 8), (8, 9), (9, 8), (9, 9)]);
        let add_merged = merge_image(&add_lhs, &add_rhs);

        let cases = vec![
            Case {
                name: "shift_invariance",
                scenario: Scenario::ShiftInvariant {
                    lhs: build_image(9, 9, shift_base_points),
                    rhs: build_image(9, 9, shift_moved_points),
                },
            },
            Case {
                name: "additivity",
                scenario: Scenario::Additivity {
                    lhs: add_lhs,
                    rhs: add_rhs,
                    merged: add_merged,
                },
            },
            Case {
                name: "golden_all_one_3x3",
                scenario: Scenario::Golden {
                    image: Array2::from_elem((3, 3), true),
                    expected: [1; HLAC_DIM],
                },
            },
        ];

        let extractor = HlacExtractor::new_binary_25();

        for case in cases {
            match case.scenario {
                Scenario::ShiftInvariant { lhs, rhs } => {
                    let lhs_feature = extract_binary_bool_array(&extractor, &lhs).unwrap();
                    let rhs_feature = extract_binary_bool_array(&extractor, &rhs).unwrap();
                    assert_feature_eq(&lhs_feature, &rhs_feature.counts, case.name);
                }
                Scenario::Additivity { lhs, rhs, merged } => {
                    let lhs_feature = extract_binary_bool_array(&extractor, &lhs).unwrap();
                    let rhs_feature = extract_binary_bool_array(&extractor, &rhs).unwrap();
                    let merged_feature = extract_binary_bool_array(&extractor, &merged).unwrap();
                    let expected = sum_feature(&lhs_feature, &rhs_feature);
                    assert_feature_eq(&merged_feature, &expected, case.name);
                }
                Scenario::Golden { image, expected } => {
                    let feature = extract_binary_bool_array(&extractor, &image).unwrap();
                    assert_feature_eq(&feature, &expected, case.name);
                }
            }
        }
    }

    #[test]
    fn hlac_binary_error_cases() {
        enum ErrorInput {
            Bool(Array2<bool>),
            U8(Array2<u8>),
            RawBool {
                image: Vec<bool>,
                width: usize,
                height: usize,
            },
        }

        struct Case {
            name: &'static str,
            input: ErrorInput,
            expected: HlacError,
        }

        let mut non_binary = Array2::from_elem((3, 3), 0_u8);
        non_binary[(1, 1)] = 2;

        let cases = vec![
            Case {
                name: "too_small_height",
                input: ErrorInput::Bool(Array2::from_elem((2, 4), false)),
                expected: HlacError::ImageTooSmall {
                    width: 4,
                    height: 2,
                },
            },
            Case {
                name: "too_small_width",
                input: ErrorInput::Bool(Array2::from_elem((4, 2), false)),
                expected: HlacError::ImageTooSmall {
                    width: 2,
                    height: 4,
                },
            },
            Case {
                name: "invalid_buffer_len",
                input: ErrorInput::RawBool {
                    image: vec![false; 8],
                    width: 3,
                    height: 3,
                },
                expected: HlacError::InvalidBufferLength {
                    width: 3,
                    height: 3,
                    len: 8,
                },
            },
            Case {
                name: "non_binary_u8",
                input: ErrorInput::U8(non_binary),
                expected: HlacError::NonBinaryValue {
                    x: 1,
                    y: 1,
                    value: 2,
                },
            },
        ];

        let extractor = HlacExtractor::new_binary_25();

        for case in cases {
            let result = match case.input {
                ErrorInput::Bool(image) => extract_binary_bool_array(&extractor, &image),
                ErrorInput::U8(image) => extract_binary_u8_array(&extractor, &image),
                ErrorInput::RawBool {
                    image,
                    width,
                    height,
                } => extractor.extract_binary_bool(&image, width, height),
            };
            assert_error_eq(result, &case.expected, case.name);
        }
    }

    #[test]
    fn hlac_binary_simd_matches_scalar_cases() {
        struct Case {
            name: &'static str,
            image: Array2<u8>,
        }

        let mut generated = Array2::from_elem((32, 64), 0_u8);
        for y in 0..32 {
            for x in 0..64 {
                generated[(y, x)] = if ((x * 31 + y * 17 + x * y) % 5) < 2 {
                    1
                } else {
                    0
                };
            }
        }

        let cases = vec![
            Case {
                name: "small_width_tail_only",
                image: Array2::from_shape_vec(
                    (4, 6),
                    vec![
                        0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 1, 0,
                    ],
                )
                .unwrap(),
            },
            Case {
                name: "one_simd_chunk",
                image: Array2::from_shape_vec(
                    (5, 18),
                    (0..90)
                        .map(|v| if (v % 3) == 0 { 1_u8 } else { 0_u8 })
                        .collect(),
                )
                .unwrap(),
            },
            Case {
                name: "multi_chunk_generated",
                image: generated,
            },
        ];

        let extractor = HlacExtractor::new_binary_25();

        for case in cases {
            let scalar = extract_binary_u8_array(&extractor, &case.image).unwrap();
            let simd = extract_binary_u8_simd_array(&extractor, &case.image).unwrap();
            assert_eq!(scalar, simd, "case={}", case.name);
        }
    }

    #[test]
    fn image_luma8_to_binary_ndarray_normal_cases() {
        struct Case {
            name: &'static str,
            width: u32,
            height: u32,
            raw: Vec<u8>,
            threshold: u8,
            expected: Array2<bool>,
        }

        let cases = vec![
            Case {
                name: "threshold_127_2x3",
                width: 3,
                height: 2,
                raw: vec![0, 127, 128, 255, 1, 200],
                threshold: 127,
                expected: Array2::from_shape_vec(
                    (2, 3),
                    vec![false, false, true, true, false, true],
                )
                .unwrap(),
            },
            Case {
                name: "threshold_0_2x2",
                width: 2,
                height: 2,
                raw: vec![0, 1, 2, 0],
                threshold: 0,
                expected: Array2::from_shape_vec((2, 2), vec![false, true, true, false]).unwrap(),
            },
        ];

        for case in cases {
            let image =
                ImageBuffer::<Luma<u8>, Vec<u8>>::from_vec(case.width, case.height, case.raw)
                    .expect("invalid image buffer");
            let actual = u8_to_binary_array(
                image.as_raw(),
                case.width as usize,
                case.height as usize,
                case.threshold,
            )
            .unwrap();
            let actual =
                Array2::from_shape_vec((case.height as usize, case.width as usize), actual)
                    .unwrap();
            assert_eq!(actual, case.expected, "case={}", case.name);
        }
    }

    #[test]
    fn image_luma8_to_binary_ndarray_error_cases() {
        let actual = u8_to_binary_array(&[0_u8, 1, 2], 2, 2, 0);
        assert_error_eq(
            actual,
            &HlacError::InvalidBufferLength {
                width: 2,
                height: 2,
                len: 3,
            },
            "invalid_buffer_len",
        );
    }

    #[test]
    fn hlac_gray_value_range_cases() {
        struct Case {
            name: &'static str,
            image: Array2<u8>,
        }

        let mut ramp = Array2::from_elem((4, 4), 0_u8);
        for y in 0..4 {
            for x in 0..4 {
                ramp[(y, x)] = (y * 4 + x) as u8;
            }
        }

        let cases = vec![
            Case {
                name: "all_zero_3x3",
                image: Array2::from_elem((3, 3), 0_u8),
            },
            Case {
                name: "all_255_3x3",
                image: Array2::from_elem((3, 3), 255_u8),
            },
            Case {
                name: "ramp_4x4",
                image: ramp,
            },
        ];

        let extractor = HlacExtractor::new_binary_25();

        for case in cases {
            let feature = extract_gray_u8_array(&extractor, &case.image).unwrap();
            let (height, width) = case.image.dim();
            let valid_positions = ((height - 2) * (width - 2)) as f64;
            let max_possible = valid_positions * 255_f64.powi(3);

            assert_gray_non_negative_finite(&feature, case.name);

            for (idx, value) in feature.sums.iter().enumerate() {
                assert!(
                    *value <= max_possible + 1e-9,
                    "case={}, pattern={}, value={}, max_possible={}",
                    case.name,
                    idx,
                    value,
                    max_possible
                );
            }
        }
    }

    #[test]
    fn hlac_gray_normal_cases() {
        enum Scenario {
            ShiftInvariant {
                lhs: Array2<u8>,
                rhs: Array2<u8>,
            },
            Additivity {
                lhs: Array2<u8>,
                rhs: Array2<u8>,
                merged: Array2<u8>,
            },
            Golden {
                image: Array2<u8>,
                expected: [f64; HLAC_DIM],
            },
        }

        struct Case {
            name: &'static str,
            scenario: Scenario,
        }

        let shift_lhs = build_gray_image(
            9,
            9,
            &[(2, 2, 10), (2, 3, 20), (3, 2, 30), (3, 3, 40), (3, 4, 50)],
        );
        let shift_rhs = build_gray_image(
            9,
            9,
            &[(3, 4, 10), (3, 5, 20), (4, 4, 30), (4, 5, 40), (4, 6, 50)],
        );

        let add_lhs = build_gray_image(12, 12, &[(2, 2, 2), (2, 3, 2), (3, 2, 2), (3, 3, 2)]);
        let add_rhs = build_gray_image(12, 12, &[(8, 8, 3), (8, 9, 3), (9, 8, 3), (9, 9, 3)]);
        let add_merged = merge_gray_image_disjoint(&add_lhs, &add_rhs);

        let mut expected_255 = [0.0_f64; HLAC_DIM];
        for (idx, mask) in HLAC25_OFFSETS.iter().enumerate() {
            expected_255[idx] = 255_f64.powi(mask.len() as i32);
        }

        let cases = vec![
            Case {
                name: "shift_invariance",
                scenario: Scenario::ShiftInvariant {
                    lhs: shift_lhs,
                    rhs: shift_rhs,
                },
            },
            Case {
                name: "additivity_disjoint",
                scenario: Scenario::Additivity {
                    lhs: add_lhs,
                    rhs: add_rhs,
                    merged: add_merged,
                },
            },
            Case {
                name: "golden_all_one_3x3",
                scenario: Scenario::Golden {
                    image: Array2::from_elem((3, 3), 1_u8),
                    expected: [1.0; HLAC_DIM],
                },
            },
            Case {
                name: "golden_all_255_3x3",
                scenario: Scenario::Golden {
                    image: Array2::from_elem((3, 3), 255_u8),
                    expected: expected_255,
                },
            },
        ];

        let extractor = HlacExtractor::new_binary_25();

        for case in cases {
            match case.scenario {
                Scenario::ShiftInvariant { lhs, rhs } => {
                    let lhs_feature = extract_gray_u8_array(&extractor, &lhs).unwrap();
                    let rhs_feature = extract_gray_u8_array(&extractor, &rhs).unwrap();
                    assert_gray_feature_close(&lhs_feature, &rhs_feature.sums, 1e-9, case.name);
                }
                Scenario::Additivity { lhs, rhs, merged } => {
                    let lhs_feature = extract_gray_u8_array(&extractor, &lhs).unwrap();
                    let rhs_feature = extract_gray_u8_array(&extractor, &rhs).unwrap();
                    let merged_feature = extract_gray_u8_array(&extractor, &merged).unwrap();
                    let expected = sum_gray_feature(&lhs_feature, &rhs_feature);
                    assert_gray_feature_close(&merged_feature, &expected, 1e-9, case.name);
                }
                Scenario::Golden { image, expected } => {
                    let feature = extract_gray_u8_array(&extractor, &image).unwrap();
                    assert_gray_feature_close(&feature, &expected, 1e-9, case.name);
                }
            }
        }
    }

    #[test]
    fn hlac_gray_error_cases() {
        struct Case {
            name: &'static str,
            image: Array2<u8>,
            expected: HlacError,
        }

        let cases = vec![
            Case {
                name: "too_small_height",
                image: Array2::from_elem((2, 4), 0_u8),
                expected: HlacError::ImageTooSmall {
                    width: 4,
                    height: 2,
                },
            },
            Case {
                name: "too_small_width",
                image: Array2::from_elem((4, 2), 0_u8),
                expected: HlacError::ImageTooSmall {
                    width: 2,
                    height: 4,
                },
            },
            Case {
                name: "invalid_buffer_len",
                image: Array2::from_elem((3, 3), 0_u8),
                expected: HlacError::InvalidBufferLength {
                    width: 3,
                    height: 3,
                    len: 8,
                },
            },
        ];

        let extractor = HlacExtractor::new_binary_25();

        for case in cases {
            let result = if case.name == "invalid_buffer_len" {
                extractor.extract_gray_u8(
                    &case
                        .image
                        .as_slice()
                        .expect("gray ndarray must be contiguous")[..8],
                    3,
                    3,
                )
            } else {
                extract_gray_u8_array(&extractor, &case.image)
            };
            assert_error_eq(result, &case.expected, case.name);
        }
    }
}
