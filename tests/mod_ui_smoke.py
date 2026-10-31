#!/usr/bin/env python3
"""Verify the sidebar with two private Rex servers, a normal core, and stock Herdr."""
import json, os, pathlib, shlex, shutil, subprocess, tempfile, time
from smoke import BIN, ROOT, eventually
APP=pathlib.Path(os.environ['HERDR_ORIGIN_WORKTREE'])
SCRIPT=APP/'scripts/sandbox'
name=f'rex-sidebar-{os.getpid()}'
root=pathlib.Path('/var/tmp')/f'herdr-app-{name}'
env=dict(os.environ,HERDR_APP_SANDBOX=name,HERDR_APP_BIN=os.environ['HERDR_ORIGIN_BIN'],HERDR_APP_NO_SOUND='1')

def sandbox(*args,check=True):
    p=subprocess.run([str(SCRIPT),*args],env=env,capture_output=True,text=True,timeout=120)
    if check and p.returncode:raise AssertionError(f'sandbox {args}: {p.stdout}\n{p.stderr}')
    return p.stdout

def status():
    world=json.loads(subprocess.check_output([env['HERDR_APP_BIN'],'ls'],env=dict(env,HERDR_APP_HOME=str(root/'home')),text=True,timeout=10))
    world['window']=json.loads(sandbox('status'));return world

with tempfile.TemporaryDirectory(prefix='rex-sidebar-demo-') as temporary:
    d=pathlib.Path(temporary);(d/'init.lua').write_text('');servers=[];logs=[];up=False
    native_env=dict(env,HC_SESSION=f'rex-sidebar-native-{os.getpid()}',XDG_STATE_HOME=str(d/'native-state'))
    def rex(origin,*args):return subprocess.check_output(['rex','--autostart=false','-S',origin,*args],env=env,stderr=subprocess.PIPE,text=True,timeout=20)
    try:
        origins={}
        for machine,session_name in [('buildbox','Build tools'),('workstation','API project')]:
            origin=f'unix://{d}/{machine}.sock';origins[machine]=origin
            log=open(d/f'{machine}.log','w+');logs.append(log)
            child=subprocess.Popen(['rex','server','run','--restore=false','--listen',origin,'--state-dir',str(d/machine),'--config',str(d/'init.lua')],env=env,stdout=log,stderr=log);servers.append(child)
            eventually(lambda: pathlib.Path(origin.removeprefix('unix://')).exists() and rex(origin,'version'))
            rex(origin,'new',session_name,'-c',str(d),'--json','--shell=none','--','/bin/sh')
        (root/'home').mkdir(parents=True);(root/'home/shared.json').write_text('{"frame.welcomed":"skipped"}')
        for mod in ['rex-bridge','rex-bridge-spaces']:
            shutil.copytree(ROOT/'mods'/mod,root/'home/mods'/mod,ignore=shutil.ignore_patterns('*.d.ts','jsconfig.json'))
        up=True
        for machine,origin in origins.items():
            via=shlex.join([str(BIN),'--server',origin,'--state-dir',str(d/'bridge'),'via'])
            sandbox('hc','machine','add','--name',machine,'--via',via,'--json')
        # A normal herdr-core reached through its own isolated session.
        via=d/'core-via'
        via.write_text('#!/bin/sh\nexport HC_SESSION='+shlex.quote(native_env['HC_SESSION'])+'\nexport XDG_STATE_HOME='+shlex.quote(native_env['XDG_STATE_HOME'])+'\nexec '+shlex.quote(shutil.which('hc'))+' connect\n');via.chmod(0o700)
        sandbox('hc','machine','add','--name','core-host','--via',shlex.quote(str(via)),'--json')
        sandbox('up')
        sandbox('do','space.new',json.dumps({'name':'Local project','folder':str(d),'argv':['/bin/sh']}))
        eventually(lambda: json.loads(sandbox('do','space.new',json.dumps({'name':'Core project','machine':'core-host','folder':str(d),'argv':['/bin/sh']}))))
        sandbox('do','layout.clone','{"name":"main","as":"rex-sidebar"}')
        layout=root/'home/layouts/rex-sidebar.json';tree=json.loads(layout.read_text())
        def find(n,id):
            if n.get('id')==id:return n
            for c in n.get('children',[]):
                f=find(c,id)
                if f:return f
        find(tree,'spaces')['content']['mod']='rex-bridge-spaces'
        lists=find(tree,'sidebar-lists');at=next(i for i,c in enumerate(lists['children']) if c['id']=='spaces')
        lists['children'].insert(at+1,{'id':'rex-servers','type':'block','size':{'weight':1},'content':{'mod':'rex-bridge'}})
        layout.write_text(json.dumps(tree,indent=2)+'\n')
        # Herdr hot reloads the personal clone, leaving its shipped layout intact.
        eventually(lambda:status()['window']['layout']=='rex-sidebar')
        remote=eventually(lambda:next((s for s in status()['spaces'] if s.get('machine')=='workstation'),None))
        sandbox('do','space.select',json.dumps({'id':remote['id']}))
        sandbox('type','echo Rex backend connected');sandbox('key','Return')
        sandbox('do','pane.split','{"direction":"right","argv":["/bin/sh"]}')
        sandbox('type','echo Rex terminal in Herdr');sandbox('key','Return')
        time.sleep(1)
        s=status();assert s['window']['errors']==[],s
        assert any(x.get('machine')=='core-host' for x in s['spaces']),s
        shared=json.loads((root/'home/shared.json').read_text())
        assert shared['rex-bridge.servers']==['buildbox','workstation'],shared
        sandbox('shot',str(ROOT/'preview.png'))
        # Both mods must also survive a narrow sidebar without a render error.
        sandbox('do','layout.resize','{"id":"sidebar","px":90}')
        time.sleep(.2);assert status()['window']['errors']==[],status()
        sandbox('do','layout.resize','{"id":"sidebar","px":240}')
        assert subprocess.check_output(['git','-C',str(APP),'status','--porcelain'],text=True)==''
        print('PASS: separate native-style Herdr and Rex sidebar groups; two Rex servers; normal core preserved; live terminal input/split; narrow mode; updated preview.')
    finally:
        if up:sandbox('down',check=False)
        subprocess.run(['hc','server','stop'],env=native_env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10)
        for child in servers:
            child.terminate()
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:child.kill();child.wait()
        for log in logs:log.close()
