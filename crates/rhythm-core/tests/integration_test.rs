use rhythm_core::{
    fixed_math::{BpmLimitParam, BpmQ8},
    PulseSyncParam, RhythmGenerator, RhythmMessage, SyncState,
};

const PHASE_ONE_PERCENT: i32 = 655;
const QUARTER_PHASE: u16 = 16_384;

#[inline]
fn phase_abs_diff(a: u16, b: u16) -> i32 {
    a.wrapping_sub(b) as i16 as i32
}

#[inline]
fn assert_case(label: &str, condition: bool, message: &str) {
    assert!(condition, "[{label}] {message}");
}

// ── 値域確認: 自律更新のラップアラウンドと拍カウント ─────────────────────────────

#[test]
fn value_range_wraparound_cases() {
    struct Case {
        label: &'static str,
        bpm: u16,
        bpm_limit: BpmLimitParam,
        updates_ms: &'static [u32],
        expected_beats: u64,
        expected_phase: u16,
        max_phase_error: i32,
    }

    let cases = [
        Case {
            label: "120bpm_500ms_1beat",
            bpm: 120,
            bpm_limit: BpmLimitParam::new(60, 120),
            updates_ms: &[500],
            expected_beats: 1,
            expected_phase: 0,
            max_phase_error: 0,
        },
        Case {
            label: "120bpm_250ms_x2",
            bpm: 120,
            bpm_limit: BpmLimitParam::new(60, 120),
            updates_ms: &[250, 250],
            expected_beats: 1,
            expected_phase: 0,
            max_phase_error: 0,
        },
        Case {
            label: "90bpm_2000ms_3beat",
            bpm: 90,
            bpm_limit: BpmLimitParam::new(60, 120),
            updates_ms: &[2_000],
            expected_beats: 3,
            expected_phase: 0,
            max_phase_error: 0,
        },
    ];

    for case in &cases {
        let mut generator = RhythmGenerator::from_int_bpm(0, case.bpm, 12);
        for dt_ms in case.updates_ms {
            generator.update(*dt_ms, &case.bpm_limit);
        }

        assert_case(
            case.label,
            generator.beat_count == case.expected_beats,
            "beat_count が期待値と一致しない",
        );

        let phase_error = phase_abs_diff(generator.phase, case.expected_phase).abs();
        assert_case(
            case.label,
            phase_error <= case.max_phase_error,
            "phase が期待値からずれている",
        );
    }
}

// ── 明示検証: beat_count ラップ境界でも sync の BPM 推定が破綻しない ───────────────

#[test]
fn explicit_sync_beat_count_wraparound_cases() {
    struct Case {
        label: &'static str,
        start_bpm: u16,
        hinted_bpm: u16,
        first_beat_count: u32,
        beat_step_per_message: u32,
        interval_ms: u32,
        expected_bpm_min: u16,
        expected_bpm_max: u16,
    }

    let cases = [
        Case {
            label: "wrap_max_to_zero_120bpm",
            start_bpm: 60,
            hinted_bpm: 120,
            first_beat_count: u32::MAX,
            beat_step_per_message: 1,
            interval_ms: 500,
            expected_bpm_min: 119,
            expected_bpm_max: 120,
        },
        Case {
            label: "wrap_sparse_every_2beats_90bpm",
            start_bpm: 120,
            hinted_bpm: 90,
            first_beat_count: u32::MAX - 1,
            beat_step_per_message: 2,
            interval_ms: 1_333,
            expected_bpm_min: 89,
            expected_bpm_max: 91,
        },
    ];

    for case in &cases {
        let bpm_limit = BpmLimitParam::new(60, 120);
        let mut generator = RhythmGenerator::from_int_bpm(0, case.start_bpm, 12);
        let mut now_ms = 100_000_u64;

        let first = RhythmMessage::new(
            now_ms,
            case.first_beat_count,
            0,
            BpmQ8::from_int(case.hinted_bpm),
        );
        generator.sync(first, now_ms, &bpm_limit);
        assert_case(
            case.label,
            generator.sync_state == SyncState::WaitSecondPoint,
            "1点目受信後に WaitSecondPoint へ遷移しない",
        );

        generator.update(case.interval_ms, &bpm_limit);
        now_ms += case.interval_ms as u64;
        let second_beat_count = case
            .first_beat_count
            .wrapping_add(case.beat_step_per_message);
        let second = RhythmMessage::new(
            now_ms,
            second_beat_count,
            0,
            BpmQ8::from_int(case.hinted_bpm),
        );
        generator.sync(second, now_ms, &bpm_limit);

        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "2点観測後に Locked へ遷移しない",
        );
        assert_case(
            case.label,
            (case.expected_bpm_min..=case.expected_bpm_max)
                .contains(&generator.base_bpm.to_int_round()),
            "ラップ境界観測後の base_bpm が期待範囲外",
        );

        // Locked 後の追従でもラップ算術が破綻しないことを確認する。
        generator.update(case.interval_ms, &bpm_limit);
        now_ms += case.interval_ms as u64;
        let third_beat_count = second_beat_count.wrapping_add(case.beat_step_per_message);
        let third = RhythmMessage::new(
            now_ms,
            third_beat_count,
            0,
            BpmQ8::from_int(case.hinted_bpm),
        );
        generator.sync(third, now_ms, &bpm_limit);

        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "Locked 維持中に同期状態が崩れた",
        );
        assert_case(
            case.label,
            (case.expected_bpm_min..=case.expected_bpm_max)
                .contains(&generator.current_bpm.to_int_round()),
            "Locked 維持中の current_bpm が期待範囲外",
        );
    }
}

