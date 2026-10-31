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
        (ROOT/'artifacts').mkdir(exist_ok=True)
        (ROOT/'artifacts/stock-herdr-status.json').write_text(json.dumps(st,indent=2)+'\n')
        spaces=st['spaces'];remote=next(s for s in spaces if s.get('machine')=='rex')
        sandbox('do','space.select',json.dumps({'id':remote['id']}))
        sandbox('type','echo bridge-ui-ok');sandbox('key','Return')
        eventually(lambda:'bridge-ui-ok' in rex('capture','-s',session,'--trim'))
        # A native Herdr split must create a split in Rex, through the bridge.
        sandbox('do','pane.split','{"direction":"right"}')
        eventually(lambda:len(json.loads(rex('block','ls','-s',session,'--json'))['blocks'])>=2)
        time.sleep(.6)
        sandbox('type','clear');sandbox('key','Return')
        sandbox('type','echo Rex pane created by stock Herdr');sandbox('key','Return')
        time.sleep(.3)
        sandbox('shot',str(ROOT/'artifacts/stock-herdr-rex.png'))
        (ROOT/'artifacts/stock-herdr-status.json').write_text(json.dumps(status(),indent=2)+'\n')
        assert subprocess.check_output(['git','-C',str(APP),'status','--porcelain'],text=True)==''
        print(f'PASS: stock origin {subprocess.check_output(["git","-C",str(APP),"rev-parse","--short","HEAD"],text=True).strip()}; Rex section, typing, pane.split, screenshot; no app source changes')
    finally:
        if up:sandbox('down',check=False)
        server.terminate()
        try:server.wait(timeout=5)
        except subprocess.TimeoutExpired:server.kill();server.wait()
        log.close()
