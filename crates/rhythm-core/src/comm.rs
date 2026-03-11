use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

use crate::consts::{BPM_Q8_ONE, MS_PER_MINUTE, RHYTHM_MESSAGE_WIRE_SIZE};
use crate::{BpmLimitParam, Rhythm, RhythmGenerator, RhythmMessage};

pub const LOOPBACK_ADDR: Ipv4Addr = Ipv4Addr::LOCALHOST;
pub const MULTICAST_GROUP_ADDR: Ipv4Addr = Ipv4Addr::new(239, 0, 0, 1);
pub const DEFAULT_MULTICAST_PORT: u16 = 12_345;
pub const MIN_SEND_INTERVAL_MS: u32 = 20;
pub const MAX_BEAT_DIV: u32 = 4;
pub const DEFAULT_LISTENER_POLL_TIMEOUT: Duration = Duration::from_millis(20);

#[inline]
fn phase_slot_index(phase: u16, beat_div: u32) -> u32 {
    ((phase as u64 * beat_div as u64) >> 16) as u32
}

#[inline]
fn phase_for_slot(slot: u32, beat_div: u32) -> u16 {
    ((slot as u64 * 65_536_u64) / beat_div as u64) as u16
}

/// 送信周期（ms）を計算する。
///
/// `beat_div=1` で 1拍ごと、`beat_div=4` で 1/4 拍ごとの内部更新間隔になる。
#[inline]
pub fn select_send_interval_ms(bpm_q8: u16, beat_div: u32) -> u32 {
    let beat_div = beat_div.clamp(1, MAX_BEAT_DIV);
    if bpm_q8 == 0 {
        return MIN_SEND_INTERVAL_MS;
    }

    let beat_ms = ((MS_PER_MINUTE as u64 * BPM_Q8_ONE as u64) + bpm_q8 as u64 / 2) / bpm_q8 as u64;
    let interval = beat_ms / beat_div as u64;
    interval.max(MIN_SEND_INTERVAL_MS as u64) as u32
}

/// monotonic clock から現在時刻をミリ秒で取得する。実装は nix クレートの time モジュールを使用する。
///
/// # Panic
///
/// nix::time::ClockId::CLOCK_MONOTONIC.now() が失敗した場合にパニックする。
#[inline]
pub fn instant_millis() -> u64 {
    let monotonic: Duration = nix::time::ClockId::CLOCK_MONOTONIC.now().unwrap().into();
    monotonic.as_millis() as u64
}

