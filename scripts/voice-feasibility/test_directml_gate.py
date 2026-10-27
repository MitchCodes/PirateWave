import copy
import unittest
from build_directml_pack import validate_reports


class DirectMlGateTests(unittest.TestCase):
    def setUp(self):
        base = {'pass':True,'worker_sha256':'qualified-worker'}
        self.reports = [dict(base,audio_seconds=1800,one_second_windows=1800),
            dict(base,cases=[{'name':str(i),'pass':True} for i in range(14)]),
            dict(base,duration_seconds=1800,underruns=0,first_ready_ms=89,backend='DirectMl'),
            dict(base,reports=[dict(base,case=str(i),single_step=single) for i in range(14) for single in [False,True]]),
            dict(base,mean_ms=9.2,p99_ms=14.3,chunks=1600)]

    def check(self, reports=None):
        validate_reports('qualified-worker', *(reports or self.reports))

    def test_complete_matching_evidence_passes(self):
        self.check()

    def test_failed_or_different_worker_is_rejected(self):
        for index in range(5):
            for change in [{'pass':False},{'worker_sha256':'another-worker'}]:
                reports = copy.deepcopy(self.reports)
                reports[index].update(change)
                with self.assertRaises(ValueError):
                    self.check(reports)

    def test_underruns_latency_or_short_continuous_test_is_rejected(self):
        for index, change in [(0,{'audio_seconds':1799}),(0,{'one_second_windows':1799}),
            (2,{'duration_seconds':1799}),(2,{'underruns':1}),(2,{'first_ready_ms':101}),(2,{'backend':'Cpu'})]:
            reports = copy.deepcopy(self.reports)
            reports[index].update(change)
            with self.assertRaises(ValueError):
                self.check(reports)

    def test_duplicate_or_failed_cases_are_rejected(self):
        for index, key in [(1,'cases'),(3,'reports')]:
            reports = copy.deepcopy(self.reports)
            reports[index][key][1] = copy.deepcopy(reports[index][key][0])
            with self.assertRaises(ValueError):
                self.check(reports)

    def test_insufficient_timing_headroom_is_rejected(self):
        for change in [{'mean_ms':13.01},{'p99_ms':26},{'chunks':1599}]:
            reports = copy.deepcopy(self.reports)
            reports[4].update(change)
            with self.assertRaises(ValueError):
                self.check(reports)


if __name__ == '__main__':
    unittest.main()
