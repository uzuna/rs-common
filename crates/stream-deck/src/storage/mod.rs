//! Host Storage レイヤ。
//!
//! メトリクスの時系列履歴と最新値をメモリ上で保持する。
//! 設定再読込やページ遷移をまたいでも履歴が消えないよう、
//! [`MemoryStorage`] は `ConfigManager` のライフタイムより長く生存させる。

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::plugin_contract::{MetricEvent, MetricKey, MetricValue};

// ── MetricPoint ───────────────────────────────────────────────

/// 時系列データの1点。
#[derive(Debug, Clone)]
pub struct MetricPoint {
    /// 記録時刻
    pub timestamp: Instant,
    /// スカラー値（Gauge の場合のみ意味を持つ）
    pub value: f32,
}

// ── トレイト境界 ──────────────────────────────────────────────

/// Storage への書き込み境界。
pub trait StorageWriter {
    /// メトリクスイベントを追記する。
    fn append(&mut self, event: MetricEvent);
}

/// Storage からの読み出し境界。
pub trait StorageReader {
    /// 指定キーの最新値を返す。未記録なら `None`。
    fn latest(&self, key: &MetricKey) -> Option<MetricPoint>;

    /// 指定キーの直近 `capacity` 件の履歴を古い順に返す。
    ///
    /// 件数が `capacity` に満たない場合は先頭を 0.0 でパディングし、
    /// 常に `capacity` 本を返す。
    fn history(&self, key: &MetricKey, capacity: usize) -> Vec<f32>;
}

// ── RingBuffer（内部実装）─────────────────────────────────────

/// キーごとの固定長リングバッファ。
struct RingBuffer {
    data: VecDeque<MetricPoint>,
    capacity: usize,
}

impl RingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn push(&mut self, point: MetricPoint) {
        if self.data.len() == self.capacity {
            self.data.pop_front();
        }
        self.data.push_back(point);
    }

    fn latest(&self) -> Option<&MetricPoint> {
        self.data.back()
    }

    fn history_f32(&self, capacity: usize) -> Vec<f32> {
        let pad = capacity.saturating_sub(self.data.len());
        let mut out = vec![0.0f32; pad];
        // 直近 capacity 件を古い順に取り出す
        let skip = self.data.len().saturating_sub(capacity);
        out.extend(self.data.iter().skip(skip).map(|p| p.value));
        out
    }
}

// ── MemoryStorage ─────────────────────────────────────────────

/// in-memory リングバッファを使ったデフォルト実装。
///
/// キーごとに独立したリングバッファを保持する。
/// [`ensure_ring`] で容量を事前登録できる。未登録のキーには
/// `default_capacity` のリングバッファが `append` 時に自動生成される。
///
/// [`ensure_ring`]: MemoryStorage::ensure_ring
pub struct MemoryStorage {
    rings: HashMap<String, RingBuffer>,
    default_capacity: usize,
}

impl MemoryStorage {
    /// フォールバック容量でストレージを生成する。
    ///
    /// `default_capacity` は [`ensure_ring`] を呼ばずに `append` したキーに使われる。
    /// セクション単位で容量を明示したい場合は [`ensure_ring`] で事前登録すること。
    ///
    /// [`ensure_ring`]: MemoryStorage::ensure_ring
    pub fn new(default_capacity: usize) -> Self {
        Self {
            rings: HashMap::new(),
            default_capacity,
        }
    }

    fn ring_mut(&mut self, key: &str) -> &mut RingBuffer {
        let cap = self.default_capacity;
        self.rings
            .entry(key.to_owned())
            .or_insert_with(|| RingBuffer::new(cap))
    }

    fn ring(&self, key: &str) -> Option<&RingBuffer> {
        self.rings.get(key)
    }

    /// キーのリングバッファを指定容量で事前確保する。
    ///
    /// キーが未登録の場合は `capacity` でリングを生成する。
    /// 既登録の場合（設定再読込など）は既存リングを保持して履歴を維持する。
    /// 容量が変わった場合も再起動まで旧容量を維持する（in-memory のため許容）。
    pub fn ensure_ring(&mut self, key: &MetricKey, capacity: usize) {
        self.rings
            .entry(key.as_str().to_owned())
            .or_insert_with(|| RingBuffer::new(capacity));
    }
}

impl StorageWriter for MemoryStorage {
    fn append(&mut self, event: MetricEvent) {
        let value = match event.value {
            MetricValue::Gauge(v) => v,
            MetricValue::Counter(c) => c as f32,
        };
        let point = MetricPoint {
            timestamp: event.timestamp,
            value,
        };
        self.ring_mut(event.key.as_str()).push(point);
    }
}

impl StorageReader for MemoryStorage {
    fn latest(&self, key: &MetricKey) -> Option<MetricPoint> {
        self.ring(key.as_str())?.latest().cloned()
    }

    fn history(&self, key: &MetricKey, capacity: usize) -> Vec<f32> {
        match self.ring(key.as_str()) {
            Some(ring) => ring.history_f32(capacity),
            None => vec![0.0f32; capacity],
        }
    }
}