// ── 正常系: 低頻度入力で 2点観測ロックし、位相誤差 1% 以内へ収束 ───────────────────

#[test]
fn normal_low_frequency_lock_cases() {
    struct Case {
        label: &'static str,
        start_phase: u16,
        start_bpm: u16,
        bpm_limit: BpmLimitParam,
        reference_bpm: u16,
        beat_stride: u32,
        message_count: u32,
        expected_bpm_min: u16,
        expected_bpm_max: u16,
    }

    let cases = [
        Case {
            label: "start_phase_offset_90bpm",
            start_phase: 24_000,
            start_bpm: 120,
            bpm_limit: BpmLimitParam::new(60, 120),
            reference_bpm: 90,
            beat_stride: 1,
            message_count: 8,
            expected_bpm_min: 90,
            expected_bpm_max: 90,
        },
        Case {
            label: "start_quarter_offset_90bpm",
            start_phase: QUARTER_PHASE,
            start_bpm: 105,
            bpm_limit: BpmLimitParam::new(60, 120),
            reference_bpm: 90,
            beat_stride: 1,
            message_count: 8,
            expected_bpm_min: 90,
            expected_bpm_max: 90,
        },
        Case {
            label: "sparse_messages_every_2beats_120bpm",
            start_phase: 8_000,
            start_bpm: 60,
            bpm_limit: BpmLimitParam::new(60, 120),
            reference_bpm: 120,
            beat_stride: 2,
            message_count: 6,
            expected_bpm_min: 120,
            expected_bpm_max: 120,
        },
    ];

    for case in &cases {
        let mut generator = RhythmGenerator::from_int_bpm(case.start_phase, case.start_bpm, 12);
        let mut now_ms = 0_u64;

        let first = RhythmMessage::new(now_ms, 0, 0, BpmQ8::from_int(case.reference_bpm));
        generator.sync(first, now_ms, &case.bpm_limit);
        assert_case(
            case.label,
            generator.sync_state == SyncState::WaitSecondPoint,
            "1点目受信後に WaitSecondPoint へ遷移しない",
        );

        for idx in 1..=case.message_count {
            let interval_ms = (60_000_u32 / case.reference_bpm as u32) * case.beat_stride;
            generator.update(interval_ms, &case.bpm_limit);
            now_ms += interval_ms as u64;

            let beat = idx * case.beat_stride;
            let msg = RhythmMessage::new(now_ms, beat, 0, BpmQ8::from_int(case.reference_bpm));
            generator.sync(msg, now_ms, &case.bpm_limit);
        }

        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "2点観測後に Locked へ遷移しない",
        );
        assert_case(
            case.label,
            (case.expected_bpm_min..=case.expected_bpm_max)
                .contains(&generator.base_bpm.to_int_round()),
            "base_bpm が期待 BPM 範囲に収束しない",
        );

        let phase_error = phase_abs_diff(generator.phase, 0).abs();
        assert_case(
            case.label,
            phase_error <= PHASE_ONE_PERCENT,
            "位相誤差が 1% を超えている",
        );
    }
}

// ── 異常系: 範囲外 BPM(180) を 2:1 分周(90)としてロック ─────────────────────────

