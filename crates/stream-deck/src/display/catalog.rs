//! 設定上の display sample を解決するカタログ。

use crate::config::{DisplayPatternKind, DisplaySamplePayload, DisplaySampleTarget};
use crate::display::model::{ButtonSampleSpec, SectionSampleSpec};
use crate::runtime::RuntimeConfig;

pub fn resolve_button_sample(config: &RuntimeConfig, sample_id: &str) -> Option<ButtonSampleSpec> {
    let sample = config
        .display
        .samples
        .iter()
        .find(|sample| sample.id == sample_id)?;

    if sample.target != DisplaySampleTarget::Button {
        return None;
    }

    match (&sample.pattern, &sample.payload) {
        (DisplayPatternKind::LabelOnly, DisplaySamplePayload::LabelOnly(payload)) => {
            Some(ButtonSampleSpec::LabelOnly(payload.clone()))
        }
        (DisplayPatternKind::LabelValue, DisplaySamplePayload::LabelValue(payload)) => {
            Some(ButtonSampleSpec::LabelValue(payload.clone()))
        }
        (DisplayPatternKind::IconBadge, DisplaySamplePayload::IconBadge(payload)) => {
            Some(ButtonSampleSpec::IconBadge(payload.clone()))
        }
        (DisplayPatternKind::BarTrend, DisplaySamplePayload::BarTrend(payload)) => {
            Some(ButtonSampleSpec::BarTrend(payload.clone()))
        }
        (DisplayPatternKind::ErrorFallback, DisplaySamplePayload::ErrorFallback(payload)) => {
            Some(ButtonSampleSpec::ErrorFallback(payload.clone()))
        }
        _ => None,
    }
}

pub fn resolve_section_sample(
    config: &RuntimeConfig,
    sample_id: &str,
) -> Option<SectionSampleSpec> {
    let sample = config
        .display
        .samples
        .iter()
        .find(|sample| sample.id == sample_id)?;

    if sample.target != DisplaySampleTarget::Section {
        return None;
    }

    match (&sample.pattern, &sample.payload) {
        (DisplayPatternKind::LabelOnly, DisplaySamplePayload::LabelOnly(payload)) => {
            Some(SectionSampleSpec::LabelOnly(payload.clone()))
        }
        (DisplayPatternKind::LabelValue, DisplaySamplePayload::LabelValue(payload)) => {
            Some(SectionSampleSpec::LabelValue(payload.clone()))
        }
        (DisplayPatternKind::IconBadge, DisplaySamplePayload::IconBadge(payload)) => {
            Some(SectionSampleSpec::IconBadge(payload.clone()))
        }
        (DisplayPatternKind::BarTrend, DisplaySamplePayload::BarTrend(payload)) => {
            Some(SectionSampleSpec::BarTrend(payload.clone()))
        }
        (DisplayPatternKind::ErrorFallback, DisplaySamplePayload::ErrorFallback(payload)) => {
            Some(SectionSampleSpec::ErrorFallback(payload.clone()))
        }
        _ => None,
    }
}
