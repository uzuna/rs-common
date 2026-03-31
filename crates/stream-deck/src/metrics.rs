//! MetricsProvider: CPU・メモリ・ロードアベレージをサンプリングするモジュール

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

/// CPU・メモリ・ロードアベレージを定期的にサンプリングする
pub struct MetricsProvider {
    sys: System,
    cpu_count: usize,
}

impl MetricsProvider {
    /// sysinfo を初期化し、初回サンプルを収集する
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
        Self { sys, cpu_count }
    }

    /// 全メトリクスを1回更新してスナップショットを返す
    pub fn sample(&mut self) -> MetricsSnapshot {
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

        MetricsSnapshot {
            cpu_pct,
            mem_pct,
            used_memory_bytes: used,
            total_memory_bytes: total,
            load_one,
            cpu_count: self.cpu_count,
        }
    }
}

impl Default for MetricsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl MetricsProvider {
        /// CPU 使用率のみ更新して返す（後方互換）
        pub fn cpu_usage(&mut self) -> f32 {
            self.sys.refresh_cpu_usage();
            self.sys.global_cpu_usage()
        }

        /// 各論理コアの使用率を返す (0.0 - 100.0)
        pub fn per_core_usage(&self) -> Vec<f32> {
            self.sys.cpus().iter().map(|cpu| cpu.cpu_usage()).collect()
        }
    }

    /// 値域確認: CPU使用率は 0.0〜100.0 の範囲に収まる
    #[test]
    fn test_cpu_usage_range() {
        let mut provider = MetricsProvider::new();
        let cases = vec![provider.cpu_usage(), provider.cpu_usage()];
        for usage in cases {
            assert!((0.0..=100.0).contains(&usage), "CPU使用率が範囲外: {usage}");
        }
    }

    /// 各コアの使用率も 0.0〜100.0 の範囲に収まる
    #[test]
    fn test_per_core_usage_range() {
        let provider = MetricsProvider::new();
        for (i, usage) in provider.per_core_usage().iter().enumerate() {
            assert!(
                (0.0..=100.0).contains(usage),
                "コア{i} の使用率が範囲外: {usage}"
            );
        }
    }

    /// 正常系: スナップショットの全値域が有効範囲に収まる
    #[test]
    fn test_snapshot_ranges() {
        let mut provider = MetricsProvider::new();
        let snap = provider.sample();

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
}
