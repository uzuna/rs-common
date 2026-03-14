use std::fmt;

use ndarray::ArrayView2;

use crate::mask::{HLAC25_OFFSETS, HLAC_DIM};

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

/// HLAC抽出時のエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlacError {
    ImageTooSmall { width: usize, height: usize },
    NonBinaryValue { x: usize, y: usize, value: u8 },
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
            HlacError::NonBinaryValue { x, y, value } => {
                write!(f, "2値以外の値を検出しました: x={x}, y={y}, value={value}")
            }
        }
    }
}

impl std::error::Error for HlacError {}

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
    /// 3x3・25次元の標準2値HLAC抽出器を生成する。
    pub fn new_binary_25() -> Self {
        Self {
            masks: HLAC25_OFFSETS,
        }
    }

    /// `bool` 配列（`true`=1, `false`=0）から2値HLACを抽出する。
    pub fn extract_binary_bool(
        &self,
        image: ArrayView2<'_, bool>,
    ) -> Result<HlacFeature, HlacError> {
        let (height, width) = image.dim();
        Self::validate_shape(width, height)?;
        Ok(self.extract_core_unchecked(width, height, |y, x| image[(y, x)]))
    }

    /// `u8` 配列（0/1のみ許可）から2値HLACを抽出する。
    pub fn extract_binary_u8(&self, image: ArrayView2<'_, u8>) -> Result<HlacFeature, HlacError> {
        let (height, width) = image.dim();
        Self::validate_shape(width, height)?;

        for ((y, x), value) in image.indexed_iter() {
            if *value > 1 {
                return Err(HlacError::NonBinaryValue {
                    x,
                    y,
                    value: *value,
                });
            }
        }

        Ok(self.extract_core_unchecked(width, height, |y, x| image[(y, x)] == 1))
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
}

#[cfg(test)]
mod tests {
    use ndarray::Array2;

    use super::{HlacError, HlacExtractor, HlacFeature, HLAC_DIM};

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

    fn sum_feature(lhs: &HlacFeature, rhs: &HlacFeature) -> [u64; HLAC_DIM] {
        let mut sum = [0_u64; HLAC_DIM];
        for (idx, item) in sum.iter_mut().enumerate() {
            *item = lhs.counts[idx] + rhs.counts[idx];
        }
        sum
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

    fn assert_error_eq(
        result: Result<HlacFeature, HlacError>,
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
            let feature = extractor.extract_binary_bool(case.image.view()).unwrap();
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
                    let lhs_feature = extractor.extract_binary_bool(lhs.view()).unwrap();
                    let rhs_feature = extractor.extract_binary_bool(rhs.view()).unwrap();
                    assert_feature_eq(&lhs_feature, &rhs_feature.counts, case.name);
                }
                Scenario::Additivity { lhs, rhs, merged } => {
                    let lhs_feature = extractor.extract_binary_bool(lhs.view()).unwrap();
                    let rhs_feature = extractor.extract_binary_bool(rhs.view()).unwrap();
                    let merged_feature = extractor.extract_binary_bool(merged.view()).unwrap();
                    let expected = sum_feature(&lhs_feature, &rhs_feature);
                    assert_feature_eq(&merged_feature, &expected, case.name);
                }
                Scenario::Golden { image, expected } => {
                    let feature = extractor.extract_binary_bool(image.view()).unwrap();
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
                ErrorInput::Bool(image) => extractor.extract_binary_bool(image.view()),
                ErrorInput::U8(image) => extractor.extract_binary_u8(image.view()),
            };
            assert_error_eq(result, &case.expected, case.name);
        }
    }
}
