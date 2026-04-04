//! 設定上の display sample を解決するカタログ。

use tracing::warn;

use crate::config::DisplaySeverity;
use crate::display::dto::{DisplayPayloadDto, DisplayTargetDto};
use crate::display::model::{
    BarTrendSpec, ButtonSampleSpec, ErrorFallbackSpec, IconBadgeSpec, LabelOnlySpec,
    LabelValueSpec, SectionSampleSpec,
};
use crate::display::types::{
    DisplayTypeError, HistoryPoints, NonEmptyText, NonNegativeF32, Normalized, Percent,
    PositiveF32, RgbColor, SampleId,
};
use crate::runtime::RuntimeConfig;

/// DTO フィールドから `LabelOnlySpec` を組み立てる。
/// 空白のみのラベルは `NonEmptyText` の生成時点で拒否される。
fn build_label_only(label: String, bg_color: [u8; 3]) -> Result<LabelOnlySpec, DisplayTypeError> {
    Ok(LabelOnlySpec {
        label: NonEmptyText::try_from(label)?,
        bg_color: RgbColor::from(bg_color),
    })
}

/// DTO フィールドから `LabelValueSpec` を組み立てる。
/// title と value は空白禁止。unit は空文字も許容する。
fn build_label_value(
    title: String,
    value: String,
    unit: String,
    severity: DisplaySeverity,
) -> Result<LabelValueSpec, DisplayTypeError> {
    Ok(LabelValueSpec {
        title: NonEmptyText::try_from(title)?,
        value: NonEmptyText::try_from(value)?,
        unit,
        severity,
    })
}

/// DTO フィールドから `IconBadgeSpec` を組み立てる。
/// icon テキストは空白禁止。badge_count はゼロ許容。
fn build_icon_badge(
    icon: String,
    badge_count: u32,
    status: DisplaySeverity,
) -> Result<IconBadgeSpec, DisplayTypeError> {
    Ok(IconBadgeSpec {
        icon: NonEmptyText::try_from(icon)?,
        badge_count,
        status,
    })
}

