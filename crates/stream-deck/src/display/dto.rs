//! 表示設定のDTO。
//! config モジュールの serde 構造体から、表示ドメイン専用の中間表現へ変換する。
//! この層は「構文的に正しい config 値」を「表示ドメインへ渡せる形」に整形する境界であり、
//! 値域の厳密検証は次段の NewType 変換層に委譲する。

use std::fmt;

use crate::config::{self, DisplayPatternKind};

/// `display` セクション全体を保持するトップレベル DTO。
/// `config::DisplayConfig` から変換され、`RuntimeConfig.display` として保持される。
#[derive(Debug, Clone, Default)]
pub struct DisplayConfigDto {
    /// 表示サンプル定義の一覧。
    pub samples: Vec<DisplaySampleDto>,
}

/// 1 つのサンプル定義を保持する DTO。
/// `config::DisplaySampleConfig` 1 対 1 で変換される。
#[derive(Debug, Clone)]
pub struct DisplaySampleDto {
    /// サンプルID。空禁止などの厳密検証は NewType 変換層で行う。
    pub id: String,
    /// ボタン/セクションのどちらに適用されるか。
    pub target: DisplayTargetDto,
    /// パターンごとのペイロード本体。
    pub payload: DisplayPayloadDto,
}

/// サンプルの適用先を示す列挙型。
/// TOML の `target = "button"` / `target = "section"` に対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayTargetDto {
    /// ボタン描画向けサンプル。
    Button,
    /// セクション描画向けサンプル。
    Section,
}

/// パターンごとのペイロードを保持する列挙型。
/// `config::DisplaySamplePayload` から変換され、`catalog` 層へ渡る中間表現となる。
/// 値域の密な検証は次段 NewType 層に委譲する。
#[derive(Debug, Clone)]
pub enum DisplayPayloadDto {
    /// 単一ラベル + 背景色。
    LabelOnly { label: String, bg_color: [u8; 3] },
    /// タイトル + 値 + 単位 + 重要度。
    LabelValue {
        title: String,
        value: String,
        unit: String,
        severity: config::DisplaySeverity,
    },
    /// アイコン + 件数バッジ。
    IconBadge {
        icon: String,
        badge_count: u32,
        status: config::DisplaySeverity,
    },
    /// 現在値 + 最大値 + 履歴列。
    BarTrend {
        current: f32,
        max: f32,
        history: Vec<f32>,
    },
    /// エラー表示用のコード + メッセージ。
    ErrorFallback { error_code: String, message: String },
}

/// DTO 変換時に発生するエラー。
#[derive(Debug, Clone)]
pub enum DisplayDtoError {
    /// `pattern` と `payload` の組が一致しない。
    PatternPayloadMismatch,
}

impl fmt::Display for DisplayDtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisplayDtoError::PatternPayloadMismatch => {
                write!(f, "pattern と payload の組み合わせが一致しません")
            }
        }
    }
}

impl std::error::Error for DisplayDtoError {}

impl TryFrom<&config::DisplayConfig> for DisplayConfigDto {
    type Error = DisplayDtoError;

    /// `DisplayConfig` 全体を DTO 化する。
    /// サンプル単位の変換失敗があればそこで打ち切ってエラーを返す。
    fn try_from(value: &config::DisplayConfig) -> Result<Self, Self::Error> {
        let mut samples = Vec::with_capacity(value.samples.len());
        for sample in &value.samples {
            samples.push(DisplaySampleDto::try_from(sample)?);
        }
        Ok(Self { samples })
    }
}

impl TryFrom<&config::DisplaySampleConfig> for DisplaySampleDto {
    type Error = DisplayDtoError;

