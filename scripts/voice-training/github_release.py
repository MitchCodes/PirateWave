"""Stage/finish PirateWave's Beta release using Git's existing noninteractive credential.

Tokens stay in memory and are never printed or written. Only this repository's
explicit Beta tag is writable; published releases are never overwritten.
"""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import subprocess
import urllib.parse
import urllib.request

REPO=os.environ.get('RELEASES_REPOSITORY', '')
TAG='v0.2.0-beta.1'


def main():
    if not REPO or REPO.count("/") != 1:
        raise ValueError("Set RELEASES_REPOSITORY explicitly before publishing")
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--folder',type=Path,required=True)
    p.add_argument('--body',type=Path,required=True)
    p.add_argument('--commit',default='main')
    p.add_argument('--publish',action='store_true')
    args=p.parse_args()
    env=dict(os.environ,GIT_TERMINAL_PROMPT='0',GCM_INTERACTIVE='Never')
    result=subprocess.run(['git','credential','fill'],input='protocol=https\nhost=github.com\n\n',capture_output=True,text=True,timeout=20,env=env)
    credential=dict(line.split('=',1) for line in result.stdout.splitlines() if '=' in line)
    token=credential.get('password')
    if not token:raise RuntimeError('No stored noninteractive GitHub credential')
    headers={'Authorization':'Bearer '+token,'Accept':'application/vnd.github+json','User-Agent':'PirateWave-Beta-release','X-GitHub-Api-Version':'2022-11-28'}
    def api(path,method='GET',body=None):
        data=None if body is None else json.dumps(body).encode()
        request=urllib.request.Request('https://api.github.com/repos/'+REPO+path,data=data,headers=headers,method=method)
        with urllib.request.urlopen(request,timeout=60) as response:return json.load(response) if response.status!=204 else None
    releases=api('/releases?per_page=100')
    release=next((r for r in releases if r['tag_name']==TAG),None)
    if release and not release['draft']:raise RuntimeError('Release is already published; refusing to overwrite it')
    body=args.body.read_text(encoding='utf-8')
    if 'Voice Creation' not in body:raise ValueError('Expected explicit Voice Creation release notes')
    if release is None:
        release=api('/releases','POST',dict(tag_name=TAG,target_commitish=args.commit,name='PirateWave 0.2.0 Beta — local Voice Creation',body=body,draft=True,prerelease=True))
    else:
        if 'Voice Creation' not in release.get('body',''):raise ValueError('Existing draft has a different purpose')
        release=api('/releases/'+str(release['id']),'PATCH',dict(target_commitish=args.commit,body=body,prerelease=True))
    state=dict(id=release['id'],url=release['html_url'],draft=True,tag=TAG,assets=[])
    def save(): (args.folder/'release-state.json').write_text(json.dumps(state,indent=2)+'\n',encoding='utf-8')
    save()
    files=sorted(args.folder.glob('*.part*'))
    files += [p for p in [args.folder/name for name in (
        'PirateWave-LLVC-Voice-v1-beta1.zip','PirateWave-v0.2.0-beta.1-windows-x64.zip',
        'SHA256SUMS.txt','catalog.json','VOICE_TRAINING_NOTICES.txt',
        'VOICE_TRAINING.md','VOICE_TRAINING_BETA_VALIDATION.json','BUILD_MANIFEST.json',
        'validation-cpu.json','validation-cuda.json')] if p.is_file()]
    assets={a['name']:a for a in api('/releases/'+str(release['id'])+'/assets?per_page=100')}
    for path in files:
        digest=hashlib.sha256()
        with path.open('rb') as f:
            for block in iter(lambda:f.read(1024*1024),b''):digest.update(block)
        checksum=digest.hexdigest()
        existing=assets.get(path.name)
        if existing:
            if existing['size']!=path.stat().st_size or existing.get('digest')!='sha256:'+checksum:
                raise ValueError('Existing asset differs or cannot be verified: '+path.name)
            state['assets'].append(dict(name=path.name,sha256=checksum,url=existing['browser_download_url']))
            save();continue
        if path.stat().st_size>=2*1024**3:raise ValueError('GitHub asset exceeds the per-file limit')
        url=release['upload_url'].split('{',1)[0]+'?'+urllib.parse.urlencode({'name':path.name})
        parsed=urllib.parse.urlsplit(url)
        if parsed.scheme!='https' or parsed.hostname!='uploads.github.com':raise ValueError('Unexpected upload endpoint')
        connection=http.client.HTTPSConnection(parsed.hostname,timeout=180)
        connection.putrequest('POST',parsed.path+'?'+parsed.query)
        for key,value in headers.items():connection.putheader(key,value)
        connection.putheader('Content-Type','application/octet-stream')
        connection.putheader('Content-Length',str(path.stat().st_size));connection.endheaders()
        sent=0
        with path.open('rb') as f:
            for block in iter(lambda:f.read(1024*1024),b''):
                connection.send(block);sent+=len(block)
                if sent%(128*1024*1024)==0:print(json.dumps(dict(asset=path.name,sent=sent,total=path.stat().st_size)),flush=True)
        response=connection.getresponse()
        if response.status!=201:raise RuntimeError('Asset upload failed with HTTP '+str(response.status))
        asset=json.load(response);connection.close()
        if asset.get('digest')!='sha256:'+checksum:raise ValueError('GitHub asset digest differs: '+path.name)
        state['assets'].append(dict(name=path.name,sha256=checksum,url=asset['browser_download_url']));save()
        print(json.dumps(dict(uploaded=path.name,bytes=path.stat().st_size)),flush=True)
    if args.publish:
        if not any(a['name']=='PirateWave-v0.2.0-beta.1-windows-x64.zip' for a in state['assets']):raise ValueError('Portable app asset missing')
        for backend in ('cpu','cuda'):
            check=json.loads((args.folder/f'validation-{backend}.json').read_text())
            if not check['import_training_export_install_pass'] or not check['live_pipeline']['pass']:raise ValueError('Portable workflow validation missing')
        release=api('/releases/'+str(release['id']),'PATCH',dict(tag_name=TAG,draft=False,prerelease=True,target_commitish=args.commit,body=body))
        published={a['name']:a for a in api('/releases/'+str(release['id'])+'/assets?per_page=100')}
        for asset in state['assets']:
            asset['url']=published[asset['name']]['browser_download_url']
        state['draft']=False;state['url']=release['html_url'];save()
    print(json.dumps(dict(url=state['url'],draft=state['draft'],assets=len(state['assets']))),flush=True)


if __name__=='__main__':main()
