use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    time::Duration,
};

use rhythm_core::{
    comm::{
        LoopbackMulticast, UdpMulticastSyncListener, UdpMulticastSyncSender,
        DEFAULT_LISTENER_POLL_TIMEOUT,
    },
    BpmLimitParam, RhythmGenerator, SyncState,
};

fn pick_unused_udp_port() -> u16 {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("failed to bind temporary UDP socket for port allocation");
    socket
        .local_addr()
        .expect("failed to read local address")
        .port()
}

#[inline]
fn assert_case(label: &str, condition: bool, message: &str) {
    assert!(condition, "[{label}] {message}");
}

// ── 値域確認: 送信ステップは phase グリッド（0/div）で publish する ─────────────────

#[test]
fn value_range_sender_publish_on_phase_grid_cases() {
    struct Case {
        label: &'static str,
        bpm: u16,
        beat_div: u32,
        steps: usize,
        expected_interval_ms: u32,
        expect_subbeat_phase: bool,
    }

    let cases = [
        Case {
            label: "90bpm_quarter_step_publish_each_4step",
            bpm: 90,
            beat_div: 4,
            steps: 8,
            expected_interval_ms: 166,
            expect_subbeat_phase: true,
        },
        Case {
            label: "120bpm_full_step_publish_every_step",
            bpm: 120,
            beat_div: 1,
            steps: 4,
            expected_interval_ms: 500,
            expect_subbeat_phase: false,
        },
    ];

    for case in &cases {
        let port = pick_unused_udp_port();
        let mut sender = UdpMulticastSyncSender::new(
            port,
            case.bpm,
            12,
            case.beat_div,
            BpmLimitParam::new(60, 120),
        )
        .expect("failed to create sender");

        let mut now_ms = 1_000_u64;
        let mut sent_messages = Vec::new();

        for _ in 0..case.steps {
            let step = sender.step(now_ms).expect("sender step failed");
            assert_case(
                case.label,
                step.interval_ms == case.expected_interval_ms,
                "interval_ms が期待値と一致しない",
            );

            if let Some(msg) = step.sent_message {
                sent_messages.push(msg);
            }

            now_ms = now_ms.saturating_add(step.interval_ms as u64);
        }

        assert_case(
            case.label,
            !sent_messages.is_empty(),
            "送信メッセージが 1 件もない",
        );

        let div = case.beat_div.clamp(1, 4);
        let valid_phases: Vec<u16> = (0..div)
            .map(|slot| ((slot as u64 * 65_536_u64) / div as u64) as u16)
            .collect();

        let mut prev_beat_count: Option<u32> = None;
        for msg in &sent_messages {
            assert_case(
                case.label,
                valid_phases.contains(&msg.phase),
                "送信 phase が位相グリッド外",
            );
            if let Some(prev) = prev_beat_count {
                assert_case(
                    case.label,
                    msg.beat_count >= prev,
                    "送信メッセージの beat_count が逆行している",
                );
            }
            prev_beat_count = Some(msg.beat_count);
        }

        if case.expect_subbeat_phase {
            assert_case(
                case.label,
                sent_messages.iter().any(|msg| msg.phase != 0),
                "分割送信なのに phase=0 以外が送信されていない",
            );
            assert_case(
                case.label,
                sent_messages.len() > sender.rhythm().beat_count as usize,
                "分割送信なのに送信回数が beat_count を上回っていない",
            );
        } else {
            assert_case(
                case.label,
                sent_messages.iter().all(|msg| msg.phase == 0),
                "phase=0 送信モードなのに非ゼロ phase が混在している",
            );
        }
    }
}

// ── 正常系: comm の送受信ステップで同期ロックできる ─────────────────────────────

#[test]
fn normal_sync_with_comm_step_cases() {
    struct Case {
        label: &'static str,
        sender_bpm: u16,
        sender_beat_div: u32,
        steps: usize,
        expected_bpm_min: u16,
        expected_bpm_max: u16,
    }

    let cases = [
        Case {
            label: "90bpm_sender_quarter_step_listener_locks_to_90",
            sender_bpm: 90,
            sender_beat_div: 4,
            steps: 64,
            expected_bpm_min: 86,
            expected_bpm_max: 94,
        },
        Case {
            label: "120bpm_sender_full_step_listener_locks_to_120",
            sender_bpm: 120,
            sender_beat_div: 1,
            steps: 16,
            expected_bpm_min: 118,
            expected_bpm_max: 120,
        },
    ];

    for case in &cases {
        let port = pick_unused_udp_port();
        let bpm_limit = BpmLimitParam::new(60, 120);

        let mut sender =
            UdpMulticastSyncSender::new(port, case.sender_bpm, 12, case.sender_beat_div, bpm_limit)
                .expect("failed to create sender");
        let mut listener = UdpMulticastSyncListener::new(
            port,
            120,
            12,
            Some(DEFAULT_LISTENER_POLL_TIMEOUT.min(Duration::from_millis(5))),
            bpm_limit,
        )
        .expect("failed to create listener");

        let mut now_ms = 10_000_u64;
        let mut received_count = 0_usize;

        for _ in 0..case.steps {
            let send_step = sender.step(now_ms).expect("sender step failed");
            now_ms = now_ms.saturating_add(send_step.interval_ms as u64);

            let recv_step = listener
                .step(send_step.interval_ms, now_ms)
                .expect("listener step failed");
            if recv_step.received_message.is_some() {
                received_count += 1;
            }
        }

        let local = listener.rhythm();
        let bpm_now = local.current_bpm.to_int_round();

        assert_case(
            case.label,
            received_count >= 2,
            "同期に必要なメッセージ数を受信できていない",
        );
        assert_case(
            case.label,
            local.sync_state == SyncState::Locked,
            "sync_state が Locked になっていない",
        );
        assert!(
            (case.expected_bpm_min..=case.expected_bpm_max).contains(&bpm_now),
            "[{}] current_bpm が期待範囲に収束していない: bpm_now={} expected={}..={}",
            case.label,
            bpm_now,
            case.expected_bpm_min,
            case.expected_bpm_max,
        );
    }
}

