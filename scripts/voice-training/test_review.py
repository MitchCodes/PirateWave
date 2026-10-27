import json
from pathlib import Path
import tempfile
import unittest

import numpy as np
import soundfile as sf

from review_stages import build
from review_duration import build as compare


class StageReviewTests(unittest.TestCase):
    def test_context_comparison_rejects_changed_data_and_unequal_budgets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            a, b = root / 'a', root / 'b'
            reports = []
            for folder, context in ((a, 10240), (b, 65536)):
                (folder / 'review').mkdir(parents=True)
                (folder / 'native').mkdir()
                report = {'llvc': {'optimizer_steps': 1000}, 'pairs': {'count': 1},
                          'recipe': {'speaker': '1462', 'llvc_context_samples': context}}
                reports.append(report)
                (folder / 'report.json').write_text(json.dumps(report))
                (folder / 'native/report.json').write_text(json.dumps({'waveform_and_state_pass': True}))
                for path in ['pair-0-original.wav', 'pair-0-converted.wav', 'native/native-converted.wav',
                             'review/target-reference-0.wav',
                             *[f'review/{i}-{kind}.wav' for i in (3, 4, 5)
                               for kind in ('original', 'teacher', 'llvc-offline')]]:
                    sf.write(folder / path, np.zeros(160), 16000)
            compare(a, b, root / 'comparison', 'context')
            with self.assertRaisesRegex(ValueError, 'generator context'):
                compare(a, b, root / 'comparison', 'duration')
            reports[1]['llvc']['optimizer_steps'] = 500
            (b / 'report.json').write_text(json.dumps(reports[1]))
            with self.assertRaisesRegex(ValueError, 'equal update budgets'):
                compare(a, b, root / 'comparison', 'context')
            reports[1]['llvc']['optimizer_steps'] = 1000
            (b / 'report.json').write_text(json.dumps(reports[1]))
            sf.write(b / 'pair-0-converted.wav', np.ones(160), 16000)
            with self.assertRaisesRegex(ValueError, 'Paired training audio changed'):
                compare(a, b, root / 'comparison', 'context')

    def test_native_excerpts_follow_original_lengths_and_half_second_tails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'input'
            output = Path(directory) / 'review'
            for speaker in ('1462', '1272', '1673'):
                review = root / speaker / 'review'
                native = root / speaker / 'native'
                review.mkdir(parents=True)
                native.mkdir()
                (native / 'report.json').write_text(json.dumps({'waveform_and_state_pass': True}))
                sf.write(review / 'target-reference-0.wav', np.zeros(160), 16000)
                blocks = []
                for index in (3, 4, 5):
                    for kind in ('original', 'teacher', 'llvc-offline'):
                        sf.write(review / f'{index}-{kind}.wav', np.zeros(160 * index), 16000)
                    blocks.append(np.full(160 * index + 8000, index / 10))
                sf.write(native / 'native-converted.wav', np.concatenate(blocks), 16000)
            page = build(root, output)
            self.assertTrue(page.exists())
            evidence = json.loads((output / 'evidence.json').read_text())
            self.assertEqual(len(evidence['rows']), 9)
            self.assertFalse(evidence['release_accepted'])
            for row in evidence['rows']:
                data, rate = sf.read(output / f"{row['target']}-{row['source_index']}-native.wav")
                self.assertEqual(rate, 16000)
                self.assertEqual(len(data), row['source_index'] * 160 + 8000)
                self.assertTrue(np.allclose(data, row['source_index'] / 10, atol=1/32768))


if __name__ == '__main__':
    unittest.main()
