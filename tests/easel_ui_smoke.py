#!/usr/bin/env python3
"""Exercise an unchanged upstream Herdr build on its private headless display."""
import json, os, pathlib, shlex, subprocess, tempfile, time
from smoke import BIN, ROOT, eventually
APP=pathlib.Path(os.environ['HERDR_ORIGIN_WORKTREE'])
SCRIPT=APP/'scripts/sandbox'
name=f'rex-bridge-{os.getpid()}'
root=pathlib.Path('/var/tmp')/f'herdr-app-{name}'
env=dict(os.environ,HERDR_APP_SANDBOX=name,HERDR_APP_BIN=os.environ['HERDR_ORIGIN_BIN'],HERDR_APP_NO_SOUND='1')

def sandbox(*args,check=True):
    p=subprocess.run([str(SCRIPT),*args],env=env,capture_output=True,text=True,timeout=120)
    if check and p.returncode:raise AssertionError(f'sandbox {args}: {p.stdout}\n{p.stderr}')
    return p.stdout

def status():
    world=json.loads(subprocess.check_output([env['HERDR_APP_BIN'],'ls'],env=dict(env,HERDR_APP_HOME=str(root/'home')),text=True,timeout=10))
    world['window']=json.loads(sandbox('status'))
    return world

with tempfile.TemporaryDirectory(prefix='herdr-rex-bridge-ui-') as d:
    d=pathlib.Path(d);origin=f'unix://{d}/rex.sock';(d/'init.lua').write_text('')
    log=open(d/'rex.log','w+')
    server=subprocess.Popen(['rex','server','run','--restore=false','--listen',origin,'--state-dir',str(d/'state'),'--config',str(d/'init.lua')],env=dict(env,SHELL='/bin/sh'),stdout=log,stderr=log)
    up=False
    try:
        def rex(*args):return subprocess.check_output(['rex','--autostart=false','-S',origin,*args],env=env,stderr=subprocess.PIPE,text=True,timeout=20)
        eventually(lambda:(d/'rex.sock').exists() and rex('version'))
        session=json.loads(rex('new','Rex bridge demo','-c',str(d),'--json','--shell=none','--','/bin/sh'))['session_id']
        (root/'home').mkdir(parents=True)
        (root/'home/shared.json').write_text('{"frame.welcomed":"skipped"}')
        up=True
        via=shlex.join([str(BIN),'--server',origin,'--state-dir',str(d/'bridge'),'via'])
        sandbox('hc','machine','add','--name','rex','--via',via,'--json')
        sandbox('up')
        st=eventually(lambda:status() if any(s.get('machine')=='rex' for s in status().get('spaces',[])) else None)
        spaces=st['spaces'];remote=next(s for s in spaces if s.get('machine')=='rex')
        sandbox('do','space.select',json.dumps({'id':remote['id']}))
        sandbox('type','echo bridge-ui-ok');sandbox('key','Return')
        eventually(lambda:'bridge-ui-ok' in rex('capture','-s',session,'--trim'))
        easel=pathlib.Path(os.environ.get('EASEL_WORKTREE', ROOT.parent/'easel'))
        terminal=remote['tabs'][0]['terminals'][0]['terminal']
        mcp_env=dict(env,HERDR_APP_HOME=str(root/'home'),REX_BLOCK=terminal.split('/',1)[1],GPUI_ARTIFACTS_PROJECT=str(d),GPUI_ARTIFACTS_HERDR=env['HERDR_APP_BIN'])
        mcp_env.pop('HC_TERMINAL_ID',None)
        message={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'render_gpui','arguments':{'id':'card','title':'Easel beside Rex','source':(easel/'fixtures/card.rs').read_text(),'mode':'interpret'}}}
        p=subprocess.run([str(easel/'target/debug/gpui-artifacts-mcp')],input=json.dumps(message)+'\n',capture_output=True,text=True,env=mcp_env,timeout=20)
        print('MCP:',p.stdout,flush=True)
        assert not json.loads(p.stdout)['result']['isError'],p.stdout
        time.sleep(1)
        rex('session','rename',session,'Rex with Easel')
        time.sleep(1)
        # Reconnect the private machine transport: the decoration must come
        # back from bridge metadata, independent of the Herdr process.
        sandbox('hc','machine','rm','rex')
        sandbox('hc','machine','add','--name','rex','--via',via,'--json')
        def restored():
            return next((s for s in status()['spaces'] if s.get('machine')=='rex'),None)
        remote=eventually(restored)
        sandbox('do','space.select',json.dumps({'id':remote['id']}))
        time.sleep(1)
        assert len(json.loads(rex('block','ls','-s',session,'--json'))['blocks']) == 1, 'A mod must not create a fake Rex terminal'
        sandbox('shot' ,str(easel/'target/herdr-mod-proof/stock-herdr-rex-easel.png'))
        ping=json.loads(subprocess.check_output([str(easel/'target/debug/gpui-artifacts-mod'),'call',str(d),'{"action":"ping"}'],env=mcp_env,text=True))
        print('Easel backend:',ping,flush=True)
        print('Herdr:',status(),flush=True)
        assert ping['attached'],ping
        assert subprocess.check_output(['git','-C',str(APP),'status','--porcelain'],text=True)==''
        print('PASS: Easel beside Rex survives external layout updates and bridge reconnect, without creating extra Rex terminals or patching Herdr')
    finally:
        if up:sandbox('down',check=False)
        server.terminate()
        try:server.wait(timeout=5)
        except subprocess.TimeoutExpired:server.kill();server.wait()
        log.close()
