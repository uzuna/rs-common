//! Plugin 共通契約モジュール。
//!
//! Monitor/View/Action 各レイヤで共通して使う型を定義する。
//! 具体的な実装は各モジュールへ委譲し、ここでは型・トレイト境界のみを提供する。

use std::time::Instant;

// ── MetricKey ────────────────────────────────────────────────────

/// メトリクスを識別するキー。
///
/// ドット区切りの階層形式で表現する。
/// 例: `system.cpu.usage`, `system.mem.usage`, `podman.container.<id>.cpu`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetricKey(String);

impl MetricKey {
    /// MetricKey を生成する（空文字列は不可）。
    pub fn new(key: impl Into<String>) -> Result<Self, MetricKeyError> {
        let key = key.into();
        if key.is_empty() {
            return Err(MetricKeyError::Empty);
        }
        Ok(Self(key))
    }

    /// 内部文字列への参照を返す。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MetricKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// [`MetricKey`] 生成失敗の理由。
#[derive(Debug, PartialEq, Eq)]
pub enum MetricKeyError {
    /// 空文字列は MetricKey として不正。
    Empty,
}

impl std::fmt::Display for MetricKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "MetricKey は空文字列にできません"),
        }
    }
}

/// system.cpu.usage — CPU 使用率 (0.0..=100.0)
pub const KEY_SYSTEM_CPU_USAGE: &str = "system.cpu.usage";
/// system.mem.usage — メモリ使用率 (0.0..=100.0)
pub const KEY_SYSTEM_MEM_USAGE: &str = "system.mem.usage";
/// system.load.one — 1分間ロードアベレージ (0.0..)
pub const KEY_SYSTEM_LOAD_ONE: &str = "system.load.one";
/// system.brightness — 輝度 (0.0..=100.0)
pub const KEY_SYSTEM_BRIGHTNESS: &str = "system.brightness";
/// system.notif.count — 未読通知件数
pub const KEY_SYSTEM_NOTIF_COUNT: &str = "system.notif.count";

// ── MetricValue / MetricEvent ──────────────────────────────────

/// メトリクスの値型。
#[derive(Debug, Clone)]
pub enum MetricValue {
    /// ゲージ値（瞬間値、例: CPU使用率）
    Gauge(f32),
    /// カウンタ値（単調増加）
    Counter(u64),
}

impl MetricValue {
    /// 正規化済み値 (0.0..=1.0) を返す。
    ///
    /// `max` はGauge値を正規化するための最大値。
    /// Counter は 0.0 を返す（正規化には別途変換が必要）。
    pub fn as_gauge(&self) -> Option<f32> {
        match self {
            Self::Gauge(v) => Some(*v),
            Self::Counter(_) => None,
        }
    }
}

/// メトリクスの1イベント。
#[derive(Debug, Clone)]
pub struct MetricEvent {
    /// メトリクスキー
    pub key: MetricKey,
    /// 値
    pub value: MetricValue,
    /// 記録時刻
    pub timestamp: Instant,
}

impl MetricEvent {
    /// `Gauge` 値のイベントをタイムスタンプ付きで生成する。
    pub fn gauge(key: MetricKey, value: f32) -> Self {
        Self {
            key,
            value: MetricValue::Gauge(value),
            timestamp: Instant::now(),
        }
    }
}

// ── Capability / PluginManifest ──────────────────────────────

/// プラグインが持つ機能種別。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// 外部データ収集
    Monitor,
    /// 表示ビュー生成
    View,
    /// 外部操作実行
    Action,
}

/// プラグインの自己記述。
///
/// 各プラグインは `PluginManifest` を返し、ホスト側が機能の有無を判断する。
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// プラグイン識別子（例: "system-metrics", "podman"）
    pub id: String,
    /// 利用可能な機能一覧
    pub capabilities: Vec<Capability>,
}

impl PluginManifest {
    /// Monitor + View capability を持つマニフェストを生成する。
    pub fn monitor_view(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            capabilities: vec![Capability::Monitor, Capability::View],
        }
    }

    /// 指定 capability を持つかを返す。
    pub fn has(&self, cap: &Capability) -> bool {
        self.capabilities.contains(cap)
    }
}

// ── テスト ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 値域確認: 空文字列は MetricKey 生成に失敗する
    #[test]
    fn test_metric_key_rejects_empty() {
        let cases = [""];
        for key in cases {
            assert_eq!(
                MetricKey::new(key),
                Err(MetricKeyError::Empty),
                "key={key:?}"
            );
        }
    }

    /// 正常系: 非空文字列は MetricKey 生成に成功する
    #[test]
    fn test_metric_key_accepts_valid() {
        let cases = [
            KEY_SYSTEM_CPU_USAGE,
            KEY_SYSTEM_MEM_USAGE,
            KEY_SYSTEM_LOAD_ONE,
            "podman.container.abc123.cpu",
        ];
        for key in cases {
            let mk = MetricKey::new(key).unwrap();
            assert_eq!(mk.as_str(), key);
        }
    }

    /// 正常系: PluginManifest::has は宣言済み capability のみ true
    #[test]
    fn test_plugin_manifest_has_capability() {
        let manifest = PluginManifest::monitor_view("system-metrics");
        assert!(manifest.has(&Capability::Monitor));
        assert!(manifest.has(&Capability::View));
        assert!(!manifest.has(&Capability::Action));
    }

    /// 正常系: MetricEvent::gauge タイムスタンプが生成直後以降
    #[test]
    fn test_metric_event_gauge_timestamp() {
        let before = Instant::now();
        let key = MetricKey::new(KEY_SYSTEM_CPU_USAGE).unwrap();
        let event = MetricEvent::gauge(key.clone(), 50.0);
        assert!(event.timestamp >= before);
        assert_eq!(event.key, key);
        assert!(matches!(event.value, MetricValue::Gauge(v) if (v - 50.0).abs() < 1e-5));
    }
}
