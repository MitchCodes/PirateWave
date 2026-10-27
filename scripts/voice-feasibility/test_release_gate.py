import copy
import unittest
from build_pack import MODEL_SHA,validate_acceptance,validate_automated


class ReleaseGate(unittest.TestCase):
    def setUp(self):
        self.q={'runtime_libraries':{'crt':'sha'},'environment':{'runtime_libraries':{'crt':'sha'},'cpu_identifier':'test-cpu'},'continuous':{'pass':True,'audio_seconds':1800,'model_sha256':MODEL_SHA},
                'integrated_pipeline':{'pass':True,'duration_seconds':1800,'underruns':0,'model_sha256':MODEL_SHA,'worker_sha256':'worker'},
                'worker_sha256':'worker',
                'causal_alignment':{'pass':True},
                'app_local_runtime':{'pass':True,'app_local_crt':True,'python_modules':[],'worker_sha256':'worker'},
                'normal_protocol':{'pass':True,'worker_sha256':'worker'},
                'worker_cases':{'pass':True,'reports':[{'pass':True,'worker_sha256':'worker','model_sha256':MODEL_SHA} for _ in range(28)]},
                'worker_states':{'pass':True,'audio_seconds':1800,'model_sha256':MODEL_SHA,'worker_sha256':'worker'},
                'native_profile':{'cpu_identifier':'test-cpu','parity_pass':True,'timing_pass':True,'model_sha256':MODEL_SHA}}
        self.a={'schema':1,'model_sha256':MODEL_SHA,'cpu_identifier':'test-cpu',
                'listening':{'accepted':True,'notes':'Reviewed all speech cases'},
                'clean_windows':{'accepted':True,'notes':'Test VM'},
                'latency':{'accepted':True,'added_ms':90,'microphone_to_output_ms':180,'method':'Measured loopback'}}
        self.g={'pass':True,'inconclusive':False,'cpu_identifier':'test-cpu'}
    def test_model_parity_does_not_replace_human_acceptance(self):
        for field in ['listening','clean_windows','latency']:
            record=copy.deepcopy(self.a);record[field]['accepted']=False
            with self.assertRaises(ValueError):validate_acceptance(self.q,record,self.g,'test-cpu')
    def test_bad_latency_unstable_runtime_and_noisy_game_are_rejected(self):
        for value in [101,float('nan'),float('inf')]:
            record=copy.deepcopy(self.a);record['latency']['added_ms']=value
            with self.assertRaises(ValueError):validate_acceptance(self.q,record,self.g,'test-cpu')
        bad=copy.deepcopy(self.q);bad['integrated_pipeline']['underruns']=1
        with self.assertRaises(ValueError):validate_acceptance(bad,self.a,self.g,'test-cpu')
        bad=dict(self.g,inconclusive=True)
        with self.assertRaises(ValueError):validate_acceptance(self.q,self.a,bad,'test-cpu')
    def test_complete_attested_record_is_required(self):
        self.assertTrue(validate_acceptance(self.q,self.a,self.g,'test-cpu'))
        with self.assertRaises(ValueError):validate_acceptance(self.q,self.a,self.g,'different-cpu')

    def test_qualification_mode_requires_current_passing_automated_evidence(self):
        self.assertTrue(validate_automated(self.q))
        for section,key,value in [('worker_states','worker_sha256','old-worker'),
                                  ('worker_states','audio_seconds',1799),
                                  ('worker_states','pass',False),
                                  ('native_profile','cpu_identifier','other-cpu'),
                                  ('integrated_pipeline','worker_sha256','old-worker'),
                                  ('causal_alignment','pass',False)]:
            q=copy.deepcopy(self.q);q[section][key]=value
            with self.assertRaises(ValueError):validate_automated(q)
        q=copy.deepcopy(self.q);q['runtime_libraries']['crt']='different-dll'
        with self.assertRaises(ValueError):validate_automated(q)


if __name__=='__main__':unittest.main()
