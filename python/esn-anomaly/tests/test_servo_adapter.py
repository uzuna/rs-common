"""ServoStreamAdapter / CommandScheduler のテスト。"""

from __future__ import annotations

import numpy as np
import pytest

from esn_anomaly.model import ESNConfig
from esn_anomaly.servo.adapter import CommandScheduler, ServoStreamAdapter, StepResult
from esn_anomaly.servo.command_test import (
    NOISE,
    SEED,
    WARMUP,
    _build_sc,
    _make_periodic_profile,
)
from esn_anomaly.servo.data import (
    CommandProfile,
    CommandSegment,
    generate_servo_with_profile,
)


TRAIN_CYCLES = 10


def _make_train_data():
    rng = np.random.default_rng(SEED)
    profile = _make_periodic_profile(TRAIN_CYCLES)
    u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)
    return u_raw, cmd_raw


def _build_adapter():
    u_raw_train, cmd_raw_train = _make_train_data()
    return ServoStreamAdapter.from_training_data(
        u_raw_train,
        cmd_raw_train,
        esn_config=ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED),
        warmup=WARMUP,
    )


# ------------------------------------------------------------------
# StepResult
# ------------------------------------------------------------------

class TestStepResult:
    def test_csv_header_length(self):
        assert len(StepResult.csv_header()) == 10

    def test_to_row_length(self):
        r = StepResult(
            ts=0.01, cmd_mrad=100.0, pos_mrad=50.0, load_mA=20.0,
            esn_res_pos=0.01, esn_res_load=0.02, esn_anomaly=False,
            phys_residual=5.0, phys_is_steady=True, phys_anomaly=False,
        )
        assert len(r.to_row()) == 10

    def test_to_row_bool_as_int(self):
        r = StepResult(
            ts=0.0, cmd_mrad=0.0, pos_mrad=0.0, load_mA=0.0,
            esn_res_pos=0.0, esn_res_load=0.0, esn_anomaly=True,
            phys_residual=0.0, phys_is_steady=False, phys_anomaly=True,
        )
        row = r.to_row()
        assert row[6] == 1   # esn_anomaly
        assert row[8] == 0   # phys_is_steady
        assert row[9] == 1   # phys_anomaly


# ------------------------------------------------------------------
# CommandScheduler
# ------------------------------------------------------------------

class TestCommandScheduler:
    def _simple_profile(self):
        return CommandProfile([
            CommandSegment("hold", 0.0, 100),
            CommandSegment("ramp", 500.0, 50),
            CommandSegment("hold", 500.0, 100),
        ])

    def test_length(self):
        sch = CommandScheduler.from_profile(self._simple_profile())
        assert len(sch) == 250

    def test_iteration_exhausts(self):
        sch = CommandScheduler.from_profile(self._simple_profile())
        cmds = list(sch)
        assert len(cmds) == 250
        assert sch.done

    def test_remaining_decrements(self):
        sch = CommandScheduler.from_profile(self._simple_profile())
        total = len(sch)
        for i, _ in enumerate(sch):
            assert sch.remaining == total - i - 1

    def test_reset(self):
        sch = CommandScheduler.from_profile(self._simple_profile())
        list(sch)
        assert sch.done
        sch.reset()
        assert not sch.done
        assert sch.remaining == 250

    def test_peek_does_not_advance(self):
        sch = CommandScheduler.from_profile(self._simple_profile())
        first = sch.peek()
        assert first is not None
        assert sch.current_index == 0
        cmd = next(sch)
        assert cmd == first
        assert sch.current_index == 1

    def test_peek_returns_none_when_done(self):
        sch = CommandScheduler.from_array(np.array([1.0, 2.0]))
        next(sch)
        next(sch)
        assert sch.peek() is None

    def test_stop_iteration(self):
        sch = CommandScheduler.from_array(np.array([1.0, 2.0]))
        next(sch)
        next(sch)
        with pytest.raises(StopIteration):
            next(sch)

    def test_hold_segment_value(self):
        """hold セグメントは指定値を繰り返す。"""
        sch = CommandScheduler.from_profile(
            CommandProfile([CommandSegment("hold", 300.0, 5)])
        )
        cmds = list(sch)
        assert all(c == pytest.approx(300.0) for c in cmds)

    def test_ramp_segment_monotone(self):
        """ramp セグメントは単調増加する。"""
        sch = CommandScheduler.from_profile(
            CommandProfile([
                CommandSegment("hold", 0.0, 1),
                CommandSegment("ramp", 500.0, 10),
            ])
        )
        cmds = list(sch)
        ramp_part = cmds[1:]
        diffs = [ramp_part[i + 1] - ramp_part[i] for i in range(len(ramp_part) - 1)]
        assert all(d > 0 for d in diffs)


# ------------------------------------------------------------------
# ServoStreamAdapter: 基本動作
# ------------------------------------------------------------------

