use std::{
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    thread,
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
use rhythm_core::{Rhythm, RhythmMessage, MS_PER_MINUTE, RHYTHM_MESSAGE_WIRE_SIZE};

const MULTICAST_PORT: u16 = 12_345;
const LOCALHOST_ADDR: Ipv4Addr = Ipv4Addr::LOCALHOST;
const MIN_SEND_INTERVAL_MS: u64 = 100;
const MAX_BEAT_DIV: u64 = 4;
const LISTENER_POLL_TIMEOUT: Duration = Duration::from_millis(20);
const LISTENER_DEFAULT_STALE_MS: u64 = 3_000;
const LISTENER_SYNC_BLEND_DIVISOR: i32 = 8;
const PHASE_MODULUS: u128 = (u16::MAX as u128) + 1;

#[derive(Debug, Parser)]
#[command(name = "udp_multicast")]
#[command(about = "Rhythm localhost UDP sender/listener")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    // ローカルで生成したビートを localhost のみへ送信する。
    Send {
        #[arg(long, default_value_t = 120)]
        bpm: u16,
        #[arg(long, default_value_t = 24)]
        k: u16,
        // beat を何分割した周期で送信するか（1..=4）。
        #[arg(long, default_value_t = MAX_BEAT_DIV, value_parser = clap::value_parser!(u64).range(1..=MAX_BEAT_DIV))]
        beat_div: u64,
    },
    // ローカルビートを維持しつつ、受信メッセージへ漸進同期する。
    Listen {
        #[arg(long, default_value_t = 120)]
        bpm: u16,
        #[arg(long, default_value_t = 16)]
        k: u16,
        #[arg(long, default_value_t = LISTENER_DEFAULT_STALE_MS)]
        stale_ms: u64,
    },
}

// 送信周期は「要求された beat/div」から試し、100ms以下になる場合は div を下げて安全側へ寄せる。
fn select_send_period(bpm: u16, requested_div: u64) -> (Duration, u64) {
    let mut div = requested_div.clamp(1, MAX_BEAT_DIV);

    if bpm == 0 {
        return (Duration::from_millis(MIN_SEND_INTERVAL_MS + 1), div);
    }

    let beat_ms = (MS_PER_MINUTE as u64 + bpm as u64 / 2) / bpm as u64;

    while div > 1 {
        let interval_ms = beat_ms / div;
        if interval_ms > MIN_SEND_INTERVAL_MS {
            return (Duration::from_millis(interval_ms), div);
        }

        div -= 1;
    }

    (
        Duration::from_millis((beat_ms).max(MIN_SEND_INTERVAL_MS + 1)),
        1,
    )
}

// u128ミリ秒をログ用途の u64 に安全に丸める。
fn to_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

// 値を一気にジャンプさせず、差分を段階的に縮める。
fn blend_toward(current: u16, target: u16, divisor: i32) -> u16 {
    let diff = target as i32 - current as i32;
    if diff == 0 {
        return current;
    }

    let mut step = diff / divisor;
    if step == 0 {
        step = diff.signum();
    }

    (current as i32 + step).clamp(0, u16::MAX as i32) as u16
}

// 受信時点のメッセージを現在時刻へ外挿して、比較用の位相・beatを推定する。
fn extrapolate_remote(remote: RhythmMessage, elapsed_since_receive_ms: u64) -> (u16, u64) {
    let phase_step = (remote.bpm as u128 * PHASE_MODULUS * elapsed_since_receive_ms as u128
        / MS_PER_MINUTE as u128)
        % PHASE_MODULUS;
    let predicted_phase = remote.phase.wrapping_add(phase_step as u16);

    let additional_beats = ((remote.bpm as u128 * elapsed_since_receive_ms as u128)
        / MS_PER_MINUTE as u128)
        .min(u64::MAX as u128) as u64;
    let predicted_beat = remote.beat_count.saturating_add(additional_beats);

    (predicted_phase, predicted_beat)
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Send { bpm, k, beat_div } => run_broadcaster(bpm, k, beat_div),
        Command::Listen { bpm, k, stale_ms } => run_listener(bpm, k, stale_ms),
    }
}

