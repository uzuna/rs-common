import unittest

from py_rhythm_core import (
    BPM_Q8_ONE,
    BpmLimitParam,
    RhythmGenerator,
    RhythmMessage,
    bpm_q8_from_int,
    bpm_q8_to_float,
)


class TestRhythmCorePyBindings(unittest.TestCase):
    def test_bpm_helpers(self) -> None:
        raw = bpm_q8_from_int(120)
        self.assertEqual(raw, 120 * BPM_Q8_ONE)
        self.assertAlmostEqual(bpm_q8_to_float(raw), 120.0, places=6)

    def test_rhythm_message_roundtrip(self) -> None:
        original = RhythmMessage(
            timestamp_ms=1234,
            beat_count=7,
            phase=4321,
            bpm_raw=bpm_q8_from_int(90),
        )
        payload = original.to_wire_bytes()
        restored = RhythmMessage.from_wire_slice(payload)

        self.assertEqual(restored.timestamp_ms, original.timestamp_ms)
        self.assertEqual(restored.beat_count, original.beat_count)
        self.assertEqual(restored.phase, original.phase)
        self.assertEqual(restored.bpm_raw, original.bpm_raw)

    def test_generator_update_and_message(self) -> None:
        generator = RhythmGenerator.from_int_bpm(phase=0, bpm=90, coupling_divisor=16)
        bpm_limit = BpmLimitParam(60, 120)

        before_phase = generator.phase
        generator.update(dt_ms=100, bpm_limit_param=bpm_limit)
        self.assertNotEqual(generator.phase, before_phase)

        message = generator.to_message(now_ms=2000)
        self.assertGreaterEqual(message.bpm_raw, bpm_q8_from_int(60))
        self.assertLessEqual(message.bpm_raw, bpm_q8_from_int(120))


if __name__ == "__main__":
    unittest.main()
