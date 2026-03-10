use std::{
    collections::VecDeque,
    error::Error,
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use rhythm_core::{
    bpm_from_interval_ms, bpm_to_int_round, BpmLimitParam, RhythmGenerator, RhythmMessage,
};

const MIN_ACCEPT_BPM: u64 = 40;
const MAX_ACCEPT_BPM: u64 = 240;
const MIN_ACCEPT_INTERVAL_MS: u64 = 60_000 / MAX_ACCEPT_BPM;
const MAX_ACCEPT_INTERVAL_MS: u64 = 60_000 / MIN_ACCEPT_BPM;
const STALE_INPUT_TIMEOUT_MS: u64 = MAX_ACCEPT_INTERVAL_MS * 2;
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

fn prune_stale_inputs(inputs: &mut VecDeque<RhythmMessage>, now_ms: u64) {
    while let Some(front) = inputs.front() {
        if now_ms.saturating_sub(front.timestamp_ms) > STALE_INPUT_TIMEOUT_MS {
            inputs.pop_front();
        } else {
            break;
        }
    }
}

fn interval_to_bpm(interval_ms: u64) -> u16 {
    let bpm = (60_000_u64 + interval_ms / 2) / interval_ms;
    bpm.min(u16::MAX as u64) as u16
}

fn main() -> Result<(), Box<dyn Error>> {
    let tick = Duration::from_millis(TICK_MS as u64);
    let bpm_limit = BpmLimitParam::new(60, 120);

    let mut local = RhythmGenerator::from_int_bpm(0, 120, 12);
    let mut local_last_beat_count = local.beat_count;

    let mut input_messages = VecDeque::with_capacity(2);
    let mut input_beat_count = 0_u64;

    print_line("rhythm-core CLI example")?;
    print_line("- 自分のビートタイミングでメッセージを出力")?;
    print_line("- スペースキー: 外部ビート入力（40-240bpmのみ受付）")?;
    print_line("- 入力間隔からQ8.8 BPMを推定し、遅延補償syncを適用")?;
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

                        prune_stale_inputs(&mut input_messages, input_now_ms);

                        let mut interval_ms = None;
                        if let Some(last) = input_messages.back() {
                            let delta_ms = input_now_ms.saturating_sub(last.timestamp_ms);
                            if delta_ms == 0 {
                                print_line("[input] 拒否: 同一時刻入力")?;
                                continue;
                            }
                            if !(MIN_ACCEPT_INTERVAL_MS..=MAX_ACCEPT_INTERVAL_MS)
                                .contains(&delta_ms)
                            {
                                print_line(format!(
                                    "[input] 拒否: interval={}ms (許容{}-{}ms)",
                                    delta_ms, MIN_ACCEPT_INTERVAL_MS, MAX_ACCEPT_INTERVAL_MS
                                ))?;
                                input_messages.clear();
                                continue;
                            }
                            interval_ms = Some(delta_ms);
                        }

                        let input_bpm_q8 = match interval_ms {
                            Some(ms) => bpm_from_interval_ms(ms as u32),
                            None => local.current_bpm,
                        };

                        let input_message = RhythmMessage {
                            phase: 0,
                            bpm: input_bpm_q8,
                            timestamp_ms: input_now_ms,
                            beat_count: input_beat_count.min(u32::MAX as u64) as u32,
                        };
                        input_beat_count = input_beat_count.saturating_add(1);

                        if input_messages.len() >= input_messages.capacity() {
                            input_messages.pop_front();
                        }
                        input_messages.push_back(input_message);

                        match interval_ms {
                            Some(ms) => print_line(format!(
                                "[input] beat={} t={}ms interval={}ms (~{}bpm)",
                                input_message.beat_count,
                                input_message.timestamp_ms,
                                ms,
                                interval_to_bpm(ms)
                            ))?,
                            None => print_line(format!(
                                "[input] beat={} t={}ms (基準入力)",
                                input_message.beat_count, input_message.timestamp_ms
                            ))?,
                        }

                        let before = local.to_message(input_now_ms);
                        local.sync(input_message, input_now_ms, &bpm_limit);
                        let after = local.to_message(input_now_ms);
                        local_last_beat_count = local.beat_count;

                        print_line(format!(
                            "[sync] bpm {} -> {} (input={}) phase {} -> {} beat={}",
                            bpm_to_int_round(before.bpm),
                            bpm_to_int_round(after.bpm),
                            bpm_to_int_round(input_message.bpm),
                            before.phase,
                            after.phase,
                            local.beat_count
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
                bpm_to_int_round(local_message.bpm),
                local.beat_count,
            ))?;
        }

        thread::sleep(tick);
    }
}
