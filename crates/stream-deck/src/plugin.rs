//! DataPlugin トレイトと具体的な実装
//!
//! 各プラグインは [`DataPlugin`] を実装し、
//! [`crate::section::Section`] に組み込んで独立したセクション表示単位を構成する。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::metrics::{format_memory, MetricsSource};
use crate::notifications::NotificationState;

/// データ収集プラグインの境界トレイト
///
/// 毎ティックに呼ばれる [`DataPlugin::update`] でデータを収集し、
/// [`DataPlugin::latest_normalized`] で 0.0..=1.0 に正規化した最新値を返す。
pub trait DataPlugin {
    /// データを収集する（毎ティック呼ばれる）
    fn update(&mut self);

    /// 収集済みの最新値を 0.0..=1.0 で返す
    fn latest_normalized(&self) -> f32;

    /// LCD に表示する値文字列（例: "75.3%", "6.2/16G"）
    fn value_text(&self) -> String;

    /// セクションラベル（例: "CPU", "MEM"）
    fn label(&self) -> &'static str;
}

// ── CPU ──────────────────────────────────────────────────────────

/// CPU 使用率プラグイン
pub struct CpuPlugin {
    source: Rc<RefCell<MetricsSource>>,
    latest: f32,
}

impl CpuPlugin {
    pub fn new(source: Rc<RefCell<MetricsSource>>) -> Self {
        Self {
            source,
            latest: 0.0,
        }
    }
}

impl DataPlugin for CpuPlugin {
    fn update(&mut self) {
        if let Some(snap) = self.source.borrow().snapshot() {
            self.latest = snap.cpu_pct;
        }
    }

    fn latest_normalized(&self) -> f32 {
        self.latest / 100.0
    }

    fn value_text(&self) -> String {
        format!("{:.1}%", self.latest)
    }

    fn label(&self) -> &'static str {
        "CPU"
    }
}

// ── MEM ──────────────────────────────────────────────────────────

/// メモリ使用量プラグイン
pub struct MemPlugin {
    source: Rc<RefCell<MetricsSource>>,
    used: u64,
    total: u64,
}

impl MemPlugin {
    pub fn new(source: Rc<RefCell<MetricsSource>>) -> Self {
        Self {
            source,
            used: 0,
            total: 1,
        }
    }
}

impl DataPlugin for MemPlugin {
    fn update(&mut self) {
        if let Some(snap) = self.source.borrow().snapshot() {
            self.used = snap.used_memory_bytes;
            self.total = snap.total_memory_bytes;
        }
    }

    fn latest_normalized(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    fn value_text(&self) -> String {
        format_memory(self.used, self.total)
    }

    fn label(&self) -> &'static str {
        "MEM"
    }
}

// ── LOAD ─────────────────────────────────────────────────────────

/// ロードアベレージプラグイン
pub struct LoadPlugin {
    source: Rc<RefCell<MetricsSource>>,
    latest: f32,
    cpu_count: usize,
}

impl LoadPlugin {
    pub fn new(source: Rc<RefCell<MetricsSource>>) -> Self {
        let cpu_count = source.borrow().cpu_count();
        Self {
            source,
            latest: 0.0,
            cpu_count,
        }
    }
}

impl DataPlugin for LoadPlugin {
    fn update(&mut self) {
        if let Some(snap) = self.source.borrow().snapshot() {
            self.latest = snap.load_one;
            self.cpu_count = snap.cpu_count;
        }
    }

    fn latest_normalized(&self) -> f32 {
        (self.latest / self.cpu_count.max(1) as f32).clamp(0.0, 1.0)
    }

    fn value_text(&self) -> String {
        format!("{:.2}", self.latest)
    }

    fn label(&self) -> &'static str {
        "LOAD"
    }
}

// ── BRIGHT ───────────────────────────────────────────────────────

/// 輝度プラグイン
///
/// `brightness` は `Rc<Cell<u8>>` を介してエンコーダ割り込み側と共有する。
pub struct BrightnessPlugin {
    brightness: Rc<Cell<u8>>,
}

impl BrightnessPlugin {
    pub fn new(brightness: Rc<Cell<u8>>) -> Self {
        Self { brightness }
    }
}

impl DataPlugin for BrightnessPlugin {
    /// 輝度は外部（入力ハンドラ）から更新されるため、ここでは何もしない
    fn update(&mut self) {}

    fn latest_normalized(&self) -> f32 {
        self.brightness.get() as f32 / 100.0
    }

    fn value_text(&self) -> String {
        format!("{}%", self.brightness.get())
    }

    fn label(&self) -> &'static str {
        "BRIGHT"
    }
}

// ── NOTIF ────────────────────────────────────────────────────────

/// 通知カウントプラグイン
///
/// `NotificationState` は `Rc<RefCell<...>>` を介して通知ハンドラと共有する。
pub struct NotifPlugin {
    state: Rc<RefCell<NotificationState>>,
}

impl NotifPlugin {
    pub fn new(state: Rc<RefCell<NotificationState>>) -> Self {
        Self { state }
    }
}

impl DataPlugin for NotifPlugin {
    /// 通知状態は外部（受信ループ・ボタンハンドラ）から更新されるため、ここでは何もしない
    fn update(&mut self) {}

    fn latest_normalized(&self) -> f32 {
        let state = self.state.borrow();
        state.active_ratio()
    }

    fn value_text(&self) -> String {
        self.state.borrow().overlay_text()
    }

    fn label(&self) -> &'static str {
        "NOTIF"
    }
}
