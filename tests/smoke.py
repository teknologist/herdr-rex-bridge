#!/usr/bin/env python3
"""Real Rex + installed hc + standalone bridge. All servers and data are isolated."""
import json, os, pathlib, selectors, struct, subprocess, tempfile, time, unittest
ROOT=pathlib.Path(__file__).resolve().parents[1]
BIN=pathlib.Path(os.environ.get('HERDR_REX_BRIDGE_BIN',ROOT/'target/debug/herdr-rex-bridge'))

def eventually(fn, timeout=15):
    end=time.monotonic()+timeout
    last=None
    while time.monotonic()<end:
        try:
            value=fn()
            if value:return value
        except (AssertionError,KeyError,subprocess.CalledProcessError) as e:last=e
        time.sleep(.05)
    raise AssertionError(f'timed out: {last}')

class Rpc:
    def __init__(self, args):
        self.p=subprocess.Popen(args,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        self.serial=0;self.buffer=bytearray();self.events=[]
        self.p.stdin.write(b'{"jsonrpc":"2.0","id":0,"method":"hello","params":{"framed":true}}\n');self.p.stdin.flush()
        self.hello=json.loads(self.p.stdout.readline())['result']
    def send(self,ch,kind,payload):
        self.p.stdin.write(struct.pack('<IBI',ch,kind,len(payload))+payload);self.p.stdin.flush()
    def frame(self,timeout=15):
        end=time.monotonic()+timeout
        while True:
            if len(self.buffer)>=9:
                ch,kind,n=struct.unpack('<IBI',self.buffer[:9])
                if len(self.buffer)>=9+n:
                    b=bytes(self.buffer[9:9+n]);del self.buffer[:9+n]
                    if ch and kind==0:self.send(ch,1,struct.pack('<I',len(b)))
                    return ch,kind,b
            remaining=end-time.monotonic()
            if remaining<=0:raise AssertionError('no frame within timeout')
            with selectors.DefaultSelector() as sel:
                sel.register(self.p.stdout,selectors.EVENT_READ)
                if not sel.select(remaining):raise AssertionError('no frame within timeout')
            b=os.read(self.p.stdout.fileno(),65536)
            if not b:raise AssertionError(f'bridge EOF: {self.p.stderr.read().decode()}')
            self.buffer.extend(b)
    def call(self,method,params=None):
        self.serial+=1; wanted=self.serial
        self.send(0,0,json.dumps(dict(jsonrpc='2.0',id=wanted,method=method,params=params or {})).encode())
        while True:
            f=self.frame()
            if f[0]==0 and f[1]==0:
                v=json.loads(f[2])
                if v.get('id')==wanted:
                    assert 'error' not in v,v
                    return v['result']
            self.events.append(f)
    def until_text(self,ch,needle):
        out=b''
        for f in self.events:
            if f[0]==ch and f[1]==0:out+=f[2]
        self.events=[]
        end=time.monotonic()+15
        while needle not in out:
            f=self.frame(max(.01,end-time.monotonic()))
            if f[0]==ch and f[1]==0:out+=f[2]
            elif f[0]==ch and f[1]==2:raise AssertionError(f'stream closed: {f}, output={out!r}')
        return out
    def close(self):
        if self.p.poll() is None:
            self.p.stdin.close()
            try:self.p.wait(timeout=5)
            except subprocess.TimeoutExpired:self.p.kill();self.p.wait()
        for stream in [self.p.stdout,self.p.stderr]:stream.close()

class BridgeTest(unittest.TestCase):
    def test_real_rex_and_hc_machine(self):
        with tempfile.TemporaryDirectory(prefix='herdr-rex-bridge-test-') as d:
            root=pathlib.Path(d);origin=f'unix://{root}/rex.sock'
            (root/'init.lua').write_text('')
            env=dict(os.environ,HC_SESSION=f'bridge-test-{os.getpid()}',XDG_STATE_HOME=str(root/'state'),XDG_CONFIG_HOME=str(root/'config'),SHELL='/bin/sh')
            log=open(root/'rex.log','w+')
            rex=subprocess.Popen(['rex','server','run','--restore=false','--listen',origin,'--state-dir',str(root/'rex-state'),'--config',str(root/'init.lua')],env=env,stdout=log,stderr=log)
            rpc=None
            def r(*args):return subprocess.check_output(['rex','--autostart=false','-S',origin,*args],env=env,stderr=subprocess.PIPE,timeout=20).decode()
            def hc(*args):
                result=subprocess.run(['hc',*args],env=env,capture_output=True,timeout=25)
                if result.returncode:raise AssertionError(f'hc {args}: {result.stdout.decode()} {result.stderr.decode()}')
                return json.loads(result.stdout)
            bridge=[str(BIN),'--server',origin,'--state-dir',str(root/'bridge')]
            try:
                eventually(lambda: (root/'rex.sock').exists() and r('version'))
                created=json.loads(r('new','Fixture','-c',d,'--json'))
                session=created['session_id']
                rpc=Rpc([*bridge,'connect'])
                self.assertEqual(rpc.hello['core']['name'],'Rex')
                terminals=eventually(lambda:rpc.call('terminal.list')['terminals'])
                terminal=terminals[0]['address']
                doc=rpc.call('doc.get',{'name':'herdr-app'})
                self.assertEqual(len(doc['value']['spaces']),1)
                sub=rpc.call('events.subscribe')
                attached=rpc.call('terminal.attach',{'terminal':terminal,'cols':80,'rows':24})
                ch=attached['channel']
                rpc.send(ch,0,b"printf 'bridge-input-ok\\n'\n")
                output=rpc.until_text(ch,b'bridge-input-ok')
                self.assertNotIn(b'GHOSTSNP',output)
                rpc.send(ch,3,struct.pack('<HH',100,30))
                rpc.send(ch,4,b'\x01')
                rpc.send(ch,5,b'{"fg":"#ffffff","bg":"#000000","cell":[8,16]}')
                # Editing the projected document must change Rex's own session.
                doc=rpc.call('doc.get',{'name':'herdr-app'})
                doc['value']['spaces'][0]['name']='Renamed in Herdr'
                rpc.call('doc.set',dict(name='herdr-app',value=doc['value'],version=doc['version']))
                eventually(lambda:'Renamed in Herdr' in r('ls','--json'))
                # A background CLI run must start even if no viewer attaches.
                marker=root/'background-started'
                rpc.call('terminal.run',{'argv':['/bin/sh','-c',f'echo started > {marker}; sleep 60'],'cwd':d})
                eventually(lambda:marker.exists())
                self.assertEqual(marker.read_text().strip(),'started')
                rpc.close();rpc=None
                # A fresh connection replays output from before the attachment.
                rpc=Rpc([*bridge,'connect'])
                attached=rpc.call('terminal.attach',{'terminal':terminal,'cols':80,'rows':24})
                rpc.until_text(attached['channel'],b'bridge-input-ok')
                rpc.close();rpc=None
                import shlex
                via=shlex.join([*bridge,'via'])
                hc('machine','add','--name','rex','--via',via,'--json')
                machines=eventually(lambda:hc('machine','ls','--json')['machines'])
                self.assertEqual(machines[0]['name'],'rex')
                remote=eventually(lambda:[t for t in hc('ls','--json')['terminals'] if t['address'].startswith('rex/')])
                self.assertEqual(remote[0]['address'],f'rex/{terminal}')
                # The app uses the normal local hc for machine docs; no custom core setting.
                conn=subprocess.Popen(['hc','connect'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                conn.stdin.write(b'{"jsonrpc":"2.0","id":1,"method":"doc.get","params":{"name":"herdr-app","on":"rex"}}\n');conn.stdin.flush()
                response=json.loads(conn.stdout.readline());self.assertEqual(response['result']['value']['spaces'][0]['name'],'Renamed in Herdr')
                conn.stdin.close();conn.wait(timeout=5);conn.stdout.close();conn.stderr.close()
                print('PASS: snapshots, input, resize/focus/facts, layout mutation, reconnect, hc --via, namespaced terminals and remote docs')
            finally:
                if rpc:rpc.close()
                subprocess.run(['hc','server','stop'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10)
                rex.terminate()
                try:rex.wait(timeout=5)
                except subprocess.TimeoutExpired:rex.kill();rex.wait()
                log.close()
if __name__=='__main__':unittest.main()