class TestServoStreamAdapterBasic:
    def test_on_measurement_returns_step_result(self):
        adapter = _build_adapter()
        u_raw, cmd_raw = _make_train_data()
        result = adapter.on_measurement(0.01, u_raw[0, 0], u_raw[0, 1])
        assert isinstance(result, StepResult)

    def test_history_accumulates(self):
        adapter = _build_adapter()
        u_raw, cmd_raw = _make_train_data()
        for i in range(5):
            adapter.on_measurement(i * 0.01, u_raw[i, 0], u_raw[i, 1])
        assert len(adapter.history) == 5

    def test_history_maxlen(self):
        u_raw_train, cmd_raw_train = _make_train_data()
        adapter = ServoStreamAdapter.from_training_data(
            u_raw_train, cmd_raw_train, history_maxlen=3, warmup=WARMUP,
        )
        u_raw, cmd_raw = _make_train_data()
        for i in range(10):
            adapter.on_measurement(i * 0.01, u_raw[i, 0], u_raw[i, 1])
        assert len(adapter.history) == 3

    def test_pop_history_clears(self):
        adapter = _build_adapter()
        u_raw, cmd_raw = _make_train_data()
        for i in range(5):
            adapter.on_measurement(i * 0.01, u_raw[i, 0], u_raw[i, 1])
        popped = adapter.pop_history()
        assert len(popped) == 5
        assert len(adapter.history) == 0

    def test_update_cmd_is_reflected(self):
        adapter = _build_adapter()
        adapter.update_cmd(123.4)
        u_raw, cmd_raw = _make_train_data()
        result = adapter.on_measurement(0.0, u_raw[0, 0], u_raw[0, 1])
        assert result.cmd_mrad == pytest.approx(123.4)

    def test_reset_clears_history_when_requested(self):
        adapter = _build_adapter()
        u_raw, cmd_raw = _make_train_data()
        adapter.on_measurement(0.0, u_raw[0, 0], u_raw[0, 1])
        adapter.reset(clear_history=True)
        assert len(adapter.history) == 0

    def test_reset_keeps_history_by_default(self):
        adapter = _build_adapter()
        u_raw, cmd_raw = _make_train_data()
        adapter.on_measurement(0.0, u_raw[0, 0], u_raw[0, 1])
        adapter.reset()
        assert len(adapter.history) == 1

    def test_step_result_ts_stored(self):
        adapter = _build_adapter()
        u_raw, _ = _make_train_data()
        result = adapter.on_measurement(42.0, u_raw[0, 0], u_raw[0, 1])
        assert result.ts == pytest.approx(42.0)


# ------------------------------------------------------------------
# ServoStreamAdapter: 検知性能
# ------------------------------------------------------------------

class TestServoStreamAdapterDetection:
    def test_low_false_alarm_on_normal(self):
        adapter = _build_adapter()

        rng = np.random.default_rng(SEED + 1)
        profile = _make_periodic_profile(5)
        u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)

        for i in range(len(u_raw)):
            adapter.update_cmd(cmd_raw[i])
            adapter.on_measurement(i * 0.01, u_raw[i, 0], u_raw[i, 1])

        flags = [r.phys_anomaly for r in adapter.history[WARMUP:]]
        rate = np.mean(flags) * 100
        assert rate < 20.0, f"物理 誤警報率 {rate:.1f}% >= 20%"

    def test_detects_external_force_via_scheduler(self):
        """CommandScheduler + adapter でストリーミング検知が機能する。"""
        adapter = _build_adapter()

        rng = np.random.default_rng(SEED + 100)
        u_raw, cmd_raw, disturbances = _build_sc(rng)

        # CommandScheduler を使ってコマンド送信をシミュレート
        scheduler = CommandScheduler.from_array(cmd_raw)
        for i, cmd in enumerate(scheduler):
            adapter.update_cmd(cmd)
            adapter.on_measurement(i * 0.01, u_raw[i, 0], u_raw[i, 1])

        phys_flags = np.array([r.phys_anomaly for r in adapter.history])
        n = len(phys_flags)
        rates = []
        for d in disturbances:
            lo = max(WARMUP, d.start)
            hi = min(n, d.end)
            if lo < hi:
                rates.append(phys_flags[lo:hi].mean() * 100)

        if rates:
            mean_rate = np.mean(rates)
            assert mean_rate > 30.0, f"外乱区間の物理検知率 {mean_rate:.1f}% <= 30%"


# ------------------------------------------------------------------
# ServoStreamAdapter: CSV 保存
# ------------------------------------------------------------------

class TestServoStreamAdapterCsv:
    def test_save_csv(self, tmp_path):
        adapter = _build_adapter()
        u_raw, cmd_raw = _make_train_data()
        for i in range(5):
            adapter.update_cmd(cmd_raw[i])
            adapter.on_measurement(i * 0.01, u_raw[i, 0], u_raw[i, 1])

        out = tmp_path / "result.csv"
        adapter.save_csv(out)
        assert out.exists()

        lines = out.read_text().splitlines()
        assert lines[0] == ",".join(StepResult.csv_header())
        assert len(lines) == 6  # header + 5 rows
