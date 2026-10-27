"""Summarize completed portable workflows without publishing recordings or paths."""
import argparse
import json
import os
from pathlib import Path
from assets import sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    summary = dict(schema=1, release='0.2.0-beta.1', scope='Experimental local Voice Creation Beta',
                   quality_improved=False, clean_windows_qualified=False,
                   custom_voice_thirty_minute_qualified=False, gaming_impact_qualified=False,
                   fixture='Three licensed LibriSpeech target takes, separate train/validation/test roles; execution test, not listening acceptance.',
                   backends={})
    for backend in ('cpu', 'cuda'):
        workflow = json.loads((args.workspace/f'release-{backend}.json').read_text(encoding='utf-8'))
        if not workflow['import_training_export_install_pass'] or not workflow['live_pipeline']['pass']:
            raise ValueError('Incomplete portable workflow: '+backend)
        folder = Path(workflow.pop('job'))
        workflow.pop('voice', None)
        training = json.loads((folder/'report.json').read_text(encoding='utf-8'))
        job = json.loads((folder/'job.json').read_text(encoding='utf-8'))
        native = json.loads((folder/'native/report.json').read_text(encoding='utf-8'))
        if not native['waveform_and_state_pass'] or not native['timing_pass']:
            raise ValueError('Native validation failed: '+backend)
        rows = native.pop('rows', [])
        native.pop('public_training_enabled', None)
        native['worst_native_errors'] = {
            tensor: {metric:max((row.get('native', {}).get(tensor, {}).get(metric, 0.) for row in rows), default=0.)
                     for metric in ('max', 'rms', 'normalized_max', 'normalized_rms')}
            for tensor in ('audio', 'encoder', 'decoder', 'synthesis', 'prenet')}
        stages = {}
        for stage in ('teacher', 'llvc'):
            stages[stage] = {k:v for k,v in training[stage].items() if k!='losses'}
        evidence = dict(workflow, native=native, stages=stages,
                        python=training['execution']['python'], torch=training['torch'],
                        gpu=training.get('gpu'), gpu_peak_allocated_bytes=training.get('gpu_peak_allocated_bytes'),
                        threads=training['threads'], dataset_sha256=training['dataset_sha256'],
                        preparation_and_training_elapsed_seconds=training.get('elapsed_seconds'),
                        source_pairs=training['pairs'],
                        recipe_identity=job['recipe'],
                        cpu_identifier=os.environ.get('PROCESSOR_IDENTIFIER'),
                        logical_processors=os.environ.get('NUMBER_OF_PROCESSORS'),
                        installed_runtime_manifest_sha256=sha256(Path(job['runtime'])/'runtime-manifest.json'),
                        shared_manifest_sha256=sha256(Path(job['shared'])/'runtime-manifest.json'),
                        cpu_reference_estimate_note='Mean update cost excludes preparation, checkpoint I/O, export and any paused time; this is not a promised completion time.')
        pause = args.workspace/f'pause-{backend}.json'
        if pause.is_file():
            evidence['application_pause_control'] = json.loads(pause.read_text(encoding='utf-8'))
        (args.out/f'validation-{backend}.json').write_text(json.dumps(evidence, indent=2)+'\n', encoding='utf-8')
        summary['backends'][backend] = evidence
    (args.out/'VOICE_TRAINING_BETA_VALIDATION.json').write_text(json.dumps(summary, indent=2)+'\n', encoding='utf-8')
    print('Wrote CPU/CUDA portable workflow evidence; full release qualification remains false.')


if __name__ == '__main__':
    main()