fn run_broadcaster(initial_bpm: u16, k: u16, beat_div: u64) -> Result<(), Box<dyn Error>> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;

    let target = SocketAddrV4::new(LOCALHOST_ADDR, MULTICAST_PORT);
    let mut rhythm = Rhythm::new(0, initial_bpm, k);
    let (mut send_interval, mut send_div) = select_send_period(rhythm.current_bpm, beat_div);
    let mut last_beat = rhythm.beat_count;
    let start = Instant::now();

    println!(
        "[send] target={} interval={}ms (beat/{}) requested_div={}",
        target,
        send_interval.as_millis(),
        send_div,
        beat_div,
    );

    loop {
        // 前回確定した送信周期でローカル位相を進める。
        rhythm.update(send_interval);
        let now = start.elapsed();

        let msg = rhythm.to_message(now);
        let bytes = msg.to_wire_bytes();
        socket.send_to(&bytes, target)?;

        // current_bpm に応じて周期を再計算し、必要なら次ループから反映する。
        let (next_interval, next_div) = select_send_period(msg.bpm, beat_div);
        if next_interval != send_interval || next_div != send_div {
            println!(
                "[send] pacing update bpm={} interval={}ms (beat/{}) requested_div={}",
                msg.bpm,
                next_interval.as_millis(),
                next_div,
                beat_div,
            );
            send_interval = next_interval;
            send_div = next_div;
        }

        // beat境界でのみ送信状態を表示する。
        if msg.beat_count != last_beat {
            last_beat = msg.beat_count;
            println!(
                "[send] t={}ms beat={} phase={} bpm={} bytes={} interval={}ms (beat/{})",
                msg.timestamp_ms,
                msg.beat_count,
                msg.phase,
                msg.bpm,
                RHYTHM_MESSAGE_WIRE_SIZE,
                send_interval.as_millis(),
                send_div,
            );
        }

        thread::sleep(send_interval);
    }
}

fn run_listener(initial_bpm: u16, k: u16, stale_ms: u64) -> Result<(), Box<dyn Error>> {
    let bind_addr = SocketAddrV4::new(LOCALHOST_ADDR, MULTICAST_PORT);
    let socket = UdpSocket::bind(bind_addr)?;
    socket.set_read_timeout(Some(LISTENER_POLL_TIMEOUT))?;

    println!(
        "[listen] bind {} (poll={}ms, stale={}ms, local_bpm={}, k={})",
        bind_addr,
        LISTENER_POLL_TIMEOUT.as_millis(),
        stale_ms,
        initial_bpm,
        k,
    );

    let mut local = Rhythm::new(0, initial_bpm, k);
    let mut last_local_beat = local.beat_count;
    let mut latest_remote: Option<(RhythmMessage, u64)> = None;

    let mut buf = [0_u8; 1500];
    let start = Instant::now();
    let mut last_tick = Instant::now();

    loop {
        match socket.recv_from(&mut buf) {
            Ok((size, _from)) => {
                let raw = &buf[..size];

                if let Some(msg) = RhythmMessage::from_wire_slice(raw) {
                    // 受信内容は標準出力せず、最新サンプルとして内部状態だけ更新する。
                    let now_ms = to_millis_u64(start.elapsed());
                    latest_remote = Some((msg, now_ms));
                }
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err.into()),
        }

        // listener自身のクロックも止めずに進める。
        let dt = last_tick.elapsed();
        last_tick = Instant::now();
        local.update(dt);

        let now = start.elapsed();
        let now_ms = to_millis_u64(now);

        if let Some((remote, received_ms)) = latest_remote {
            if now_ms.saturating_sub(received_ms) <= stale_ms {
                // 新鮮な受信データに対して、BPMを漸進補間しつつ位相同期をかける。
                let elapsed_since_receive_ms = now_ms.saturating_sub(received_ms);
                let (predicted_phase, _) = extrapolate_remote(remote, elapsed_since_receive_ms);
                local.base_bpm =
                    blend_toward(local.base_bpm, remote.bpm, LISTENER_SYNC_BLEND_DIVISOR);
                local.current_bpm =
                    blend_toward(local.current_bpm, remote.bpm, LISTENER_SYNC_BLEND_DIVISOR);
                local.sync(predicted_phase);
            } else {
                latest_remote = None;
            }
        }

        // beatごとに「受信推定との差分」を観測できるログを出す。
        if local.beat_count != last_local_beat {
            last_local_beat = local.beat_count;

            if let Some((remote, received_ms)) = latest_remote {
                let elapsed_since_receive_ms = now_ms.saturating_sub(received_ms);
                let (predicted_phase, predicted_beat) =
                    extrapolate_remote(remote, elapsed_since_receive_ms);
                let phase_error = local.phase_error(predicted_phase);
                let age_ms = elapsed_since_receive_ms;
                let bpm_diff = local.current_bpm as i32 - remote.bpm as i32;
                let beat_diff = local.beat_count as i128 - predicted_beat as i128;

                println!(
                    "[beat] t={}ms beat={} phase={} bpm={} sync_age={}ms diff_phase={} diff_bpm={} diff_beat={}",
                    now_ms,
                    local.beat_count,
                    local.phase,
                    local.current_bpm,
                    age_ms,
                    phase_error,
                    bpm_diff,
                    beat_diff,
                );
            } else {
                println!(
                    "[beat] t={}ms beat={} phase={} bpm={} (free-run)",
                    now_ms, local.beat_count, local.phase, local.current_bpm,
                );
            }
        }
    }
}