// ── テスト ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_contract::{KEY_SYSTEM_CPU_USAGE, KEY_SYSTEM_MEM_USAGE};

    fn cpu_key() -> MetricKey {
        MetricKey::new(KEY_SYSTEM_CPU_USAGE).unwrap()
    }

    fn mem_key() -> MetricKey {
        MetricKey::new(KEY_SYSTEM_MEM_USAGE).unwrap()
    }

    /// 値域確認: append 後に latest が取得できる
    #[test]
    fn test_latest_after_append() {
        let mut storage = MemoryStorage::new(10);
        let key = cpu_key();
        storage.append(MetricEvent::gauge(key.clone(), 42.5));
        let point = storage.latest(&key).unwrap();
        assert!((point.value - 42.5).abs() < 1e-5);
    }

    /// 値域確認: 未記録キーの latest は None
    #[test]
    fn test_latest_returns_none_for_unknown_key() {
        let storage = MemoryStorage::new(10);
        assert!(storage.latest(&cpu_key()).is_none());
    }

    /// 正常系: history は capacity 本のスライスを返す（パディングあり）
    #[test]
    fn test_history_pads_to_capacity() {
        let mut storage = MemoryStorage::new(10);
        let key = cpu_key();
        for v in [10.0, 20.0, 30.0] {
            storage.append(MetricEvent::gauge(key.clone(), v));
        }
        let hist = storage.history(&key, 5);
        assert_eq!(hist.len(), 5);
        // 先頭2要素はパディング
        assert_eq!(hist[0], 0.0);
        assert_eq!(hist[1], 0.0);
        // 末尾3要素は実値（古い順）
        assert!((hist[2] - 10.0).abs() < 1e-5, "hist[2]={}", hist[2]);
        assert!((hist[3] - 20.0).abs() < 1e-5, "hist[3]={}", hist[3]);
        assert!((hist[4] - 30.0).abs() < 1e-5, "hist[4]={}", hist[4]);
    }

    /// 正常系: リングバッファが capacity を超えたとき古い値が破棄される
    #[test]
    fn test_ring_drops_oldest_when_full() {
        let cap = 3;
        let mut storage = MemoryStorage::new(cap);
        let key = cpu_key();
        for v in [1.0, 2.0, 3.0, 4.0] {
            storage.append(MetricEvent::gauge(key.clone(), v));
        }
        let hist = storage.history(&key, cap);
        assert_eq!(hist.len(), cap);
        // 最初の 1.0 が破棄されて 2,3,4 になっているはず
        let expected = [2.0f32, 3.0, 4.0];
        for (i, (&got, &exp)) in hist.iter().zip(expected.iter()).enumerate() {
            assert!((got - exp).abs() < 1e-5, "hist[{i}]: got={got} exp={exp}");
        }
    }

    /// 正常系: 未記録キーの history は全て 0.0
    #[test]
    fn test_history_all_zeros_for_unknown_key() {
        let storage = MemoryStorage::new(10);
        let hist = storage.history(&cpu_key(), 5);
        assert_eq!(hist.len(), 5);
        for v in hist {
            assert_eq!(v, 0.0);
        }
    }

    /// 正常系: 複数キーが独立して保持される
    #[test]
    fn test_independent_keys() {
        let mut storage = MemoryStorage::new(5);
        let cpu = cpu_key();
        let mem = mem_key();
        storage.append(MetricEvent::gauge(cpu.clone(), 80.0));
        storage.append(MetricEvent::gauge(mem.clone(), 60.0));
        assert!((storage.latest(&cpu).unwrap().value - 80.0).abs() < 1e-5);
        assert!((storage.latest(&mem).unwrap().value - 60.0).abs() < 1e-5);
    }

    /// 異常系: capacity=0 でも latest/history がパニックしない
    #[test]
    fn test_capacity_zero_no_panic() {
        let mut storage = MemoryStorage::new(0);
        let key = cpu_key();
        storage.append(MetricEvent::gauge(key.clone(), 1.0));
        // latest は常に最後に push された値を返す（capacity=0はバッファが即時空になる）
        // history は capacity=0 で空ベクタを返す
        let hist = storage.history(&key, 0);
        assert!(hist.is_empty());
    }

    /// 正常系: ensure_ring は設定値 capacity でリングを事前確保する
    #[test]
    fn test_ensure_ring_creates_ring_with_given_capacity() {
        let mut storage = MemoryStorage::new(10); // default は 10
        let key = cpu_key();
        // 設定値 30 で事前登録
        storage.ensure_ring(&key, 30);
        // 30 件書き込んでも全件保持できる
        for i in 0..30 {
            storage.append(MetricEvent::gauge(key.clone(), i as f32));
        }
        let hist = storage.history(&key, 30);
        assert_eq!(hist.len(), 30);
        // デフォルト capacity(10) だと末尾 10 件しか残らない。
        // 正しく 30 件保持されることを確認する。
        assert!(
            (hist[0] - 0.0).abs() < 1e-5,
            "先頭は0番目: got={}",
            hist[0]
        );
        assert!(
            (hist[29] - 29.0).abs() < 1e-5,
            "末尾は29番目: got={}",
            hist[29]
        );
    }

    /// 正常系: 設定再読込後に ensure_ring を再呼び出しても既存リングが保持される
    #[test]
    fn test_ensure_ring_preserves_existing_ring_on_reload() {
        let mut storage = MemoryStorage::new(10);
        let key = cpu_key();
        storage.ensure_ring(&key, 30);
        // 5 件書き込む
        for i in 0..5_u8 {
            storage.append(MetricEvent::gauge(key.clone(), i as f32));
        }
        // 設定再読込をシミュレート: ensure_ring を再度呼ぶ
        storage.ensure_ring(&key, 30);
        // 履歴は失われていないこと
        let hist = storage.history(&key, 30);
        assert_eq!(hist.len(), 30, "history は常に capacity 本を返す");
        // 末尾 5 件に実値が入っていること
        for (i, &v) in hist[25..].iter().enumerate() {
            assert!(
                (v - i as f32).abs() < 1e-5,
                "hist[{}]={v} (expected {i})",
                25 + i
            );
        }
    }
}
