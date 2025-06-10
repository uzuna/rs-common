use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

use egui::Ui;
use egui_plot::{Corner, Legend, Line, Plot, PlotPoints};

use crate::plugin::SingleInst;

// 必要な情報源
// 表示タイムスタンプ + 表示時間レンジ
// データのタイムスタンプ
// データの解釈
fn plot_lines<T>(ui: &mut Ui, name: &str, xr: XRange, data: &[T])
where
    T: AsRecord,
{
    let plot = Plot::new(name)
        .legend(Legend::default().position(Corner::LeftBottom))
        .show_axes(true)
        .show_grid(true)
        .height(200.0);
    plot.show(ui, |plot_ui| {
        plot_ui.line(Line::new(
            name,
            PlotPoints::from_iter(data.iter().filter_map(|f| {
                let Some(duration) = xr.in_duration(f.timestamp()) else {
                    return None;
                };
                let x = duration.as_secs_f64();
                let y = f.as_f64();
                Some([x, y])
            })),
        ));
    });
}

// X軸のレンジを表す構造体
#[derive(Debug, Clone, Copy)]
struct XRange {
    start: std::time::Duration,
    end: std::time::Duration,
}

impl XRange {
    fn in_duration(&self, duration: std::time::Duration) -> Option<Duration> {
        if duration >= self.start && duration <= self.end {
            Some(duration)
        } else {
            None
        }
    }
}

// データ保持と表示時の型の違いを吸収するためのトレイト
trait AsRecord {
    fn timestamp(&self) -> std::time::Duration;
    fn as_f64(&self) -> f64;
}

// プロットのためにデータとタイムスタンプの参照を持つ
struct RecordRef<'a, T> {
    timestamp: std::time::Duration,
    value: &'a T,
}

impl AsRecord for RecordRef<'_, f64> {
    fn timestamp(&self) -> std::time::Duration {
        self.timestamp
    }

    fn as_f64(&self) -> f64 {
        *self.value
    }
}

// Sin波形生成
struct SinGenerator {
    frequency: f64,
    phase: f64,
    amplitude: f64,
}

impl SinGenerator {
    fn new(frequency: f64, amplitude: f64) -> Self {
        Self {
            frequency,
            amplitude,
            ..Default::default()
        }
    }

    fn generate(&self, timestamp: std::time::Duration) -> f64 {
        let seconds = timestamp.as_secs_f64();
        (self.frequency * seconds + self.phase).sin() * self.amplitude
    }
}

impl Default for SinGenerator {
    fn default() -> Self {
        Self {
            frequency: 2.0,
            phase: 0.0,
            amplitude: 2_i32.pow(10) as f64,
        }
    }
}

struct RecordStore {
    ts: VecDeque<std::time::Duration>,
    records: VecDeque<f64>,
    plugin_records: BTreeMap<String, VecDeque<f64>>,
    // データ保持数制限
    len: usize,
}

impl RecordStore {
    fn set_plugin(&mut self, plugin: &mut SingleInst<()>) -> anyhow::Result<()> {
        let mut record = VecDeque::with_capacity(self.len);
        let name = plugin.name()?;
        for (v, ts) in self.records.iter().zip(&self.ts) {
            let single = SingleInst::<()>::single(ts.as_nanos() as u64, *v as i16);
            let res = plugin.process(single)?;
            record.push_back(res as f64);
        }
        self.plugin_records
            .entry(name)
            .or_default()
            .append(&mut record);
        Ok(())
    }

    fn push_plugin_record(&mut self, name: &str, value: f64) {
        if let Some(records) = self.plugin_records.get_mut(name) {
            if records.len() >= self.len {
                records.pop_front();
            }
            records.push_back(value);
        }
    }
}

pub struct SignalProcess {
    generator: SinGenerator,
    records: RecordStore,
    duration: Duration,
    timestamp: Duration,
    plugins: BTreeMap<String, SingleInst<()>>,
}

impl SignalProcess {
    pub fn new(frequency: f64, amp: f64, dur: Duration) -> Self {
        let len = (dur.as_secs_f64() * frequency) as usize; // 1分間のデータ数
        let generator = SinGenerator::new(frequency, amp);
        let records = RecordStore {
            ts: VecDeque::with_capacity(len),
            records: VecDeque::with_capacity(len),
            len,
            plugin_records: BTreeMap::new(),
        };
        Self {
            generator,
            records,
            duration: dur,
            timestamp: Duration::ZERO,
            plugins: BTreeMap::new(),
        }
    }

    pub fn add_plugin(&mut self, mut plugin: SingleInst<()>) {
        self.records.set_plugin(&mut plugin).unwrap();
        self.plugins.insert(plugin.name().unwrap(), plugin);
    }

    fn xrange(&self) -> XRange {
        let end = self.timestamp;
        let start = if end > self.duration {
            end - self.duration
        } else {
            Duration::ZERO
        };
        XRange { start, end }
    }

    pub fn update(&mut self, tick: std::time::Duration) -> anyhow::Result<()> {
        self.timestamp += tick;
        let timestamp = self.timestamp;
        let value = self.generator.generate(timestamp);
        if self.records.ts.len() >= self.records.len {
            self.records.ts.pop_front();
            self.records.records.pop_front();
        }
        self.records.ts.push_back(timestamp);
        self.records.records.push_back(value);

        for (name, plugin) in self.plugins.iter_mut() {
            let single = SingleInst::<()>::single(timestamp.as_nanos() as u64, value as i16);
            let res = plugin.process(single)?;
            self.records.push_plugin_record(name, res as f64);
        }
        Ok(())
    }

    pub fn plot(&self, ui: &mut Ui) {
        let data: Vec<_> = self
            .records
            .ts
            .iter()
            .zip(&self.records.records)
            .map(|(ts, val)| RecordRef {
                timestamp: *ts,
                value: val,
            })
            .collect();
        let xr = self.xrange();
        plot_lines(ui, "Sine Wave", xr, &data);
        for (name, records) in &self.records.plugin_records {
            let data: Vec<_> = self
                .records
                .ts
                .iter()
                .zip(records)
                .map(|(ts, val)| RecordRef {
                    timestamp: *ts,
                    value: val,
                })
                .collect();
            plot_lines(ui, name, xr, &data);
        }
    }
}
