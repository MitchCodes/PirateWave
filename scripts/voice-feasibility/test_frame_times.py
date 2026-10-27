import unittest
from pathlib import Path
from compare_frame_times import metrics,compare,validate_capture_paths


class FrameComparison(unittest.TestCase):
    def test_reusing_captures_cannot_claim_repeated_runs(self):
        with self.assertRaises(ValueError):validate_capture_paths([Path('off.csv')]*3,[Path('on.csv')]*3)
        with self.assertRaises(ValueError):validate_capture_paths([Path('same.csv')],[Path('same.csv')])
        validate_capture_paths([Path(f'off{i}.csv') for i in range(3)],[Path(f'on{i}.csv') for i in range(3)])
    def test_accepts_small_change_and_rejects_material_regression(self):
        baseline=[metrics([10.0]*1000) for _ in range(3)]
        self.assertTrue(compare(baseline,[metrics([10.4]*1000) for _ in range(3)])['pass'])
        self.assertFalse(compare(baseline,[metrics([11.5]*1000) for _ in range(3)])['pass'])

    def test_noisy_baseline_is_not_approval(self):
        baseline=[metrics([v]*1000) for v in (8,10,12)]
        result=compare(baseline,[metrics([9]*1000) for _ in range(3)])
        self.assertTrue(result['inconclusive'])
        self.assertFalse(result['pass'])

    def test_invalid_or_too_short_capture_is_rejected(self):
        for values in ([10]*999,[float('nan')]*1000,[0]*1000):
            with self.assertRaises(ValueError):metrics(values)


if __name__=='__main__':unittest.main()
