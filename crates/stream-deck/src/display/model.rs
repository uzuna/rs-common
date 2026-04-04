//! 表示サンプルの中間表現。

use crate::config::{
    BarTrendPayload, ErrorFallbackPayload, IconBadgePayload, LabelOnlyPayload, LabelValuePayload,
};

#[derive(Debug, Clone)]
pub enum ButtonSampleSpec {
    LabelOnly(LabelOnlyPayload),
    LabelValue(LabelValuePayload),
    IconBadge(IconBadgePayload),
    BarTrend(BarTrendPayload),
    ErrorFallback(ErrorFallbackPayload),
}
