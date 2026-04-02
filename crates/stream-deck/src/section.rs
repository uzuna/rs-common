//! Section: DataPlugin をラップし、履歴バッファを管理する表示単位

use std::collections::VecDeque;

use crate::plugin::DataPlugin;
use crate::renderer::SectionSpec;

/// LCD の1セクション分を表す表示単位
///
/// `plugin` でデータを収集し、`history` に正規化済み値のリングバッファを保持する。
/// `capacity` を変えるだけで同一プラグイン型を複数のセクション長で共存させられる。
///
/// ```text
/// Section::new(Box::new(CpuPlugin::new(src.clone())), 10)  // 10 秒履歴
/// Section::new(Box::new(CpuPlugin::new(src.clone())), 30)  // 30 秒履歴
/// ```
pub struct Section {
    plugin: Box<dyn DataPlugin>,
    history: VecDeque<f32>,
    capacity: usize,
}

impl Section {
    pub fn new(plugin: Box<dyn DataPlugin>, capacity: usize) -> Self {
        Self {
            plugin,
            history: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// 1ティック分を更新し、履歴バッファに追記する
    pub fn tick(&mut self) {
        self.plugin.update();
        let v = self.plugin.latest_normalized();
        if self.history.len() == self.capacity {
            self.history.pop_front();
        }
        self.history.push_back(v);
    }

    /// レンダラに渡す [`SectionSpec`] を生成する
    ///
    /// 履歴が `capacity` に満たない場合は先頭を 0.0 でパディングし、
    /// 常に `capacity` 本のバーが描画されるようにする。
    pub fn as_spec(&self) -> SectionSpec {
        let pad = self.capacity.saturating_sub(self.history.len());
        let mut history = vec![0.0f32; pad];
        history.extend(self.history.iter().copied());
        SectionSpec {
            label: self.plugin.label(),
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
    }

    /// リングバッファが capacity を超えない
    #[test]
    fn test_ring_buffer_capacity() {
        let cap = 5;
        let mut sec = Section::new(Box::new(ConstPlugin { value: 0.5 }), cap);
        for _ in 0..10 {
            sec.tick();
            assert!(sec.history.len() <= cap, "履歴が capacity を超えた");
        }
        assert_eq!(sec.history.len(), cap);
    }

    /// as_spec は capacity 本の履歴を返す（パディングあり）
    #[test]
    fn test_as_spec_pads_to_capacity() {
        let cap = 10;
        let mut sec = Section::new(Box::new(ConstPlugin { value: 0.5 }), cap);
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
        let mut sec = Section::new(Box::new(ConstPlugin { value: 0.0 }), cap);
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
}
