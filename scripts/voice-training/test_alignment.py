import unittest
import numpy as np
from check_pair_alignment import lag_estimate


class AlignmentTests(unittest.TestCase):
    def test_detects_known_positive_and_negative_delays(self):
        source = np.random.default_rng(42).normal(size=(3, 400))
        for lag in (-12, 0, 8):
            converted = np.zeros_like(source)
            if lag > 0:
                converted[:, lag:] = source[:, :-lag]
            elif lag < 0:
                converted[:, :lag] = source[:, -lag:]
            else:
                converted[:] = source
            result = lag_estimate(source, converted)
            self.assertTrue(result['usable'])
            self.assertEqual(result['converted_delay_ms'], lag * 5)
            self.assertAlmostEqual(result['peak_correlation'], 1)

    def test_silence_has_no_timing_evidence(self):
        self.assertFalse(lag_estimate(np.zeros((3,400)), np.zeros((3,400)))['usable'])


if __name__ == '__main__':
    unittest.main()