pub struct LoopbackMulticast {
    socket: UdpSocket,
    target: SocketAddrV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenderStepResult {
    pub interval_ms: u32,
    pub sent_message: Option<RhythmMessage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenerStepResult {
    pub received_message: Option<RhythmMessage>,
    pub beat_changed: bool,
}

/// `udp_multicast` 送信側の共通ロジック。
///
/// 内部更新は `beat_div` 分割で行い、位相グリッドを跨いだタイミングで送信する。
/// `beat_div=1` なら `phase=0`、`beat_div>1` なら分割位相（例: 4 分割で 0/16384/32768/49152）。
pub struct UdpMulticastSyncSender {
    transport: LoopbackMulticast,
    rhythm: Rhythm,
    bpm_limit_param: BpmLimitParam,
    beat_div: u32,
    last_sent_slot: u64,
}

impl UdpMulticastSyncSender {
    pub fn new(
        port: u16,
        bpm: u16,
        coupling_divisor: u16,
        beat_div: u32,
        bpm_limit_param: BpmLimitParam,
    ) -> io::Result<Self> {
        let beat_div = beat_div.clamp(1, MAX_BEAT_DIV);
        let rhythm = Rhythm::from_int_bpm(0, bpm, coupling_divisor);
        let current_slot = rhythm
            .beat_count
            .saturating_mul(beat_div as u64)
            .saturating_add(phase_slot_index(rhythm.phase, beat_div) as u64);

        Ok(Self {
            transport: LoopbackMulticast::sender(port)?,
            rhythm,
            bpm_limit_param: bpm_limit_param.sanitize(),
            beat_div,
            last_sent_slot: current_slot,
        })
    }

    /// 1ステップ進め、位相グリッドを跨いだ場合のみメッセージを送信する。
    pub fn step(&mut self, now_ms: u64) -> io::Result<SenderStepResult> {
        let interval_ms = select_send_interval_ms(self.rhythm.current_bpm.raw(), self.beat_div);
        self.rhythm.update(interval_ms, &self.bpm_limit_param);

        let slot_in_beat = phase_slot_index(self.rhythm.phase, self.beat_div);
        let current_slot = self
            .rhythm
            .beat_count
            .saturating_mul(self.beat_div as u64)
            .saturating_add(slot_in_beat as u64);

        let mut sent_message = None;
        if current_slot != self.last_sent_slot {
            self.last_sent_slot = current_slot;

            let mut msg = self.rhythm.to_message(now_ms);
            msg.phase = phase_for_slot(slot_in_beat, self.beat_div);
            self.transport.send(&msg)?;
            sent_message = Some(msg);
        }

        Ok(SenderStepResult {
            interval_ms,
            sent_message,
        })
    }

    #[inline]
    pub fn rhythm(&self) -> &RhythmGenerator {
        &self.rhythm
    }
}

/// `udp_multicast` 受信側の共通ロジック。
pub struct UdpMulticastSyncListener {
    transport: LoopbackMulticast,
    rhythm: RhythmGenerator,
    bpm_limit_param: BpmLimitParam,
    last_reported_beat: u64,
}

impl UdpMulticastSyncListener {
    pub fn new(
        port: u16,
        bpm: u16,
        coupling_divisor: u16,
        timeout: Option<Duration>,
        bpm_limit_param: BpmLimitParam,
    ) -> io::Result<Self> {
        Ok(Self {
            transport: LoopbackMulticast::listener(port, timeout)?,
            rhythm: RhythmGenerator::from_int_bpm(0, bpm, coupling_divisor),
            bpm_limit_param: bpm_limit_param.sanitize(),
            last_reported_beat: 0,
        })
    }

    /// 1ステップ進め、受信できた場合は `sync` を適用する。
    pub fn step(&mut self, dt_ms: u32, now_ms: u64) -> io::Result<ListenerStepResult> {
        self.rhythm.update(dt_ms.max(1), &self.bpm_limit_param);

        let received_message = match self.transport.recv() {
            Ok(msg) => msg,
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                None
            }
            Err(e) => return Err(e),
        };

        if let Some(msg) = received_message {
            self.rhythm.sync(msg, now_ms, &self.bpm_limit_param);
        }

        let beat_changed = if self.rhythm.beat_count != self.last_reported_beat {
            self.last_reported_beat = self.rhythm.beat_count;
            true
        } else {
            false
        };

        Ok(ListenerStepResult {
            received_message,
            beat_changed,
        })
    }

    #[inline]
    pub fn rhythm(&self) -> &RhythmGenerator {
        &self.rhythm
    }
}

impl LoopbackMulticast {
    /// 送信用ソケット。送信インターフェースは loopback に固定する。
    pub fn sender(port: u16) -> io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into())?;
        socket.set_multicast_loop_v4(true)?;
        socket.set_multicast_ttl_v4(1)?;
        socket.set_multicast_if_v4(&LOOPBACK_ADDR)?;

        Ok(Self {
            socket: socket.into(),
            target: SocketAddrV4::new(MULTICAST_GROUP_ADDR, port),
        })
    }

    /// 受信用ソケット。複数プロセス共有を想定して reuse 設定を行う。
    pub fn listener(port: u16, timeout: Option<Duration>) -> io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;

        // Multicast受信は group宛先を受けるため INADDR_ANY で bind する。
        socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
        socket.join_multicast_v4(&MULTICAST_GROUP_ADDR, &LOOPBACK_ADDR)?;
        socket.set_multicast_loop_v4(true)?;
        socket.set_multicast_if_v4(&LOOPBACK_ADDR)?;

        let udp: UdpSocket = socket.into();
        udp.set_read_timeout(timeout)?;

        Ok(Self {
            socket: udp,
            target: SocketAddrV4::new(MULTICAST_GROUP_ADDR, port),
        })
    }

    pub fn send(&self, message: &RhythmMessage) -> io::Result<usize> {
        self.socket.send_to(&message.to_wire_bytes(), self.target)
    }

    pub fn recv(&self) -> io::Result<Option<RhythmMessage>> {
        let mut buf = [0_u8; RHYTHM_MESSAGE_WIRE_SIZE];
        match self.socket.recv_from(&mut buf) {
            Ok((size, _)) => {
                if size != RHYTHM_MESSAGE_WIRE_SIZE {
                    return Ok(None);
                }
                Ok(RhythmMessage::from_wire_slice(&buf))
            }
            Err(e) => Err(e),
        }
    }

    pub fn socket(&self) -> &UdpSocket {
        &self.socket
    }
}