#[test]
fn abnormal_harmonic_fold_cases() {
    struct Case {
        label: &'static str,
        input_bpm: u16,
        bpm_limit: BpmLimitParam,
        expected_bpm: u16,
        interval_ms: u32,
        beats_to_run: u32,
    }

    let cases = [
        Case {
            label: "180_to_90_fold",
            input_bpm: 180,
            bpm_limit: BpmLimitParam::new(60, 120),
            expected_bpm: 90,
            interval_ms: 333,
            beats_to_run: 10,
        },
        Case {
            label: "180_to_45_fold_with_dynamic_limit",
            input_bpm: 180,
            bpm_limit: BpmLimitParam::new(40, 80),
            expected_bpm: 45,
            interval_ms: 333,
            beats_to_run: 10,
        },
    ];

    for case in &cases {
        let mut generator = RhythmGenerator::from_int_bpm(0, 60, 12);
        let mut now_ms = 0_u64;

        let first = RhythmMessage::new(now_ms, 0, 0, BpmQ8::from_int(case.input_bpm));
        generator.sync(first, now_ms, &case.bpm_limit);
        assert_case(
            case.label,
            generator.sync_state == SyncState::WaitSecondPoint,
            "1点目受信後に WaitSecondPoint へ遷移しない",
        );

        for beat in 1..=case.beats_to_run {
            generator.update(case.interval_ms, &case.bpm_limit);
            now_ms += case.interval_ms as u64;

            let msg = RhythmMessage::new(now_ms, beat, 0, BpmQ8::from_int(case.input_bpm));
            generator.sync(msg, now_ms, &case.bpm_limit);
        }

        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "分周同期後に Locked を維持できない",
        );
        assert_case(
            case.label,
            generator.base_bpm.to_int_round() == case.expected_bpm,
            "base_bpm が期待分周値に一致しない",
        );
        assert_case(
            case.label,
            generator.current_bpm.to_int_round() == case.expected_bpm,
            "current_bpm が期待分周値に一致しない",
        );

        // 90度刻み同期の仕様に合わせ、位相が四半周期グリッド付近に吸着していることを確認する。
        let rem = generator.phase % QUARTER_PHASE;
        let quarter_error = rem.min(QUARTER_PHASE - rem) as i32;
        assert_case(
            case.label,
            quarter_error <= PHASE_ONE_PERCENT,
            "90度刻みグリッドから外れている",
        );
    }
}

// ── 正常系: Phase 3-2 パルス同期（ヒント無し） ────────────────────────────────

#[test]
fn normal_pulse_sync_phase3_2_cases() {
    struct Case {
        label: &'static str,
        start_bpm: u16,
        bpm_limit: BpmLimitParam,
        pulse_sync_param: PulseSyncParam,
        intervals_ms: &'static [u32],
        expected_bpm_min: u16,
        expected_bpm_max: u16,
        expect_quarter_grid_phase: bool,
    }

    let cases = [
        Case {
            label: "step_input_90_to_120",
            start_bpm: 90,
            bpm_limit: BpmLimitParam::new(60, 120),
            pulse_sync_param: PulseSyncParam::new(4, 96),
            intervals_ms: &[
                667, 667, 667, 667, 667, 667, 667, 667, 500, 500, 500, 500, 500, 500, 500, 500,
                500, 500,
            ],
            expected_bpm_min: 116,
            expected_bpm_max: 120,
            expect_quarter_grid_phase: false,
        },
        Case {
            label: "jitter_resilience_100bpm",
            start_bpm: 100,
            bpm_limit: BpmLimitParam::new(60, 120),
            pulse_sync_param: PulseSyncParam::new(4, 80),
            intervals_ms: &[
                560, 625, 585, 635, 600, 575, 620, 595, 640, 580, 610, 590, 630, 570, 615, 600,
            ],
            expected_bpm_min: 97,
            expected_bpm_max: 103,
            expect_quarter_grid_phase: false,
        },
        Case {
            label: "drift_follow_with_ema",
            start_bpm: 100,
            bpm_limit: BpmLimitParam::new(60, 120),
            pulse_sync_param: PulseSyncParam::new(4, 128),
            intervals_ms: &[600, 600, 600, 585, 575, 565, 555, 545, 535, 525],
            expected_bpm_min: 108,
            expected_bpm_max: 116,
            expect_quarter_grid_phase: false,
        },
        Case {
            label: "harmonic_fold_180_to_90",
            start_bpm: 60,
            bpm_limit: BpmLimitParam::new(60, 120),
            pulse_sync_param: PulseSyncParam::new(4, 96),
            intervals_ms: &[333, 333, 333, 333, 333, 333, 333, 333, 333, 333, 333, 333],
            expected_bpm_min: 88,
            expected_bpm_max: 92,
            expect_quarter_grid_phase: true,
        },
    ];

    for case in &cases {
        let mut generator = RhythmGenerator::from_int_bpm(0, case.start_bpm, 12);
        generator.set_pulse_sync_param(case.pulse_sync_param);
        let mut now_ms = 0_u64;

        generator.sync_pulse(now_ms, now_ms, &case.bpm_limit);
        for interval_ms in case.intervals_ms {
            generator.update(*interval_ms, &case.bpm_limit);
            now_ms += *interval_ms as u64;
            generator.sync_pulse(now_ms, now_ms, &case.bpm_limit);
        }

        let bpm_now = generator.current_bpm.to_int_round();

        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "sync_state が Locked になっていない",
        );
        assert_case(
            case.label,
            (case.expected_bpm_min..=case.expected_bpm_max).contains(&bpm_now),
            "推定 BPM が期待範囲外",
        );
        if case.expect_quarter_grid_phase {
            let rem = generator.phase % QUARTER_PHASE;
            let quarter_error = rem.min(QUARTER_PHASE - rem) as i32;
            assert_case(
                case.label,
                quarter_error <= PHASE_ONE_PERCENT,
                "位相が四半周期グリッドに収束していない",
            );
        } else {
            assert_case(
                case.label,
                phase_abs_diff(generator.phase, 0).abs() <= PHASE_ONE_PERCENT,
                "位相誤差が 1% を超えている",
            );
        }
    }
}

