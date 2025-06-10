use std::time::Duration;

use egui::Ui;
use egui_plot::{Corner, Legend, Line, Plot, PlotPoints};

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
        .show_grid(true);
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
}

impl SinGenerator {
    fn new(frequency: f64, phase: f64) -> Self {
        Self { frequency, phase }
    }

    fn generate(&self, timestamp: std::time::Duration) -> f64 {
        let seconds = timestamp.as_secs_f64();
        (self.frequency * seconds + self.phase).sin()
    }
}

struct RecordStore {
    ts: Vec<std::time::Duration>,
    records: Vec<f64>,
    // データ保持数制限
    len: usize,
}

pub struct SignalProcess {
    generator: SinGenerator,
    records: RecordStore,
    duration: Duration,
    timestamp: Duration,
}

impl SignalProcess {
    pub fn new(frequency: f64, phase: f64, dur: Duration) -> Self {
        let len = (dur.as_secs_f64() * frequency) as usize; // 1分間のデータ数
        let generator = SinGenerator::new(frequency, phase);
        let records = RecordStore {
            ts: Vec::with_capacity(len),
            records: Vec::with_capacity(len),
            len,
        };
        Self {
            generator,
            records,
            duration: dur,
            timestamp: Duration::ZERO,
        }
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

    pub fn update(&mut self, tick: std::time::Duration) {
        self.timestamp += tick;
        let timestamp = self.timestamp;
        let value = self.generator.generate(timestamp);
        if self.records.ts.len() >= self.records.len {
            self.records.ts.remove(0);
            self.records.records.remove(0);
        }
        self.records.ts.push(timestamp);
        self.records.records.push(value);
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
    }
}
