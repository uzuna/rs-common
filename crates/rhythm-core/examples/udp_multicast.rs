use std::{
    error::Error,
    io, thread,
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
use rhythm_core::{
    bpm_to_int_round,
    comm::{instant_millis, LoopbackMulticast, DEFAULT_MULTICAST_PORT},
    BpmLimitParam, Rhythm, RhythmGenerator, BPM_Q8_ONE, MS_PER_MINUTE,
};

const MIN_SEND_INTERVAL_MS: u32 = 20;
const MAX_BEAT_DIV: u32 = 4;
const LISTENER_POLL_TIMEOUT: Duration = Duration::from_millis(20);
const LISTENER_DEFAULT_STALE_MS: u64 = 3_000;

#[derive(Debug, Parser)]
#[command(name = "udp_multicast")]
#[command(about = "Rhythm UDP loopback multicast demo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Send {
        #[arg(long, default_value_t = DEFAULT_MULTICAST_PORT)]
        port: u16,
        #[arg(long, default_value_t = 120)]
        bpm: u16,
        #[arg(long, default_value_t = 12)]
        k: u16,
        #[arg(long, default_value_t = MAX_BEAT_DIV)]
        beat_div: u32,
        /// 一定のビートを確認したら終了する
        #[arg(long)]
        beat_limit: Option<u64>,
    },
    Listen {
        #[arg(long, default_value_t = DEFAULT_MULTICAST_PORT)]
        port: u16,
        #[arg(long, default_value_t = 120)]
        bpm: u16,
        #[arg(long, default_value_t = 12)]
        k: u16,
        #[arg(long, default_value_t = LISTENER_DEFAULT_STALE_MS)]
        stale_ms: u64,
        /// 一定のビートを確認したら終了する
        #[arg(long)]
        beat_limit: Option<u64>,
    },
}

fn select_send_interval_ms(bpm_q8: u16, beat_div: u32) -> u32 {
    let beat_div = beat_div.clamp(1, MAX_BEAT_DIV);
    if bpm_q8 == 0 {
        return MIN_SEND_INTERVAL_MS;
    }
    let beat_ms = ((MS_PER_MINUTE as u64 * BPM_Q8_ONE as u64) + bpm_q8 as u64 / 2) / bpm_q8 as u64;
    let interval = beat_ms / beat_div as u64;
    interval.max(MIN_SEND_INTERVAL_MS as u64) as u32
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Send {
            port,
            bpm,
            k,
            beat_div,
            beat_limit,
        } => run_sender(port, bpm, k, beat_div, beat_limit),
        Command::Listen {
            port,
            bpm,
            k,
            stale_ms,
            beat_limit,
        } => run_listener(port, bpm, k, stale_ms, beat_limit),
    }
}

fn run_sender(
    port: u16,
    bpm: u16,
    k: u16,
    beat_div: u32,
    beat_limit: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    let transport = LoopbackMulticast::sender(port)?;
    let bpm_limit = BpmLimitParam::new(60, 120);
    let mut rhythm = Rhythm::from_int_bpm(0, bpm, k);
    let mut last_beat = 0_u64;

    println!("[send] loopback multicast 239.0.0.1:{port} bpm={bpm} k={k}");

    loop {
        let interval_ms = select_send_interval_ms(rhythm.current_bpm, beat_div);
        rhythm.update(interval_ms, &bpm_limit);
        let now_ms = instant_millis();

        let msg = rhythm.to_message(now_ms);
        transport.send(&msg)?;

        if rhythm.beat_count != last_beat {
            last_beat = rhythm.beat_count;
            println!(
                "[send] t={}ms beat={} phase={} bpm={} interval={}ms",
                msg.timestamp_ms,
                rhythm.beat_count,
                msg.phase,
                bpm_to_int_round(msg.bpm),
                interval_ms,
            );
        }

        thread::sleep(Duration::from_millis(interval_ms as u64));
        if let Some(limit) = beat_limit {
            if rhythm.beat_count >= limit {
                return Ok(());
            }
        }
    }
}

fn run_listener(
    port: u16,
    bpm: u16,
    k: u16,
    stale_ms: u64,
    beat_limit: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    let transport = LoopbackMulticast::listener(port, Some(LISTENER_POLL_TIMEOUT))?;
    let bpm_limit = BpmLimitParam::new(60, 120);
    let mut local = RhythmGenerator::from_int_bpm(0, bpm, k);
    let mut last_tick = Instant::now();
    let mut last_beat = 0_u64;

    println!(
        "[listen] loopback multicast 239.0.0.1:{} bpm={} k={} stale={}ms",
        port, bpm, k, stale_ms
    );

    loop {
        let dt_ms = last_tick.elapsed().as_millis().min(u32::MAX as u128) as u32;
        last_tick = Instant::now();
        local.update(dt_ms.max(1), &bpm_limit);

        let now_ms = instant_millis();

        match transport.recv() {
            Ok(Some(msg)) => {
                local.sync(msg, now_ms, &bpm_limit);
            }
            Ok(None) => {}
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
            }
            Err(e) => return Err(e.into()),
        }

        if local.beat_count != last_beat {
            last_beat = local.beat_count;
            println!(
                "[beat] t={}ms beat={} phase={} bpm={}",
                now_ms,
                local.beat_count,
                local.phase,
                bpm_to_int_round(local.current_bpm),
            );
        }
        if let Some(limit) = beat_limit {
            if local.beat_count >= limit {
                return Ok(());
            }
        }
    }
}
