//! 表示サンプルの中間表現。

use crate::config::DisplaySeverity;
use crate::display::types::{
    HistoryPoints, NonEmptyText, NonNegativeF32, Percent, PositiveF32, RgbColor,
};

/// 単一ラベル + 背景色の描画パターン仕様。
/// 固定アクションボタンなど、テキストと色だけで表現される場面に使う。
#[derive(Debug, Clone)]
pub struct LabelOnlySpec {
    /// ボタンまたはセクションに表示するラベルテキスト。
    pub label: NonEmptyText,
    /// ボタン/セクションの背景塌りつぶし色。
    pub bg_color: RgbColor,
}

/// タイトル + 値 + 単位 + 重要度のメトリクス描画仕様。
/// CPU 使用率・メモリ量など、定期更新される数値の表示に使う。
#[derive(Debug, Clone)]
pub struct LabelValueSpec {
    /// メトリクス名・項目名（例: "CPU"）。
    pub title: NonEmptyText,
    /// 現在値のテキスト表現（例: "72"）。
    pub value: NonEmptyText,
    /// 値に付随する単位文字列（例: "%"）。空許可。
    pub unit: String,
    /// 表示色の切り替えに使う重要度。
    pub severity: DisplaySeverity,
}

/// アイコン + 右上バッジの通知/ステータス描画仕様。
/// ベルアイコン + 未読件数など、アイコンとカウントの組合せを表示する場面に使う。
#[derive(Debug, Clone)]
pub struct IconBadgeSpec {
    /// 表示するアイコン名・記号（例: "bell"）。
    pub icon: NonEmptyText,
    /// アイコンに重ねるバッジの件数。
    pub badge_count: u32,
    /// 色分けに使うステータス重要度。
    pub status: DisplaySeverity,
}

/// バーグラフ + スパークラインの負荷/流入描画仕様。
/// `percent` でバーの充填、`history` でスパークラインを描画する。
#[derive(Debug, Clone)]
pub struct BarTrendSpec {
    /// バー表示対象の現在値。
    pub current: NonNegativeF32,
    /// バー・比率計算の分母になる最大値。
    pub max: PositiveF32,
    /// `current / max` を百分率に変換した値。バー幅の計算に使う。
    pub percent: Percent,
    /// 正規化済みの時系列履歴（古い順）。スパークラインの描画入力に使う。
    pub history: HistoryPoints,
}

/// エラー影鿹表示用の最小描画仕様。
/// データ取得失敗時など、正常な描画ができない場面で使う。
#[derive(Debug, Clone)]
pub struct ErrorFallbackSpec {
    /// 短いエラー識別子（例: "E_CONN"）。
    pub error_code: NonEmptyText,
    /// ユーザー向けの簡略メッセージ（例: "offline"）。
    pub message: NonEmptyText,
}

/// ボタン描画向けのサンプル仕様。
/// `catalog::resolve_button_sample()` が返す型であり、`button_patterns` が受け取る。
#[derive(Debug, Clone)]
pub enum ButtonSampleSpec {
    /// 単一ラベル + 背景色パターン。
    LabelOnly(LabelOnlySpec),
    /// タイトル + 値 + 単位パターン。
    LabelValue(LabelValueSpec),
    /// アイコン + バッジパターン。
    IconBadge(IconBadgeSpec),
    /// バートレンドパターン。
    BarTrend(BarTrendSpec),
    /// エラー影鿹パターン。
    ErrorFallback(ErrorFallbackSpec),
}

/// LCD ストリップボタンのセクション描画向けのサンプル仕様。
/// `catalog::resolve_section_sample()` が返す型であり、`section_patterns` が受け取る。
/// 内部バリアントは `ButtonSampleSpec` と共通だが、番号体系が異なるため別型として定義する。
#[derive(Debug, Clone)]
pub enum SectionSampleSpec {
    /// 単一ラベル + 背景色パターン。
    LabelOnly(LabelOnlySpec),
    /// タイトル + 値 + 単位パターン。
    LabelValue(LabelValueSpec),
    /// アイコン + バッジパターン。
    IconBadge(IconBadgeSpec),
    /// バートレンドパターン。
    BarTrend(BarTrendSpec),
    /// エラー影鿹パターン。
    ErrorFallback(ErrorFallbackSpec),
}

impl From<ButtonSampleSpec> for SectionSampleSpec {
    /// `ButtonSampleSpec` を内布データをそのまま継承して `SectionSampleSpec` に変換する。
    /// `resolve_section_sample` 克 内部で共通ビルド樔路を再利用するために導入した。
    fn from(s: ButtonSampleSpec) -> Self {
        match s {
            ButtonSampleSpec::LabelOnly(x) => SectionSampleSpec::LabelOnly(x),
            ButtonSampleSpec::LabelValue(x) => SectionSampleSpec::LabelValue(x),
            ButtonSampleSpec::IconBadge(x) => SectionSampleSpec::IconBadge(x),
            ButtonSampleSpec::BarTrend(x) => SectionSampleSpec::BarTrend(x),
            ButtonSampleSpec::ErrorFallback(x) => SectionSampleSpec::ErrorFallback(x),
        }
    }
}
