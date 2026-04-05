//! Section: DataPlugin をラップし、Storage 経由で履歴を管理する表示単位

use std::cell::RefCell;
use std::rc::Rc;

use crate::plugin::DataPlugin;
use crate::plugin_contract::{MetricEvent, MetricKey};
use crate::renderer::SectionSpec;
use crate::storage::{MemoryStorage, StorageReader, StorageWriter};

/// LCD の1セクション分を表す表示単位
///
/// `plugin` でデータを収集し、[`MemoryStorage`] に正規化済み値のリングバッファを保持する。
/// ストレージは `Section` の外側で生存するため、設定再読込やページ遷移をまたいでも
/// 履歴が消えない。
pub struct Section {
    plugin: Box<dyn DataPlugin>,
    storage: Rc<RefCell<MemoryStorage>>,
    metric_key: MetricKey,
    capacity: usize,
}

impl Section {
    pub fn new(
        plugin: Box<dyn DataPlugin>,
        capacity: usize,
        storage: Rc<RefCell<MemoryStorage>>,
    ) -> Self {
        let metric_key = plugin.metric_key();
        // 設定値の capacity でリングを事前確保する。
        // 既登録キーは既存リングを維持（設定再読込で履歴を失わない）。
        storage.borrow_mut().ensure_ring(&metric_key, capacity);
        Self {
            plugin,
            storage,
            metric_key,
            capacity,
        }
    }

    /// 1ティック分を更新し、Storage に追記する
    pub fn tick(&mut self) {
        self.plugin.update();
        let value = self.plugin.latest_normalized();
        let event = MetricEvent::gauge(self.metric_key.clone(), value);
        self.storage.borrow_mut().append(event);
    }

    /// レンダラに渡す [`SectionSpec`] を生成する
    ///
    /// Storage から直近 `capacity` 件の履歴を取得する。
    /// 件数が `capacity` に満たない場合は先頭を 0.0 でパディングし、
    /// 常に `capacity` 本のバーが描画されるようにする。
    pub fn as_spec(&self) -> SectionSpec {
        let history = self
            .storage
            .borrow()
            .history(&self.metric_key, self.capacity);
        SectionSpec {
            label: self.plugin.label().to_string(),
            value_text: self.plugin.value_text(),
            history,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConstPlugin {
        value: f32,
    }

    impl DataPlugin for ConstPlugin {
        fn update(&mut self) {}
        fn latest_normalized(&self) -> f32 {
            self.value
        }
        fn value_text(&self) -> String {
            format!("{:.1}", self.value)
        }
        fn label(&self) -> &'static str {
            "TEST"
        }
        fn metric_key(&self) -> MetricKey {
            MetricKey::new("test.const").unwrap()
        }
    }

    fn make_section(value: f32, cap: usize) -> Section {
        let storage = Rc::new(RefCell::new(MemoryStorage::new(cap)));
        Section::new(Box::new(ConstPlugin { value }), cap, storage)
    }

    /// リングバッファが capacity を超えない（Storage 側でも同様）
    #[test]
    fn test_ring_buffer_capacity() {
        let cap = 5;
        let mut sec = make_section(0.5, cap);
        for _ in 0..10 {
            sec.tick();
        }
        let spec = sec.as_spec();
        assert_eq!(spec.history.len(), cap, "as_spec は常に capacity 本を返す");
    }

    /// as_spec は capacity 本の履歴を返す（パディングあり）
    #[test]
    fn test_as_spec_pads_to_capacity() {
        let cap = 10;
        let mut sec = make_section(0.5, cap);
        // 3 ティックだけ進める
        for _ in 0..3 {
            sec.tick();
        }
        let spec = sec.as_spec();
        assert_eq!(spec.history.len(), cap, "as_spec は常に capacity 本を返す");
        // 最初の 7 要素はパディング (0.0)
        for &v in &spec.history[..7] {
            assert_eq!(v, 0.0, "パディング要素は 0.0");
        }
        // 末尾 3 要素は実際の値
        for &v in &spec.history[7..] {
            assert!((v - 0.5).abs() < 1e-5, "実値が想定外: {v}");
        }
    }

    /// 満杯後は先頭が破棄される
    #[test]
    fn test_ring_buffer_drops_oldest() {
        let cap = 3;
        let storage = Rc::new(RefCell::new(MemoryStorage::new(cap)));
        let mut sec = Section::new(Box::new(ConstPlugin { value: 0.0 }), cap, storage.clone());
        sec.tick(); // 0.0
        sec.plugin = Box::new(ConstPlugin { value: 1.0 });
        sec.tick(); // 1.0
        sec.tick(); // 1.0
        sec.tick(); // 1.0 (0.0 が押し出される)
        let spec = sec.as_spec();
        assert_eq!(spec.history.len(), cap);
        // すべて 1.0 になっているはず
        for &v in &spec.history {
            assert!((v - 1.0).abs() < 1e-5, "古い値が残っている: {v}");
        }
    }

    /// 設定再読込後に同じ Storage を使う新しい Section が履歴を引き継ぐ
    #[test]
    fn test_storage_survives_section_replacement() {
        let cap = 5;
        let storage = Rc::new(RefCell::new(MemoryStorage::new(cap)));

        // 最初の Section で 3 ティック記録
        {
            let mut sec = Section::new(Box::new(ConstPlugin { value: 0.7 }), cap, storage.clone());
            for _ in 0..3 {
                sec.tick();
            }
        }

        // 設定再読込をシミュレート: 新しい Section を作るが storage は同じ
        let new_sec = Section::new(Box::new(ConstPlugin { value: 0.7 }), cap, storage.clone());
        let spec = new_sec.as_spec();

        // 3件の履歴が維持されていることを確認（先頭2件はパディング）
        assert_eq!(spec.history.len(), cap);
        for &v in &spec.history[..2] {
            assert_eq!(v, 0.0, "パディングは 0.0");
        }
        for &v in &spec.history[2..] {
            assert!((v - 0.7).abs() < 1e-5, "引き継がれた履歴: {v}");
        }
    }

    /// 正常系: Section の capacity が MemoryStorage の default と異なる場合でも正しく保持される
    ///
    /// layout.toml では cpu/mem/load が capacity=30 だが、MemoryStorage は HISTORY_LEN=10 で
    /// 生成されることがある。Section::new() の ensure_ring() で正しい容量が登録されることを確認する。
    #[test]
    fn test_section_capacity_overrides_storage_default() {
        // Storage は default=10 で作成
        let storage = Rc::new(RefCell::new(MemoryStorage::new(10)));
        let section_capacity = 30;

        // Section は capacity=30 で作成 → ensure_ring が 30 でリングを確保
        let mut sec = Section::new(
            Box::new(ConstPlugin { value: 0.5 }),
            section_capacity,
            storage.clone(),
        );

        // 30 ティック書き込む
        for _ in 0..30 {
            sec.tick();
        }

        let spec = sec.as_spec();
        assert_eq!(
            spec.history.len(),
            section_capacity,
            "capacity=30 分が確保される"
        );
        // 30 件全てに実値が入っていること（パディングなし）
        for &v in &spec.history {
            assert!((v - 0.5).abs() < 1e-5, "実値が想定外: {v}");
        }
    }
}
