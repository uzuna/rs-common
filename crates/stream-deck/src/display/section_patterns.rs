//! セクション表示サンプルの適用ロジック。

use crate::config::PageItemConfig;
use crate::display::catalog;
use crate::display::model::SectionSampleSpec;
use crate::renderer::SectionSpec;
use crate::runtime::RuntimeConfig;
use crate::state::{PageState, Phase0ActionRuntime};

/// `SectionSampleSpec` を `SectionSpec` に変換する。
/// `history_len` はレンダラが期待する履歴の要素数で、スパークラインの円滑な描画のためにパディングまたは山切りを行う。
fn render_section_pattern(sample: &SectionSampleSpec, history_len: usize) -> SectionSpec {
    match sample {
        SectionSampleSpec::LabelOnly(payload) => SectionSpec {
            label: payload.label.as_str().to_string(),
            value_text: String::new(),
            history: vec![0.0; history_len],
        },
        SectionSampleSpec::LabelValue(payload) => SectionSpec {
            label: payload.title.as_str().to_string(),
            value_text: format!("{}{}", payload.value.as_str(), payload.unit),
            history: vec![0.0; history_len],
        },
        SectionSampleSpec::IconBadge(payload) => SectionSpec {
            label: payload.icon.as_str().to_string(),
            value_text: format!("{}", payload.badge_count),
            history: vec![0.0; history_len],
        },
        SectionSampleSpec::BarTrend(payload) => {
            let max = payload.max.get();
            let mut history = payload
                .history
                .as_slice()
                .iter()
                .map(|v| v.get())
                .collect::<Vec<_>>();

            if history.len() > history_len {
                history = history[history.len() - history_len..].to_vec();
            }
            if history.len() < history_len {
                let mut pad = vec![0.0; history_len - history.len()];
                pad.extend(history);
                history = pad;
            }

            SectionSpec {
                label: "Trend".to_string(),
                value_text: format!("{:.0}/{:.0}", payload.current.get(), max),
                history,
            }
        }
        SectionSampleSpec::ErrorFallback(payload) => SectionSpec {
            label: payload.error_code.as_str().to_string(),
            value_text: payload.message.as_str().to_string(),
            history: vec![0.0; history_len],
        },
    }
}

/// 現在ページの `Sample` アイテムに応じて `SectionSpec` 列を上書きする。
/// ページ内の Sample 順 (上から) に `specs` の先頭から始まるインデックスと対応させる。
/// 対応するサンプルが見つからない場合はそのインデックスをスキップして次のアイテムを処理する。
pub fn apply_section_pattern_overrides(
    specs: &mut [SectionSpec],
    config: &RuntimeConfig,
    page_state: &PageState,
    phase0_actions: &Phase0ActionRuntime,
) {
    let Some(page) = config
        .pages
        .iter()
        .find(|p| p.id == page_state.current_page_id)
    else {
        return;
    };

    let mut section_idx = 0usize;
    for item in &page.items {
        if section_idx >= specs.len() {
            break;
        }
        if let PageItemConfig::Sample {
            sample_id,
            action_ref,
            ..
        } = item
        {
            if let Some(sample) = catalog::resolve_section_sample(config, sample_id) {
                let history_len = specs[section_idx].history.len().max(1);
                specs[section_idx] = render_section_pattern(&sample, history_len);
                if let Some(feedback) = action_ref
                    .as_deref()
                    .and_then(|id| phase0_actions.feedback_text(id))
                {
                    specs[section_idx].value_text = feedback;
                }
                section_idx += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::display::dto::{
        DisplayConfigDto, DisplayPayloadDto, DisplaySampleDto, DisplayTargetDto,
    };
    use crate::state::Phase0ActionRuntime;

    fn runtime_with_section_sample() -> RuntimeConfig {
        RuntimeConfig {
            sections: vec![],
            home_page_id: "home".to_string(),
            pages: vec![config::PageConfig {
                id: "home".to_string(),
                title: "Home".to_string(),
                items: vec![config::PageItemConfig::Sample {
                    sample_id: "sec_mode".to_string(),
                    label: "SecMode".to_string(),
                    action_ref: Some("act_sec_mode".to_string()),
                    priority: Some(10),
                }],
            }],
            actions: vec![config::ActionConfig::Transition {
                id: "act_sec_mode".to_string(),
                states: vec!["idle".to_string(), "watch".to_string()],
                initial_state: Some("idle".to_string()),
            }],
            display: DisplayConfigDto {
                samples: vec![DisplaySampleDto {
                    id: "sec_mode".to_string(),
                    target: DisplayTargetDto::Section,
                    payload: DisplayPayloadDto::LabelValue {
                        title: "Sec".to_string(),
                        value: "42".to_string(),
                        unit: "%".to_string(),
                        severity: config::DisplaySeverity::Normal,
                    },
                }],
            },
            watch_targets: vec![],
            podman: None,
        }
    }

    #[test]
    fn test_apply_section_pattern_overrides_prefers_action_feedback_text() {
        let config = runtime_with_section_sample();
        let page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec![],
        };
        let mut phase0_actions = Phase0ActionRuntime::from_config(&config);
        let mut specs = vec![SectionSpec {
            label: "orig".to_string(),
            value_text: "orig".to_string(),
            history: vec![0.0; 4],
        }];

        apply_section_pattern_overrides(&mut specs, &config, &page_state, &phase0_actions);
        assert_eq!(specs[0].value_text, "idle");

        let _ = phase0_actions.apply("act_sec_mode");
        apply_section_pattern_overrides(&mut specs, &config, &page_state, &phase0_actions);
        assert_eq!(specs[0].value_text, "watch");
    }
}