// ── 実用系: ロス・遅延ジッタを注入しても同期ロックを維持できる ─────────────────────

#[test]
fn practical_loss_and_delay_jitter_cases() {
    struct Case {
        label: &'static str,
        sender_bpm: u16,
        sender_beat_div: u32,
        receiver_start_bpm: u16,
        steps: usize,
        drop_every_nth_recv: usize,
        extra_delay_jitter_ms: &'static [u32],
        min_synced_messages: usize,
        expected_bpm_min: u16,
        expected_bpm_max: u16,
    }

    let cases = [
        Case {
            label: "drop_every_3rd_recv_90bpm",
            sender_bpm: 90,
            sender_beat_div: 1,
            receiver_start_bpm: 120,
            steps: 40,
            drop_every_nth_recv: 3,
            extra_delay_jitter_ms: &[0, 6, 12, 3, 15],
            min_synced_messages: 20,
            expected_bpm_min: 88,
            expected_bpm_max: 92,
        },
        Case {
            label: "drop_every_2nd_recv_120bpm",
            sender_bpm: 120,
            sender_beat_div: 1,
            receiver_start_bpm: 90,
            steps: 30,
            drop_every_nth_recv: 2,
            extra_delay_jitter_ms: &[0, 10, 4, 18],
            min_synced_messages: 12,
            expected_bpm_min: 118,
            expected_bpm_max: 120,
        },
    ];

    for case in &cases {
        let port = pick_unused_udp_port();
        let bpm_limit = BpmLimitParam::new(60, 120);

        let mut sender =
            UdpMulticastSyncSender::new(port, case.sender_bpm, 12, case.sender_beat_div, bpm_limit)
                .expect("failed to create sender");
        let receiver_transport = LoopbackMulticast::listener(
            port,
            Some(DEFAULT_LISTENER_POLL_TIMEOUT.min(Duration::from_millis(5))),
        )
        .expect("failed to create receiver transport");
        let mut local = RhythmGenerator::from_int_bpm(0, case.receiver_start_bpm, 12);

        let mut now_ms = 50_000_u64;
        let mut recv_count = 0_usize;
        let mut synced_count = 0_usize;

        for step_idx in 0..case.steps {
            let send_step = sender.step(now_ms).expect("sender step failed");
            now_ms = now_ms.saturating_add(send_step.interval_ms as u64);

            local.update(send_step.interval_ms, &bpm_limit);

            match receiver_transport.recv() {
                Ok(Some(msg)) => {
                    recv_count += 1;

                    let should_drop =
                        case.drop_every_nth_recv > 0 && recv_count % case.drop_every_nth_recv == 0;
                    if should_drop {
                        continue;
                    }

                    let jitter = if case.extra_delay_jitter_ms.is_empty() {
                        0_u64
                    } else {
                        case.extra_delay_jitter_ms[step_idx % case.extra_delay_jitter_ms.len()]
                            as u64
                    };
                    local.sync(msg, now_ms.saturating_add(jitter), &bpm_limit);
                    synced_count += 1;
                }
                Ok(None) => {}
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut => {}
                Err(e) => panic!("[{}] recv failed: {e}", case.label),
            }
        }

        let bpm_now = local.current_bpm.to_int_round();
        assert_case(
            case.label,
            recv_count >= case.min_synced_messages,
            "想定より受信できていない",
        );
        assert_case(
            case.label,
            synced_count >= case.min_synced_messages,
            "同期へ使えたメッセージが不足している",
        );
        assert_case(
            case.label,
            local.sync_state == SyncState::Locked,
            "ロス注入後に Locked を維持できていない",
        );
        assert_case(
            case.label,
            (case.expected_bpm_min..=case.expected_bpm_max).contains(&bpm_now),
            "ロス・遅延ジッタ注入後の BPM が期待範囲外",
        );
    }
}
