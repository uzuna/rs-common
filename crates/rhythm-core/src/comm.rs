use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

use crate::consts::RHYTHM_MESSAGE_WIRE_SIZE;
use crate::RhythmMessage;

pub const LOOPBACK_ADDR: Ipv4Addr = Ipv4Addr::LOCALHOST;
pub const MULTICAST_GROUP_ADDR: Ipv4Addr = Ipv4Addr::new(239, 0, 0, 1);
pub const DEFAULT_MULTICAST_PORT: u16 = 12_345;

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
