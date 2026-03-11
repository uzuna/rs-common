use std::{
    error::Error,
    thread,
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
use rhythm_core::{
    comm::{
        instant_millis, UdpMulticastSyncListener, UdpMulticastSyncSender,
        DEFAULT_LISTENER_POLL_TIMEOUT, DEFAULT_MULTICAST_PORT, MAX_BEAT_DIV,
    },
    BpmLimitParam,
};

const LISTENER_DEFAULT_STALE_MS: u64 = 3_000;
const PHASE_WRAP_MIN_JUMP: u16 = 32_768;

#[inline]
fn phase_wrapped_across_zero(prev_phase: u16, current_phase: u16) -> bool {
    prev_phase > current_phase && prev_phase.wrapping_sub(current_phase) >= PHASE_WRAP_MIN_JUMP
}

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
    let bpm_limit = BpmLimitParam::new(60, 120);
    let mut sender = UdpMulticastSyncSender::new(port, bpm, k, beat_div, bpm_limit)?;

    println!("[send] loopback multicast 239.0.0.1:{port} bpm={bpm} k={k}");

    loop {
        let step = sender.step(instant_millis())?;

        if let Some(msg) = step.sent_message {
            let rhythm = sender.rhythm();
            println!(
                "[send] t={}ms beat={} phase={} bpm={} interval={}ms",
                msg.timestamp_ms,
                rhythm.beat_count,
                msg.phase,
                msg.bpm.to_int_round(),
                step.interval_ms,
            );
        }

        thread::sleep(Duration::from_millis(step.interval_ms as u64));
        if let Some(limit) = beat_limit {
            if sender.rhythm().beat_count >= limit {
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
    let bpm_limit = BpmLimitParam::new(60, 120);
    let mut listener = UdpMulticastSyncListener::new(
        port,
        bpm,
        k,
        Some(DEFAULT_LISTENER_POLL_TIMEOUT),
        bpm_limit,
    )?;
    let mut last_tick = Instant::now();
    let mut last_phase: Option<u16> = None;

    println!(
        "[listen] loopback multicast 239.0.0.1:{} bpm={} k={} stale={}ms",
        port, bpm, k, stale_ms
    );

    loop {
        let dt_ms = last_tick.elapsed().as_millis().min(u32::MAX as u128) as u32;
        last_tick = Instant::now();

        let now_ms = instant_millis();
        listener.step(dt_ms.max(1), now_ms)?;
        let local = listener.rhythm();

        let should_print = last_phase
            .map(|prev_phase| phase_wrapped_across_zero(prev_phase, local.phase))
            .unwrap_or(false);
        last_phase = Some(local.phase);

        if should_print {
            println!(
                "[beat] t={}ms beat={} phase={} bpm={}",
                now_ms,
                local.beat_count,
                local.phase,
                local.current_bpm.to_int_round(),
            );
        }
        if let Some(limit) = beat_limit {
            if listener.rhythm().beat_count >= limit {
                return Ok(());
            }
        }
    }
}
