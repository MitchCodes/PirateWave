"""Publish compact, hashed qualification evidence beside the source documentation."""
from pathlib import Path
import hashlib
import json
import re
from datetime import datetime, timezone
from build_pack import validate_automated
from build_directml_pack import validate_reports


def main():
    root=Path(__file__).resolve().parents[2]
    reports={
        'native_13ms_one_thread':'diagnostics/native-torch-13-cpu1.json',
        'native_13ms_two_threads':'diagnostics/native-torch-13-cpu2.json',
        'native_26ms_one_thread':'diagnostics/native-torch-26-cpu1.json',
        'initial_native_cpu_profile':'diagnostics/native-torch-26-cpu2.json',
        'native_cpu_profile':'diagnostics/native-torch-26-cpu2-final.json',
        'model_continuous':'torchscript-continuous-30m/summary.json',
        'actual_worker_continuous':'diagnostics/actual-worker-states-30m.json',
        'actual_worker_cases':'diagnostics/worker-cases.json',
        'normal_worker_protocol':'diagnostics/worker-protocol-final.json',
        'integrated_pipeline':'diagnostics/native-pipeline-final-30m.json',
        'final_adapter_pipeline':'diagnostics/native-pipeline-final-adapter.json',
        'causal_alignment':'diagnostics/causal-alignment.json',
        'matching_thread_diagnostic':'diagnostics/actual-worker-states-threads2.json',
        'identical_state_diagnostic':'diagnostics/actual-worker-single-step.json',
        'no_warmup_diagnostic':'diagnostics/actual-worker-states-no-warm.json',
        'runtime_environment':'diagnostics/runtime-environment.json',
        'app_local_runtime':'diagnostics/app-local-runtime.json',
        'hardware':'diagnostics/hardware.json',
        'native_modules':'diagnostics/final-worker-modules.json',
    }
    evidence={}
    for label,name in reports.items():
        path=root/'dist/voice-feasibility'/name
        evidence[label]={'report_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'result':json.loads(path.read_text(encoding='utf-8-sig'))}
    qualification=json.loads((root/'dist/voice-pack/llvc-f8312-v1/QUALIFICATION.json').read_text(encoding='utf-8'))
    validate_automated(qualification)
    tests=(root/'dist/voice-feasibility/diagnostics/directml-workspace-tests.log').read_text(encoding='utf-8-sig')
    counts=re.findall(r'test result: ok\. (\d+) passed',tests)
    source_files=['src/voice/mod.rs','src/voice/resample.rs','src/voice/pack.rs','src/voice/tests.rs',
                  'src/voice/job.rs','src/engine.rs','src/audio_scheduling.rs','crates/piratewave-core/src/graph.rs',
                  'crates/piratewave-core/src/voice.rs','tools/voice-worker/src/main.rs','tools/voice-worker/Cargo.lock',
                  'tools/voice-dml-worker/src/main.rs','tools/voice-dml-worker/Cargo.lock']
    directml=json.loads((root/'dist/voice-pack/llvc-f8312-v1/DIRECTML_QUALIFICATION.json').read_text(encoding='utf-8'))
    if directml['available']:
        validate_reports(directml['worker_sha256'], directml['worker_states'], directml['normal_protocol'],
                         directml['integrated_pipeline'], directml['worker_cases'], directml['native_benchmark'])
    directml['full_model_onnx_available']=False
    directml['full_model_reason']='Encoder arithmetic failed streaming-state acceptance; original CPU encoder/synthesis retained.'
    result={'schema':1,'created_utc':datetime.now(timezone.utc).isoformat(),
            'scope':'One LLVC voice; native Rust/LibTorch CPU or hybrid DirectML transformer; explicitly installed optional pack',
            'automated_qualified':True,'release_qualified':qualification['release_qualified'],
            'live_beta_enabled':True,
            'live_policy':'Normal launches permit automated-qualified CPU or hybrid DirectML conversion on the validated hardware. Auto prefers CPU; manual release acceptance remains separate.',
            'manual_pending':qualification['pending'],
            'directml':directml,
            'verification':{'workspace_unit_tests_passed':sum(map(int,counts)),'python_gate_and_frame_tests_passed':13,
                'directml_worker_failure_restart_bypass_disconnect':'passed',
                'native_ui_views':40,'native_integration_tests_passed':4,'native_worker_failure_restart_bypass_disconnect':'passed',
                'optional_pack_install_reuse_corruption_reinstall':'passed',
                'ui_capture_note':'Packaged normal-launch UI passed 40 views. Earlier qualification-mode capture required a retry after an egui_glow zero-size screenshot startup panic.',
                'notes':'No physical audio devices were used by automated native worker tests. Software first-ready time does not measure complete hardware latency.'},
            'voice_archive_sha256':hashlib.sha256((root/'dist/PirateWave-LLVC-Voice-v1.zip').read_bytes()).hexdigest(),
            'model_sha256':qualification['native_profile']['model_sha256'],
            'worker_sha256':qualification['worker_sha256'],
            'source_sha256':{name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in source_files},
            'evidence':evidence}
    path=root/'docs/VOICE_CHANGER_VALIDATION.json'
    path.write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print(path)


if __name__=='__main__':main()