    /// 1 サンプル分を DTO 化する。
    /// `pattern` と `payload` のバリアントが一致しない場合は `PatternPayloadMismatch` を返す。
    fn try_from(value: &config::DisplaySampleConfig) -> Result<Self, Self::Error> {
        let target = match value.target {
            config::DisplaySampleTarget::Button => DisplayTargetDto::Button,
            config::DisplaySampleTarget::Section => DisplayTargetDto::Section,
        };

        let payload = match (&value.pattern, &value.payload) {
            (DisplayPatternKind::LabelOnly, config::DisplaySamplePayload::LabelOnly(payload)) => {
                DisplayPayloadDto::LabelOnly {
                    label: payload.label.clone(),
                    bg_color: payload.bg_color,
                }
            }
            (DisplayPatternKind::LabelValue, config::DisplaySamplePayload::LabelValue(payload)) => {
                DisplayPayloadDto::LabelValue {
                    title: payload.title.clone(),
                    value: payload.value.clone(),
                    unit: payload.unit.clone(),
                    severity: payload.severity,
                }
            }
            (DisplayPatternKind::IconBadge, config::DisplaySamplePayload::IconBadge(payload)) => {
                DisplayPayloadDto::IconBadge {
                    icon: payload.icon.clone(),
                    badge_count: payload.badge_count,
                    status: payload.status,
                }
            }
            (DisplayPatternKind::BarTrend, config::DisplaySamplePayload::BarTrend(payload)) => {
                DisplayPayloadDto::BarTrend {
                    current: payload.current,
                    max: payload.max,
                    history: payload.history.clone(),
                }
            }
            (
                DisplayPatternKind::ErrorFallback,
                config::DisplaySamplePayload::ErrorFallback(payload),
            ) => DisplayPayloadDto::ErrorFallback {
                error_code: payload.error_code.clone(),
                message: payload.message.clone(),
            },
            _ => return Err(DisplayDtoError::PatternPayloadMismatch),
        };

        Ok(Self {
            id: value.id.clone(),
            target,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_sample(
        pattern: DisplayPatternKind,
        payload: config::DisplaySamplePayload,
    ) -> config::DisplaySampleConfig {
        config::DisplaySampleConfig {
            id: "sample-1".to_string(),
            target: config::DisplaySampleTarget::Button,
            pattern,
            payload,
        }
    }

    /// 値域確認: bar_trend の代表値がDTOへ保持されること
    #[test]
    fn test_display_sample_dto_range_cases() {
        let cases = [
            (0.0_f32, 1.0_f32, vec![0.0_f32, 1.0_f32]),
            (50.0_f32, 100.0_f32, vec![10.0_f32, 50.0_f32, 90.0_f32]),
            (100.0_f32, 100.0_f32, vec![0.0_f32, 100.0_f32]),
        ];

        for (current, max, history) in cases {
            let sample = mk_sample(
                DisplayPatternKind::BarTrend,
                config::DisplaySamplePayload::BarTrend(config::BarTrendPayload {
                    current,
                    max,
                    history: history.clone(),
                }),
            );

            let dto = DisplaySampleDto::try_from(&sample).expect("dto conversion");
            let DisplayPayloadDto::BarTrend {
                current: got_current,
                max: got_max,
                history: got_history,
            } = dto.payload
            else {
                panic!("bar_trend payload expected");
            };
            assert!((got_current - current).abs() < 1e-6);
            assert!((got_max - max).abs() < 1e-6);
            assert_eq!(got_history, history);
        }
    }

    /// 正常系: pattern と payload が一致する全パターンが変換可能
    #[test]
    fn test_display_sample_dto_normal_cases() {
        let cases = vec![
            mk_sample(
                DisplayPatternKind::LabelOnly,
                config::DisplaySamplePayload::LabelOnly(config::LabelOnlyPayload {
                    label: "Apps".to_string(),
                    bg_color: [1, 2, 3],
                }),
            ),
            mk_sample(
                DisplayPatternKind::LabelValue,
                config::DisplaySamplePayload::LabelValue(config::LabelValuePayload {
                    title: "CPU".to_string(),
                    value: "42".to_string(),
                    unit: "%".to_string(),
                    severity: config::DisplaySeverity::Normal,
                }),
            ),
            mk_sample(
                DisplayPatternKind::IconBadge,
                config::DisplaySamplePayload::IconBadge(config::IconBadgePayload {
                    icon: "bell".to_string(),
                    badge_count: 7,
                    status: config::DisplaySeverity::Warn,
                }),
            ),
            mk_sample(
                DisplayPatternKind::BarTrend,
                config::DisplaySamplePayload::BarTrend(config::BarTrendPayload {
                    current: 10.0,
                    max: 100.0,
                    history: vec![1.0, 2.0, 3.0],
                }),
            ),
            mk_sample(
                DisplayPatternKind::ErrorFallback,
                config::DisplaySamplePayload::ErrorFallback(config::ErrorFallbackPayload {
                    error_code: "E_CONN".to_string(),
                    message: "offline".to_string(),
                }),
            ),
        ];

        for sample in &cases {
            let got = DisplaySampleDto::try_from(sample);
            assert!(got.is_ok(), "id={} should succeed", sample.id);
        }

        let cfg = config::DisplayConfig { samples: cases };
        let cfg_dto = DisplayConfigDto::try_from(&cfg);
        assert!(cfg_dto.is_ok());
    }

    /// 異常系: pattern と payload が不一致な場合は変換エラー
    #[test]
    fn test_display_sample_dto_error_cases() {
        let cases = vec![
            mk_sample(
                DisplayPatternKind::LabelOnly,
                config::DisplaySamplePayload::LabelValue(config::LabelValuePayload {
                    title: "CPU".to_string(),
                    value: "42".to_string(),
                    unit: "%".to_string(),
                    severity: config::DisplaySeverity::Normal,
                }),
            ),
            mk_sample(
                DisplayPatternKind::BarTrend,
                config::DisplaySamplePayload::ErrorFallback(config::ErrorFallbackPayload {
                    error_code: "E_CONN".to_string(),
                    message: "offline".to_string(),
                }),
            ),
            mk_sample(
                DisplayPatternKind::IconBadge,
                config::DisplaySamplePayload::LabelOnly(config::LabelOnlyPayload {
                    label: "Apps".to_string(),
                    bg_color: [1, 2, 3],
                }),
            ),
        ];

        for sample in cases {
            let got = DisplaySampleDto::try_from(&sample);
            assert!(matches!(got, Err(DisplayDtoError::PatternPayloadMismatch)));
        }
    }
}
