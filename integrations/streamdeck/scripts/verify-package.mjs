import { execFileSync } from 'node:child_process';
// Use the same standard ZIP reader as release verification. Do not extract or
// execute package contents, and reject development files and traversal paths.
const program = `import json,sys,zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
 assert z.testzip() is None, 'Corrupt Stream Deck package'
 names=z.namelist()
 assert not any('..' in n.split('/') or n.startswith('/') or any(p in n for p in ('node_modules/', '.env', '.log', '.map', 'test/')) for n in names), 'Unexpected package file'
 manifests=[n for n in names if n.endswith('/manifest.json') or n=='manifest.json']
 assert len(manifests)==1, 'Missing or duplicate manifest'
 m=json.loads(z.read(manifests[0])); root=manifests[0][:-len('manifest.json')]
 allowed={'manifest.json','LICENSE','THIRD_PARTY_NOTICES.txt','bin/plugin.mjs','ui/index.html','ui/inspector.mjs','ui/model.mjs','ui/style.css'}
 allowed.update('images/'+name+'.png' for name in ('plugin','plugin@2x','category','category@2x','action','action@2x','key','key@2x'))
 assert all(n.startswith(root) and n[len(root):] in allowed for n in names if not n.endswith('/')), 'File outside package allowlist'
 assert m['UUID']=='com.piratewave.streamdeck' and m['SDKVersion']==2
 assert m['Nodejs']['Version']=='24' and m['Software']['MinimumVersion']=='7.1'
 assert len(m['Actions'])==12 and len({a['UUID'] for a in m['Actions']})==12
 for f in ('bin/plugin.mjs','ui/index.html','ui/inspector.mjs','ui/model.mjs','ui/style.css','images/key.png','LICENSE','THIRD_PARTY_NOTICES.txt'):
  assert root+f in names, 'Missing '+f
 for a in m['Actions']:
  assert a['DisableAutomaticStates'] and root+a['PropertyInspectorPath'] in names
 print('Verified Stream Deck installer: manifest, backend, property inspector, controls and icons')
`;
execFileSync('python', ['-c', program, process.argv[2]], { stdio: 'inherit', windowsHide: true });
