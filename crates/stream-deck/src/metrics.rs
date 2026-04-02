//! MetricsProvider: CPU・メモリ・ロードアベレージをサンプリングするモジュール
//! MetricsSource: CPU・メモリ・ロードアベレージをサンプリングするモジュール

use std::time::Duration;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// CPU 使用率の初回サンプル取得に必要な待機時間
const INITIAL_WAIT: Duration = Duration::from_millis(200);

/// メトリクスの1回分スナップショット
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    /// CPU 使用率 (0.0 - 100.0)
    pub cpu_pct: f32,
    /// メモリ使用率 (0.0 - 100.0)
    pub mem_pct: f32,
    /// 使用中メモリ (bytes)
    pub used_memory_bytes: u64,
    /// 物理メモリ合計 (bytes)
    pub total_memory_bytes: u64,
    /// 1分間ロードアベレージ (0.0 - cpu_count)
    pub load_one: f32,
    /// 論理 CPU コア数 (ロードアベレージの正規化基準)
    pub cpu_count: usize,
}

/// CPU・メモリ・ロードアベレージを共有データソースとして管理する
///
/// 毎ティック冒頭で [`MetricsSource::refresh`] を一度だけ呼び、
/// 各プラグインは [`MetricsSource::snapshot`] でキャッシュを読む。
pub struct MetricsSource {
    sys: System,
    cpu_count: usize,
    snapshot: Option<MetricsSnapshot>,
}

impl MetricsSource {
    /// sysinfo を初期化し、初回スナップショットを取得する
    ///
    /// 初回の `refresh_cpu_usage` は常に 0.0 を返すため、
    /// 短い待機後に再度収集して有効な初期値を得る。
    pub fn new() -> Self {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        // CPU: 初回は差分がないため 0.0 → 一定時間後に再収集
        sys.refresh_cpu_usage();
        std::thread::sleep(INITIAL_WAIT);
        sys.refresh_cpu_usage();
        sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());

        let cpu_count = sys.cpus().len().max(1);
        Self {
            sys,
            cpu_count,
            snapshot: None,
        }
    }

    /// 全メトリクスを1回更新してスナップショットをキャッシュする
    pub fn refresh(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());

        let cpu_pct = self.sys.global_cpu_usage();
        let total = self.sys.total_memory();
        let used = self.sys.used_memory();
        let mem_pct = if total > 0 {
            (used as f64 / total as f64 * 100.0) as f32
        } else {
            0.0
        };
        let load_one = System::load_average().one as f32;

        self.snapshot = Some(MetricsSnapshot {
            cpu_pct,
            mem_pct,
            used_memory_bytes: used,
            total_memory_bytes: total,
            load_one,
            cpu_count: self.cpu_count,
        });
    }

    /// キャッシュ済みスナップショットへの参照を返す
    pub fn snapshot(&self) -> Option<&MetricsSnapshot> {
        self.snapshot.as_ref()
    }

    /// 論理 CPU コア数
    pub fn cpu_count(&self) -> usize {
        self.cpu_count
    }
}

impl Default for MetricsSource {
    fn default() -> Self {
        Self::new()
    }
}

/// メモリ使用量を "使用量/最大値" 形式の文字列に変換する (例: "6.2/16G")
pub fn format_memory(used: u64, total: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    if total >= GB {
        let used_gb = used as f64 / GB as f64;
        let total_gb = total as f64 / GB as f64;
        format!("{used_gb:.1}/{total_gb:.0}G")
    } else {
        let used_mb = used / MB;
        let total_mb = total / MB;
        format!("{used_mb}/{total_mb}M")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl MetricsSource {
        /// CPU 使用率のみ更新して返す（テスト用）
        pub fn cpu_usage(&mut self) -> f32 {
            self.sys.refresh_cpu_usage();
            self.sys.global_cpu_usage()
        }

        /// 各論理コアの使用率を返す (0.0 - 100.0)（テスト用）
        pub fn per_core_usage(&self) -> Vec<f32> {
            self.sys.cpus().iter().map(|cpu| cpu.cpu_usage()).collect()
        }
    }

    /// 値域確認: CPU使用率は 0.0〜100.0 の範囲に収まる
    #[test]
    fn test_cpu_usage_range() {
        let mut source = MetricsSource::new();
        let cases = vec![source.cpu_usage(), source.cpu_usage()];
        for usage in cases {
            assert!((0.0..=100.0).contains(&usage), "CPU使用率が範囲外: {usage}");
        }
    }

    /// 各コアの使用率も 0.0〜100.0 の範囲に収まる
    #[test]
    fn test_per_core_usage_range() {
        let source = MetricsSource::new();
        for (i, usage) in source.per_core_usage().iter().enumerate() {
            assert!(
                (0.0..=100.0).contains(usage),
                "コア{i} の使用率が範囲外: {usage}"
            );
        }
    }

    /// 正常系: スナップショットの全値域が有効範囲に収まる
    #[test]
    fn test_snapshot_ranges() {
        let mut source = MetricsSource::new();
        source.refresh();
        let snap = source.snapshot().expect("スナップショットが None");

        assert!(
            (0.0..=100.0).contains(&snap.cpu_pct),
            "cpu_pct が範囲外: {}",
            snap.cpu_pct
        );
        assert!(
            (0.0..=100.0).contains(&snap.mem_pct),
            "mem_pct が範囲外: {}",
            snap.mem_pct
        );
        assert!(snap.load_one >= 0.0, "load_one が負: {}", snap.load_one);
        assert!(snap.cpu_count >= 1, "cpu_count が 0: {}", snap.cpu_count);
    }

    #[test]
    fn test_format_memory() {
        const GB: u64 = 1 << 30;
        const MB: u64 = 1 << 20;
        let cases: &[(u64, u64, &str)] = &[
            (6 * GB + GB / 5, 16 * GB, "6.2/16G"),
            (0, 16 * GB, "0.0/16G"),
            (16 * GB, 16 * GB, "16.0/16G"),
            (256 * MB, 512 * MB, "256/512M"),
        ];
        for &(used, total, expected) in cases {
            let got = format_memory(used, total);
            assert_eq!(got, expected, "used={used} total={total}");
        }
    }
}