// ── 異常系: パルス欠落が閾値周期続いたら WaitSecondPoint へ戻す ─────────────────────

#[test]
fn abnormal_pulse_timeout_to_wait_second_point_cases() {
    struct Case {
        label: &'static str,
        pulse_sync_param: PulseSyncParam,
        expected_interval_ms: u32,
    }

    let cases = [
        Case {
            label: "default_threshold_4cycles",
            pulse_sync_param: PulseSyncParam::new(4, 96),
            expected_interval_ms: 500,
        },
        Case {
            label: "custom_threshold_2cycles",
            pulse_sync_param: PulseSyncParam::new(2, 96),
            expected_interval_ms: 500,
        },
    ];

    for case in &cases {
        let bpm_limit = BpmLimitParam::new(60, 120);
        let mut generator = RhythmGenerator::from_int_bpm(0, 120, 12);
        generator.set_pulse_sync_param(case.pulse_sync_param);
        let mut now_ms = 0_u64;

        // 2点目でロック。
        generator.sync_pulse(now_ms, now_ms, &bpm_limit);
        generator.update(case.expected_interval_ms, &bpm_limit);
        now_ms += case.expected_interval_ms as u64;
        generator.sync_pulse(now_ms, now_ms, &bpm_limit);
        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "2点観測後に Locked へ遷移しない",
        );

        for _ in 0..case
            .pulse_sync_param
            .missing_cycle_threshold
            .saturating_sub(1)
        {
            generator.update(case.expected_interval_ms, &bpm_limit);
            now_ms += case.expected_interval_ms as u64;
            assert_case(
                case.label,
                generator.sync_state == SyncState::Locked,
                "閾値未満で WaitSecondPoint へ遷移してしまった",
            );
        }

        generator.update(case.expected_interval_ms, &bpm_limit);
        now_ms += case.expected_interval_ms as u64;
        assert_case(
            case.label,
            generator.sync_state == SyncState::WaitSecondPoint,
            "パルス欠落が閾値周期続いても WaitSecondPoint へ遷移しない",
        );

        // 欠落後の最初のパルスは再キャリブレーションの1点目として扱う。
        generator.sync_pulse(now_ms, now_ms, &bpm_limit);
        assert_case(
            case.label,
            generator.sync_state == SyncState::WaitSecondPoint,
            "欠落後最初のパルスで即時再ロックしてしまった",
        );

        generator.update(case.expected_interval_ms, &bpm_limit);
        now_ms += case.expected_interval_ms as u64;
        generator.sync_pulse(now_ms, now_ms, &bpm_limit);
        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "再キャリブレーション2点目で Locked に戻れない",
        );
    }
}
