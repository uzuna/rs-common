//! 表示ドメインで使う型レベル制約。

use std::fmt;

/// 表示ドメイン型の生成・変換で発生するエラー。
/// 各 NewType の `TryFrom` 実装が返す失敗理由を列挙する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayTypeError {
    /// 空白のみの文字列が渡された（`NonEmptyText`）。
    EmptyText,
    /// 空白のみの文字列が sample_id として渡された（`SampleId`）。
    EmptySampleId,
    /// 0 以下の値が渡された。正の値が必要（`PositiveF32`）。
    NonPositive,
    /// 負の値が渡された。0 以上が必要（`NonNegativeF32`）。
    Negative,
    /// 許容範囲外の値が渡された（`Percent` > 100、`Normalized` > 1.0 など）。
    OutOfRange,
    /// 空の配列が history として渡された（`HistoryPoints`）。
    EmptyHistory,
    /// `HistoryPoints::MAX_LEN` を超える配列が渡された（`HistoryPoints`）。
    TooLongHistory,
}

impl fmt::Display for DisplayTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisplayTypeError::EmptyText => write!(f, "空文字は許可されません"),
            DisplayTypeError::EmptySampleId => write!(f, "sample_id が空です"),
            DisplayTypeError::NonPositive => write!(f, "0 より大きい値が必要です"),
            DisplayTypeError::Negative => write!(f, "0 以上の値が必要です"),
            DisplayTypeError::OutOfRange => write!(f, "許容範囲外の値です"),
            DisplayTypeError::EmptyHistory => write!(f, "history が空です"),
            DisplayTypeError::TooLongHistory => write!(f, "history が長すぎます"),
        }
    }
}

impl std::error::Error for DisplayTypeError {}

/// `display.samples[].id` を表す識別子。空白のみの文字列は拒否する。
/// カタログ検索では文字列参照（`as_str()`）で照合する。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SampleId(String);

impl SampleId {
    /// 保持している ID 文字列をスライスで返す。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SampleId {
    type Error = DisplayTypeError;

    /// 空白のみの文字列を拒否し、有効な場合に `SampleId` を生成する。
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err(DisplayTypeError::EmptySampleId);
        }
        Ok(Self(value))
    }
}

/// 空白のみを禁止したテキスト NewType。
/// ラベル・タイトル・アイコン名など、描画に必須の文字列フィールドに使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyText(String);

impl NonEmptyText {
    /// 保持しているテキストをスライスで返す。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NonEmptyText {
    type Error = DisplayTypeError;

    /// 空白のみの文字列を拒否し、有効な場合に `NonEmptyText` を生成する。
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err(DisplayTypeError::EmptyText);
        }
        Ok(Self(value))
    }
}

/// 0 より大きい浮動小数点数を保証する NewType。
/// `bar_trend.max`（最大値）など、分母として使う値の誤った 0 を防ぐ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositiveF32(f32);

impl PositiveF32 {
    /// 保持している値を返す。
    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for PositiveF32 {
    type Error = DisplayTypeError;

    /// 0 以下の値を拒否し、有効な場合に `PositiveF32` を生成する。
    fn try_from(value: f32) -> Result<Self, Self::Error> {
        if value > 0.0 {
            Ok(Self(value))
        } else {
            Err(DisplayTypeError::NonPositive)
        }
    }
}

/// 0 以上の浮動小数点数を保証する NewType。
/// `bar_trend.current`（現在値）など、負になり得ない測定値に使う。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NonNegativeF32(f32);

impl NonNegativeF32 {
    /// 保持している値を返す。
    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for NonNegativeF32 {
    type Error = DisplayTypeError;

    /// 負の値を拒否し、有効な場合に `NonNegativeF32` を生成する。
    fn try_from(value: f32) -> Result<Self, Self::Error> {
        if value >= 0.0 {
            Ok(Self(value))
        } else {
            Err(DisplayTypeError::Negative)
        }
    }
}

/// 0〜100 の整数パーセント値を保証する NewType。
/// バー描画での充填率など、百分率として扱う値に使う。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percent(u8);

impl Percent {
    /// 保持しているパーセント値（0〜100）を返す。
    pub fn get(self) -> u8 {
        self.0
    }

    /// `current / max` の比率から `Percent` を計算する。
    /// `current > max` の場合は `OutOfRange` を返す。
    pub fn from_ratio(current: NonNegativeF32, max: PositiveF32) -> Result<Self, DisplayTypeError> {
        if current.get() > max.get() {
            return Err(DisplayTypeError::OutOfRange);
        }
        let ratio = current.get() / max.get();
        let pct = (ratio * 100.0).round() as u8;
        Ok(Self(pct))
    }
}

impl TryFrom<u8> for Percent {
    type Error = DisplayTypeError;

    /// 101 以上の値を拒否し、有効な場合に `Percent` を生成する。
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(DisplayTypeError::OutOfRange)
        }
    }
}

/// 0.0〜1.0 の正規化済み浮動小数点数を保証する NewType。
/// `HistoryPoints` の各要素など、描画入力として正規化された値に使う。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normalized(f32);

impl Normalized {
    /// 保持している正規化値（0.0〜1.0）を返す。
    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Normalized {
    type Error = DisplayTypeError;

    /// 0.0〜1.0 の範囲外を拒否し、有効な場合に `Normalized` を生成する。
    fn try_from(value: f32) -> Result<Self, Self::Error> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(DisplayTypeError::OutOfRange)
        }
    }
}

/// 正規化済み時系列データ列を保証する NewType。
/// 空配列禁止・上限長制約を持ち、スパークラインやバートレンドの描画入力に使う。
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryPoints(Vec<Normalized>);

impl HistoryPoints {
    /// 許容する履歴の最大要素数。
    pub const MAX_LEN: usize = 256;

    /// 保持している正規化値列をスライスで返す。
    pub fn as_slice(&self) -> &[Normalized] {
        &self.0
    }

    /// 正規化済みベクタから `HistoryPoints` を生成する。
    /// 空配列・`MAX_LEN` 超えの場合はエラーを返す。
    pub fn from_normalized(values: Vec<Normalized>) -> Result<Self, DisplayTypeError> {
        if values.is_empty() {
            return Err(DisplayTypeError::EmptyHistory);
        }
        if values.len() > Self::MAX_LEN {
            return Err(DisplayTypeError::TooLongHistory);
        }
        Ok(Self(values))
    }
}

/// RGB 色（各チャネル 0〜255）の意味論的ラッパー。
/// `[u8; 3]` をそのまま扱わず、色として扱う値を型で明示する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor([u8; 3]);

impl RgbColor {
    /// `[r, g, b]` 配列を返す。描画ライブラリへの受け渡しに使う。
    pub fn as_array(self) -> [u8; 3] {
        self.0
    }
}

impl From<[u8; 3]> for RgbColor {
    /// `[u8; 3]` 配列を `RgbColor` へ変換する。失敗しない。
    fn from(value: [u8; 3]) -> Self {
        Self(value)
    }
}