/// DTO フィールドから `BarTrendSpec` を組み立てる。
/// current >= 0、max > 0、history は空禁止・要素は max 以下の値域制約を課す。
/// 各制約は NewType（`NonNegativeF32`, `PositiveF32`, `Normalized`）が保証する。
fn build_bar_trend(
    current: f32,
    max: f32,
    history: Vec<f32>,
) -> Result<BarTrendSpec, DisplayTypeError> {
    let current = NonNegativeF32::try_from(current)?;
    let max = PositiveF32::try_from(max)?;
    let percent = Percent::from_ratio(current, max)?;

    let normalized = history
        .iter()
        .map(|v| {
            let point = NonNegativeF32::try_from(*v)?;
            if point.get() > max.get() {
                return Err(DisplayTypeError::OutOfRange);
            }
            Normalized::try_from(point.get() / max.get())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let history = HistoryPoints::from_normalized(normalized)?;

    Ok(BarTrendSpec {
        current,
        max,
        percent,
        history,
    })
}

/// DTO フィールドから `ErrorFallbackSpec` を組み立てる。
/// error_code と message はどちらも空白禁止。
fn build_error_fallback(
    error_code: String,
    message: String,
) -> Result<ErrorFallbackSpec, DisplayTypeError> {
    Ok(ErrorFallbackSpec {
        error_code: NonEmptyText::try_from(error_code)?,
        message: NonEmptyText::try_from(message)?,
    })
}

/// DTO ペイロードから `ButtonSampleSpec` を組み立てる共通関数。
/// `resolve_button_sample` / `resolve_section_sample` の両経路で再利用する。
pub fn build_spec_from_payload(
    payload: &DisplayPayloadDto,
) -> Result<ButtonSampleSpec, DisplayTypeError> {
    match payload {
        DisplayPayloadDto::LabelOnly { label, bg_color } => {
            build_label_only(label.clone(), *bg_color).map(ButtonSampleSpec::LabelOnly)
        }
        DisplayPayloadDto::LabelValue {
            title,
            value,
            unit,
            severity,
        } => build_label_value(title.clone(), value.clone(), unit.clone(), *severity)
            .map(ButtonSampleSpec::LabelValue),
        DisplayPayloadDto::IconBadge {
            icon,
            badge_count,
            status,
        } => build_icon_badge(icon.clone(), *badge_count, *status).map(ButtonSampleSpec::IconBadge),
        DisplayPayloadDto::BarTrend {
            current,
            max,
            history,
        } => build_bar_trend(*current, *max, history.clone()).map(ButtonSampleSpec::BarTrend),
        DisplayPayloadDto::ErrorFallback {
            error_code,
            message,
        } => build_error_fallback(error_code.clone(), message.clone())
            .map(ButtonSampleSpec::ErrorFallback),
    }
}

/// sample_id でサンプルを特定し、target を確認した後に spec を構築する内部ヘルパー。
fn resolve_sample_inner(
    config: &RuntimeConfig,
    sample_id: &str,
    expected_target: DisplayTargetDto,
    warn_tag: &str,
) -> Option<ButtonSampleSpec> {
    let sample_id = SampleId::try_from(sample_id.to_string()).ok()?;
    let sample = config
        .display
        .samples
        .iter()
        .find(|s| s.id == sample_id.as_str())?;

    if sample.target != expected_target {
        return None;
    }

    match build_spec_from_payload(&sample.payload) {
        Ok(spec) => Some(spec),
        Err(e) => {
            warn!(sample_id = %sample_id.as_str(), "{warn_tag} の変換に失敗: {e}");
            None
        }
    }
}

/// `display.samples[*].id` に対応するボタン向けサンプルを解決して返す。
/// 該当 ID が存在しない・target が `Button` でない・値域制約違反の場合は `None` を返す。
/// 値域制約違反時は `warn!` ログを出力する。
pub fn resolve_button_sample(config: &RuntimeConfig, sample_id: &str) -> Option<ButtonSampleSpec> {
    resolve_sample_inner(
        config,
        sample_id,
        DisplayTargetDto::Button,
        "display button sample",
    )
}

/// `display.samples[*].id` に対応するセクション向けサンプルを解決して返す。
/// 該当 ID が存在しない・target が `Section` でない・値域制約違反の場合は `None` を返す。
/// 値域制約違反時は `warn!` ログを出力する。
pub fn resolve_section_sample(
    config: &RuntimeConfig,
    sample_id: &str,
) -> Option<SectionSampleSpec> {
    resolve_sample_inner(
        config,
        sample_id,
        DisplayTargetDto::Section,
        "display section sample",
    )
    .map(SectionSampleSpec::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        BarTrendPayload, DisplayConfig, DisplayPatternKind, DisplaySampleConfig,
        DisplaySamplePayload, DisplaySampleTarget, ErrorFallbackPayload, IconBadgePayload,
        LabelOnlyPayload, LabelValuePayload,
    };

    fn runtime_with_samples(samples: Vec<DisplaySampleConfig>) -> RuntimeConfig {
        let display = crate::display::dto::DisplayConfigDto::try_from(&DisplayConfig { samples })
            .expect("display dto conversion");
        RuntimeConfig {
            sections: vec![],
            home_page_id: "home".to_string(),
            pages: vec![],
            display,
            watch_targets: vec![],
            podman: None,
        }
    }

    /// 値域確認: bar_trend の境界値が正しく解決される
    #[test]
    fn test_resolve_button_sample_range_cases() {
        let cases: Vec<(f32, f32, Vec<f32>, u8)> = vec![
            (0.0, 100.0, vec![0.0, 10.0, 20.0], 0),
            (100.0, 100.0, vec![0.0, 50.0, 100.0], 100),
            (50.0, 200.0, vec![10.0, 20.0, 30.0], 25),
        ];

        for (idx, (current, max, history, expected_pct)) in cases.into_iter().enumerate() {
            let cfg = runtime_with_samples(vec![DisplaySampleConfig {
                id: format!("s{idx}"),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::BarTrend,
                payload: DisplaySamplePayload::BarTrend(BarTrendPayload {
                    current,
                    max,
                    history,
                }),
            }]);

            let spec = resolve_button_sample(&cfg, &format!("s{idx}"));
            let Some(ButtonSampleSpec::BarTrend(spec)) = spec else {
                panic!("bar_trend が解決されませんでした idx={idx}");
            };
            assert_eq!(spec.percent.get(), expected_pct, "idx={idx}");
        }
    }

    /// 正常系: 各パターンのサンプルが解決できる
    #[test]
    fn test_resolve_button_sample_normal_cases() {
        let cases = vec![
            DisplaySampleConfig {
                id: "label_only".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::LabelOnly,
                payload: DisplaySamplePayload::LabelOnly(LabelOnlyPayload {
                    label: "Apps".to_string(),
                    bg_color: [10, 20, 30],
                }),
            },
            DisplaySampleConfig {
                id: "label_value".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::LabelValue,
                payload: DisplaySamplePayload::LabelValue(LabelValuePayload {
                    title: "CPU".to_string(),
                    value: "42".to_string(),
                    unit: "%".to_string(),
                    severity: crate::config::DisplaySeverity::Normal,
                }),
            },
            DisplaySampleConfig {
                id: "icon_badge".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::IconBadge,
                payload: DisplaySamplePayload::IconBadge(IconBadgePayload {
                    icon: "bell".to_string(),
                    badge_count: 2,
                    status: crate::config::DisplaySeverity::Warn,
                }),
            },
            DisplaySampleConfig {
                id: "bar_trend".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::BarTrend,
                payload: DisplaySamplePayload::BarTrend(BarTrendPayload {
                    current: 62.0,
                    max: 100.0,
                    history: vec![30.0, 40.0, 62.0],
                }),
            },
            DisplaySampleConfig {
                id: "error".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::ErrorFallback,
                payload: DisplaySamplePayload::ErrorFallback(ErrorFallbackPayload {
                    error_code: "E_CONN".to_string(),
                    message: "offline".to_string(),
                }),
            },
        ];

        let cfg = runtime_with_samples(cases.clone());
        for sample in cases {
            let resolved = resolve_button_sample(&cfg, &sample.id);
            assert!(resolved.is_some(), "id={} が解決されません", sample.id);
        }
    }

    /// 異常系: 型制約違反は境界で解決失敗になる
    #[test]
    fn test_resolve_button_sample_error_cases() {
        let cases = vec![
            DisplaySampleConfig {
                id: "empty_label".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::LabelOnly,
                payload: DisplaySamplePayload::LabelOnly(LabelOnlyPayload {
                    label: "   ".to_string(),
                    bg_color: [10, 20, 30],
                }),
            },
            DisplaySampleConfig {
                id: "negative_current".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::BarTrend,
                payload: DisplaySamplePayload::BarTrend(BarTrendPayload {
                    current: -1.0,
                    max: 100.0,
                    history: vec![10.0, 20.0],
                }),
            },
            DisplaySampleConfig {
                id: "max_zero".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::BarTrend,
                payload: DisplaySamplePayload::BarTrend(BarTrendPayload {
                    current: 10.0,
                    max: 0.0,
                    history: vec![10.0, 20.0],
                }),
            },
            DisplaySampleConfig {
                id: "history_empty".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::BarTrend,
                payload: DisplaySamplePayload::BarTrend(BarTrendPayload {
                    current: 10.0,
                    max: 100.0,
                    history: vec![],
                }),
            },
            DisplaySampleConfig {
                id: "history_over_max".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::BarTrend,
                payload: DisplaySamplePayload::BarTrend(BarTrendPayload {
                    current: 10.0,
                    max: 100.0,
                    history: vec![120.0],
                }),
            },
        ];

        let cfg = runtime_with_samples(cases.clone());
        for sample in cases {
            let resolved = resolve_button_sample(&cfg, &sample.id);
            assert!(resolved.is_none(), "id={} は失敗すべきです", sample.id);
        }

        // SampleId の制約 (空文字) も解決境界で拒否される。
        assert!(resolve_button_sample(&cfg, "").is_none());
    }

    /// 正常系シナリオ: config -> DTO -> RuntimeConfig -> resolve を通して
    /// button/section の両サンプルが解決できる。
    #[test]
    fn test_scenario_resolve_button_and_section_samples_end_to_end() {
        let samples = vec![
            DisplaySampleConfig {
                id: "btn_scenario".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::LabelValue,
                payload: DisplaySamplePayload::LabelValue(LabelValuePayload {
                    title: "CPU".to_string(),
                    value: "37".to_string(),
                    unit: "%".to_string(),
                    severity: crate::config::DisplaySeverity::Warn,
                }),
            },
            DisplaySampleConfig {
                id: "sec_scenario".to_string(),
                target: DisplaySampleTarget::Section,
                pattern: DisplayPatternKind::BarTrend,
                payload: DisplaySamplePayload::BarTrend(BarTrendPayload {
                    current: 72.0,
                    max: 100.0,
                    history: vec![20.0, 35.0, 48.0, 72.0],
                }),
            },
        ];
        let cfg = runtime_with_samples(samples);

        let btn = resolve_button_sample(&cfg, "btn_scenario");
        let sec = resolve_section_sample(&cfg, "sec_scenario");

        let Some(ButtonSampleSpec::LabelValue(btn_spec)) = btn else {
            panic!("button sample が解決されませんでした");
        };
        assert_eq!(btn_spec.title.as_str(), "CPU");
        assert_eq!(btn_spec.value.as_str(), "37");

        let Some(SectionSampleSpec::BarTrend(sec_spec)) = sec else {
            panic!("section sample が解決されませんでした");
        };
        assert_eq!(sec_spec.percent.get(), 72);
        assert_eq!(sec_spec.history.as_slice().len(), 4);
    }

    /// 異常系シナリオ: target 不一致の参照は resolve できない。
    #[test]
    fn test_scenario_target_mismatch_returns_none() {
        let samples = vec![
            DisplaySampleConfig {
                id: "btn_only".to_string(),
                target: DisplaySampleTarget::Button,
                pattern: DisplayPatternKind::LabelOnly,
                payload: DisplaySamplePayload::LabelOnly(LabelOnlyPayload {
                    label: "Apps".to_string(),
                    bg_color: [1, 2, 3],
                }),
            },
            DisplaySampleConfig {
                id: "sec_only".to_string(),
                target: DisplaySampleTarget::Section,
                pattern: DisplayPatternKind::LabelOnly,
                payload: DisplaySamplePayload::LabelOnly(LabelOnlyPayload {
                    label: "Sec".to_string(),
                    bg_color: [4, 5, 6],
                }),
            },
        ];
        let cfg = runtime_with_samples(samples);

        // 逆方向での参照は解決できないことを確認する。
        assert!(resolve_section_sample(&cfg, "btn_only").is_none());
        assert!(resolve_button_sample(&cfg, "sec_only").is_none());
    }
}
