use rhythm_core::{fixed_math::BpmQ8, RhythmGenerator, RhythmMessage, SyncState};

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
        updates_ms: &'static [u32],
        expected_beats: u64,
        expected_phase: u16,
        max_phase_error: i32,
    }

    let cases = [
        Case {
            label: "120bpm_500ms_1beat",
            bpm: 120,
            updates_ms: &[500],
            expected_beats: 1,
            expected_phase: 0,
            max_phase_error: 0,
        },
        Case {
            label: "120bpm_250ms_x2",
            bpm: 120,
            updates_ms: &[250, 250],
            expected_beats: 1,
            expected_phase: 0,
            max_phase_error: 0,
        },
        Case {
            label: "90bpm_2000ms_3beat",
            bpm: 90,
            updates_ms: &[2_000],
            expected_beats: 3,
            expected_phase: 0,
            max_phase_error: 0,
        },
    ];

    for case in &cases {
        let mut generator = RhythmGenerator::from_int_bpm(0, case.bpm, 12);
        for dt_ms in case.updates_ms {
            generator.update(*dt_ms);
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

// ── 正常系: 低頻度入力で 2点観測ロックし、位相誤差 1% 以内へ収束 ───────────────────

#[test]
fn normal_low_frequency_lock_cases() {
    struct Case {
        label: &'static str,
        start_phase: u16,
        start_bpm: u16,
        reference_bpm: u16,
        beats_to_run: u32,
    }

    let cases = [
        Case {
            label: "start_phase_offset_90bpm",
            start_phase: 24_000,
            start_bpm: 120,
            reference_bpm: 90,
            beats_to_run: 8,
        },
        Case {
            label: "start_quarter_offset_90bpm",
            start_phase: QUARTER_PHASE,
            start_bpm: 105,
            reference_bpm: 90,
            beats_to_run: 8,
        },
    ];

    for case in &cases {
        let mut generator = RhythmGenerator::from_int_bpm(case.start_phase, case.start_bpm, 12);
        let mut now_ms = 0_u64;

        let first = RhythmMessage::new(now_ms, 0, 0, BpmQ8::from_int(case.reference_bpm));
        generator.sync(first, now_ms);
        assert_case(
            case.label,
            generator.sync_state == SyncState::WaitSecondPoint,
            "1点目受信後に WaitSecondPoint へ遷移しない",
        );

        for beat in 1..=case.beats_to_run {
            let interval_ms = 60_000_u32 / case.reference_bpm as u32;
            generator.update(interval_ms);
            now_ms += interval_ms as u64;

            let msg = RhythmMessage::new(now_ms, beat, 0, BpmQ8::from_int(case.reference_bpm));
            generator.sync(msg, now_ms);
        }

        assert_case(
            case.label,
            generator.sync_state == SyncState::Locked,
            "2点観測後に Locked へ遷移しない",
        );
        assert_case(
            case.label,
            generator.base_bpm.to_int_round() == case.reference_bpm,
            "base_bpm が参照 BPM に収束しない",
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
        expected_bpm: u16,
        interval_ms: u32,
        beats_to_run: u32,
    }

    let cases = [Case {
        label: "180_to_90_fold",
        input_bpm: 180,
        expected_bpm: 90,
        interval_ms: 333,
        beats_to_run: 10,
    }];

    for case in &cases {
        let mut generator = RhythmGenerator::from_int_bpm(0, 60, 12);
        let mut now_ms = 0_u64;

        let first = RhythmMessage::new(now_ms, 0, 0, BpmQ8::from_int(case.input_bpm));
        generator.sync(first, now_ms);
        assert_case(
            case.label,
            generator.sync_state == SyncState::WaitSecondPoint,
            "1点目受信後に WaitSecondPoint へ遷移しない",
        );

        for beat in 1..=case.beats_to_run {
            generator.update(case.interval_ms);
            now_ms += case.interval_ms as u64;

            let msg = RhythmMessage::new(now_ms, beat, 0, BpmQ8::from_int(case.input_bpm));
            generator.sync(msg, now_ms);
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
