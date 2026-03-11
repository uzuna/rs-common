use std::{
    error::Error,
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use rhythm_core::{BpmLimitParam, PulseSyncParam, RhythmGenerator};

const TICK_MS: u32 = 10;

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> std::io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn to_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

fn print_line(line: impl core::fmt::Display) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "\r{line}\r\n")?;
    stdout.flush()
}

fn interval_to_bpm(interval_ms: u64) -> u16 {
    let bpm = (60_000_u64 + interval_ms / 2) / interval_ms;
    bpm.min(u16::MAX as u64) as u16
}

fn main() -> Result<(), Box<dyn Error>> {
    let tick = Duration::from_millis(TICK_MS as u64);
    let bpm_limit = BpmLimitParam::new(60, 180);

    let mut local = RhythmGenerator::from_int_bpm(0, 120, 12);
    local.set_pulse_sync_param(PulseSyncParam::new(4, 96));
    let mut local_last_beat_count = local.beat_count;
    let mut last_input_ts_ms: Option<u64> = None;
    let mut input_count = 0_u64;

    print_line("rhythm-core CLI example")?;
    print_line("- 自分のビートタイミングでメッセージを出力")?;
    print_line("- スペースキー: 外部パルス入力（BPMヒントなし   ）")?;
    print_line("- パルス時刻列からBPMを推定し、遅延補償 sync_pulse を適用")?;
    print_line("- 'q' キー: 終了")?;

    let _raw_mode = RawModeGuard::new()?;
    let start = Instant::now();

    loop {
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(key_event) = event::read()? {
                match key_event.code {
                    KeyCode::Char('q') => {
                        print_line("終了します")?;
                        return Ok(());
                    }
                    KeyCode::Char(' ') => {
                        let input_now = start.elapsed();
                        let input_now_ms = to_millis_u64(input_now);

                        let interval_ms =
                            last_input_ts_ms.map(|last| input_now_ms.saturating_sub(last));
                        if let Some(delta_ms) = interval_ms {
                            if delta_ms == 0 {
                                print_line("[input] 拒否: 同一時刻入力")?;
                                continue;
                            }
                        }

                        match interval_ms {
                            Some(ms) => print_line(format!(
                                "[input] pulse={} t={}ms interval={}ms (~{}bpm)",
                                input_count,
                                input_now_ms,
                                ms,
                                interval_to_bpm(ms)
                            ))?,
                            None => print_line(format!(
                                "[input] pulse={} t={}ms (基準入力)",
                                input_count, input_now_ms
                            ))?,
                        }

                        let before = local.to_message(input_now_ms);
                        local.sync_pulse(input_now_ms, input_now_ms, &bpm_limit);
                        let after = local.to_message(input_now_ms);
                        local_last_beat_count = local.beat_count;
                        last_input_ts_ms = Some(input_now_ms);
                        input_count = input_count.saturating_add(1);

                        print_line(format!(
                            "[sync-pulse] bpm {} -> {} phase {} -> {} beat={} state={:?}",
                            before.bpm.to_int_round(),
                            after.bpm.to_int_round(),
                            before.phase,
                            after.phase,
                            local.beat_count,
                            local.sync_state,
                        ))?;
                    }
                    _ => {}
                }
            }
        }

        local.update(TICK_MS, &bpm_limit);

        let now = start.elapsed();

        if local.beat_count != local_last_beat_count {
            local_last_beat_count = local.beat_count;
            let local_message = local.to_message(to_millis_u64(now));
            print_line(format!(
                "[beat t={:>6}ms] phase={:>5} bpm={:>3} beat={}",
                to_millis_u64(now),
                local_message.phase,
                local_message.bpm.to_int_round(),
                local.beat_count,
            ))?;
        }

        thread::sleep(tick);
    }
}
