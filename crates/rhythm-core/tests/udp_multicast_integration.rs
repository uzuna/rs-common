use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    thread,
    time::{Duration, Instant},
};

use rhythm_core::{comm::LoopbackMulticast, fixed_math::BpmQ8, RhythmMessage};

const RECV_TIMEOUT: Duration = Duration::from_millis(20);
const WAIT_DEADLINE: Duration = Duration::from_secs(2);

fn pick_unused_udp_port() -> u16 {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("failed to bind temporary UDP socket for port allocation");
    socket
        .local_addr()
        .expect("failed to read local address")
        .port()
}

fn send_burst(sender: &LoopbackMulticast, msg: &RhythmMessage, count: usize) -> io::Result<()> {
    for _ in 0..count {
        sender.send(msg)?;
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

fn wait_for_beat(
    listener: &LoopbackMulticast,
    expected_beat: u32,
    timeout: Duration,
) -> io::Result<RhythmMessage> {
    let started = Instant::now();
    let mut last_seen_beat: Option<u32> = None;

    loop {
        match listener.recv() {
            Ok(Some(msg)) => {
                last_seen_beat = Some(msg.beat_count);
                if msg.beat_count == expected_beat {
                    return Ok(msg);
                }
            }
            Ok(None) => {}
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
            }
            Err(e) => return Err(e),
        }

        if started.elapsed() >= timeout {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "timed out waiting beat={expected_beat}, last_seen_beat={last_seen_beat:?}"
                ),
            ));
        }
    }
}

#[test]
fn listener_receives_from_sender() {
    let port = pick_unused_udp_port();
    let listener =
        LoopbackMulticast::listener(port, Some(RECV_TIMEOUT)).expect("failed to create listener");
    let sender = LoopbackMulticast::sender(port).expect("failed to create sender");

    let expected = RhythmMessage::new(1_000, 7, 24_000, BpmQ8::from_int(90));
    send_burst(&sender, &expected, 6).expect("failed to send burst");

    let received =
        wait_for_beat(&listener, expected.beat_count, WAIT_DEADLINE).expect("did not receive beat");

    assert_eq!(received.phase, expected.phase);
    assert_eq!(received.bpm, expected.bpm);
}

#[test]
fn listener_receives_after_sender_restart() {
    let port = pick_unused_udp_port();
    let listener =
        LoopbackMulticast::listener(port, Some(RECV_TIMEOUT)).expect("failed to create listener");

    let first = RhythmMessage::new(2_000, 11, 12_345, BpmQ8::from_int(105));
    {
        let sender = LoopbackMulticast::sender(port).expect("failed to create first sender");
        send_burst(&sender, &first, 4).expect("failed to send first burst");
    }

    let first_received = wait_for_beat(&listener, first.beat_count, WAIT_DEADLINE)
        .expect("first message not received");
    assert_eq!(first_received.phase, first.phase);
    assert_eq!(first_received.bpm, first.bpm);

    thread::sleep(Duration::from_millis(40));

    let second = RhythmMessage::new(3_000, 12, 54_321, BpmQ8::from_int(80));
    {
        let sender = LoopbackMulticast::sender(port).expect("failed to create second sender");
        send_burst(&sender, &second, 6).expect("failed to send second burst");
    }

    let second_received = wait_for_beat(&listener, second.beat_count, WAIT_DEADLINE)
        .expect("second message not received");
    assert_eq!(second_received.phase, second.phase);
    assert_eq!(second_received.bpm, second.bpm);
}
