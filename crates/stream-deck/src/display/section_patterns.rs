//! セクション表示サンプルの適用ロジック。

use crate::config::PageItemConfig;
use crate::display::catalog;
use crate::display::model::SectionSampleSpec;
use crate::renderer::SectionSpec;
use crate::runtime::RuntimeConfig;
use crate::state::PageState;

fn render_section_pattern(sample: &SectionSampleSpec, history_len: usize) -> SectionSpec {
    match sample {
        SectionSampleSpec::LabelOnly(payload) => SectionSpec {
            label: payload.label.clone(),
            value_text: String::new(),
            history: vec![0.0; history_len],
        },
        SectionSampleSpec::LabelValue(payload) => SectionSpec {
            label: payload.title.clone(),
            value_text: format!("{}{}", payload.value, payload.unit),
            history: vec![0.0; history_len],
        },
        SectionSampleSpec::IconBadge(payload) => SectionSpec {
            label: payload.icon.clone(),
            value_text: format!("{}", payload.badge_count),
            history: vec![0.0; history_len],
        },
        SectionSampleSpec::BarTrend(payload) => {
            let max = payload.max.max(1.0);
            let mut history = payload
                .history
                .iter()
                .map(|v| (v / max).clamp(0.0, 1.0))
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
                value_text: format!("{:.0}/{:.0}", payload.current, payload.max),
                history,
            }
        }
        SectionSampleSpec::ErrorFallback(payload) => SectionSpec {
            label: payload.error_code.clone(),
            value_text: payload.message.clone(),
            history: vec![0.0; history_len],
        },
    }
}

pub fn apply_section_pattern_overrides(
    specs: &mut [SectionSpec],
    config: &RuntimeConfig,
    page_state: &PageState,
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
        if let PageItemConfig::Sample { sample_id, .. } = item {
            if let Some(sample) = catalog::resolve_section_sample(config, sample_id) {
                let history_len = specs[section_idx].history.len().max(1);
                specs[section_idx] = render_section_pattern(&sample, history_len);
                section_idx += 1;
            }
        }
    }
}
