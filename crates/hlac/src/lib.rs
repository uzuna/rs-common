//! HLAC (Higher-order Local Auto-Correlation) を扱うクレート。
//!
//! Phase2では2値HLAC（3x3, 25パターン）に加え、
//! グレースケール積和HLACと輝度配列からの2値化変換を提供する。
//!
//! # 2値HLACの最小例
//! ```
//! use hlac::HlacExtractor;
//!
//! let image = vec![
//!     false, false, false,
//!     false, true, true,
//!     false, true, true,
//! ];
//! let extractor = HlacExtractor::new_binary_25();
//! let feature = extractor.extract_binary_bool(&image, 3, 3).unwrap();
//! assert_eq!(feature.counts.len(), 25);
//! ```
//!
//! # 輝度配列を2値化する最小例
//! ```
//! use hlac::u8_to_binary_array;
//!
//! let src = vec![0, 128, 255, 127];
//! let bin = u8_to_binary_array(&src, 2, 2, 127).unwrap();
//! assert_eq!(bin, vec![false, true, true, false]);
//! ```

pub mod binary;
pub mod mask;

pub use binary::{u8_to_binary_array, GrayHlacFeature, HlacError, HlacExtractor, HlacFeature};
pub use mask::{HLAC25_OFFSETS, HLAC_DIM};
