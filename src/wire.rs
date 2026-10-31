//! Generation-1 hc protocol over stdin/stdout, suitable for hc machine --via.
use crate::{
    hc::{Core, RpcError, Stream},
    screen::{OutputFilter, snapshot_vt},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, BufRead, Read, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
pub const MAX_PAYLOAD: usize = 16 << 20;
const DATA: u8 = 0;
const CREDIT: u8 = 1;
const CLOSE: u8 = 2;
const RESIZE: u8 = 3;
const FOCUS: u8 = 4;
const FACTS: u8 = 5;
struct Gate {
    credit: Mutex<usize>,
    changed: Condvar,
    closed: AtomicBool,
}
impl Gate {
    fn new() -> Self {
        Self {
            credit: Mutex::new(256 << 10),
            changed: Condvar::new(),
            closed: AtomicBool::new(false),
        }
    }
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.changed.notify_all();
    }
    fn add(&self, n: usize) {
        let mut c = self.credit.lock().unwrap();
        *c = c.saturating_add(n).min(64 << 20);
        self.changed.notify_all();
    }
    fn take(&self, n: usize) -> bool {
        let mut c = self.credit.lock().unwrap();
        while *c < n && !self.closed.load(Ordering::Relaxed) {
            c = self.changed.wait(c).unwrap();
        }
        if self.closed.load(Ordering::Relaxed) {
            return false;
        }
        *c -= n;
        true
    }
}
struct Connection {
    core: Arc<Core>,
    hello: Value,
    framed: bool,
    out: Mutex<io::Stdout>,
    channels: Mutex<HashMap<u32, Arc<Gate>>>,
    closed: AtomicBool,
    calls: AtomicUsize,
}
impl Connection {
    fn frame(&self, ch: u32, kind: u8, p: &[u8]) -> Result<()> {
        ensure!(p.len() <= MAX_PAYLOAD, "frame too large");
        let mut out = self.out.lock().unwrap();
        if self.framed {
            out.write_all(&ch.to_le_bytes())?;
            out.write_all(&[kind])?;
            out.write_all(&(p.len() as u32).to_le_bytes())?;
        }
        out.write_all(p)?;
        if !self.framed {
            out.write_all(b"\n")?;
        }
        out.flush()?;
        Ok(())
    }
    fn reply(&self, id: Value, result: Result<Value>) -> Result<()> {
        let value = match result {
            Ok(v) => json!({"jsonrpc":"2.0","id":id,"result":v}),
            Err(e) => {
                let err = e
                    .downcast_ref::<RpcError>()
                    .map(|e| e.0.clone())
                    .unwrap_or_else(
                        || json!({"message":format!("{e:#}"),"data":{"kind":"failed"}}),
                    );
                let mut err = err;
                err["code"] = (-32000).into();
                json!({"jsonrpc":"2.0","id":id,"error":err})
            }
        };
        self.frame(0, DATA, value.to_string().as_bytes())
    }
    fn call(self: &Arc<Self>, req: Value) -> Result<()> {
        let id = req["id"].clone();
        let method = req["method"].as_str().context("missing method")?;
        let p = &req["params"];
        if id.is_null() {
            return Ok(());
        }
        match method {
            "hello" => self.reply(id, Ok(self.hello.clone())),
            "events.subscribe" => {
                if !self.framed {
                    return self.reply(
                        id,
                        Err(anyhow::anyhow!("events.subscribe requires framed mode")),
                    );
                }
                match self.core.link.subscribe() {
                    Ok((ch, rx, snapshot)) => {
                        let gate = self.channel(ch);
                        self.reply(id, Ok(snapshot))?;
                        self.stream(ch, rx, gate, false);
                        Ok(())
                    }
                    Err(e) => self.reply(id, Err(e)),
                }
            }
            "terminal.attach" => {
                if !self.framed {
                    return self.reply(
                        id,
                        Err(anyhow::anyhow!("terminal.attach requires framed mode")),
                    );
                }
                let cols = p["cols"].as_u64().unwrap_or(80);
                let rows = p["rows"].as_u64().unwrap_or(24);
                if cols == 0
                    || rows == 0
                    || cols > u16::MAX as u64
                    || rows > u16::MAX as u64
                    || cols * rows > 65536
                {
                    return self.reply(id, Err(anyhow::anyhow!("invalid terminal size")));
                }
                match self.core.link.attach(
                    p["terminal"].as_str().unwrap_or_default(),
                    cols as u16,
                    rows as u16,
                    p["display"].clone(),
                ) {
                    Ok(a) => {
                        let gate = self.channel(a.channel);
                        self.reply(id, Ok(json!({"channel":a.channel,"terminal":a.terminal})))?;
                        self.stream(a.channel, a.stream, gate, true);
                        Ok(())
                    }
                    Err(e) => self.reply(id, Err(e)),
                }
            }
            _ => self.reply(id, self.core.call(method, p.clone())),
        }
    }
    fn channel(&self, ch: u32) -> Arc<Gate> {
        let g = Arc::new(Gate::new());
        self.channels.lock().unwrap().insert(ch, g.clone());
        g
    }
    fn stream(
        self: &Arc<Self>,
        ch: u32,
        rx: async_channel::Receiver<Stream>,
        gate: Arc<Gate>,
        terminal: bool,
    ) {
        let this = self.clone();
        std::thread::spawn(move || {
            let mut filter = OutputFilter::default();
            let run = || -> Result<()> {
                loop {
                    let item = match rx.recv_blocking() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let bytes = match item {
                        Stream::Snapshot(b) => filter.feed(&snapshot_vt(&b)?),
                        Stream::Data(b) => {
                            if terminal {
                                filter.feed(&b)
                            } else {
                                b
                            }
                        }
                        Stream::Closed(reason) => {
                            this.frame(ch, CLOSE, reason.to_string().as_bytes())?;
                            return Ok(());
                        }
                        Stream::Lost => {
                            this.frame(ch, CLOSE, br#"{"reason":"lost"}"#)?;
                            return Ok(());
                        }
                    };
                    for chunk in bytes.chunks(16384) {
                        if !gate.take(chunk.len()) {
                            return Ok(());
                        }
                        this.frame(ch, DATA, chunk)?;
                    }
                }
                Ok(())
            };
            let mut run = run;
            if let Err(e) = run() {
                tracing::warn!(channel = ch, "stream closed: {e:#}");
                let _ = this.frame(ch, CLOSE, br#"{"reason":"lost"}"#);
            }
            gate.close();
            this.channels.lock().unwrap().remove(&ch);
            if terminal {
                this.core.link.detach(ch);
            }
        });
    }
    fn control(&self, ch: u32, kind: u8, p: &[u8]) -> Result<()> {
        let gate = self.channels.lock().unwrap().get(&ch).cloned();
        let Some(g) = gate else {
            return Ok(());
        };
        match kind {
            DATA => self.core.link.input(ch, p),
            CREDIT => {
                ensure!(p.len() == 4, "bad CREDIT");
                g.add(u32::from_le_bytes(p.try_into().unwrap()) as usize);
            }
            CLOSE => {
                g.close();
                self.core.link.detach(ch);
                self.frame(ch, CLOSE, b"")?;
            }
            RESIZE => {
                ensure!(p.len() == 4, "bad RESIZE");
                let cols = u16::from_le_bytes(p[..2].try_into().unwrap());
                let rows = u16::from_le_bytes(p[2..].try_into().unwrap());
                ensure!(
                    cols > 0 && rows > 0 && u32::from(cols) * u32::from(rows) <= 65536,
                    "bad size"
                );
                self.core.link.resize(ch, cols, rows);
            }
            FOCUS => {
                ensure!(p.len() == 1, "bad FOCUS");
                self.core.link.focus(ch, p[0] != 0);
            }
            FACTS => self.core.link.facts(ch, &serde_json::from_slice(p)?),
            _ => anyhow::bail!("unknown frame kind {kind}"),
        }
        Ok(())
    }
    fn shutdown(&self) {
        self.closed.store(true, Ordering::Relaxed);
        for g in self.channels.lock().unwrap().values() {
            g.close();
        }
        self.core.link.close();
    }
}
pub fn read_frame(r: &mut impl Read) -> Result<Option<(u32, u8, Vec<u8>)>> {
    let mut h = [0; 9];
    match r.read(&mut h[..1])? {
        0 => return Ok(None),
        _ => r.read_exact(&mut h[1..])?,
    };
    let len = u32::from_le_bytes(h[5..9].try_into().unwrap()) as usize;
    ensure!(len <= MAX_PAYLOAD, "frame exceeds 16 MiB");
    let mut p = vec![0; len];
    r.read_exact(&mut p)?;
    Ok(Some((
        u32::from_le_bytes(h[..4].try_into().unwrap()),
        h[4],
        p,
    )))
}
pub fn serve(core: Arc<Core>, hello: Value) -> Result<()> {
    let mut input = io::BufReader::new(io::stdin());
    let mut line = String::new();
    input
        .by_ref()
        .take((MAX_PAYLOAD + 1) as u64)
        .read_line(&mut line)?;
    ensure!(line.len() <= MAX_PAYLOAD, "hello too large");
    let req: Value = serde_json::from_str(&line)?;
    ensure!(req["method"] == "hello", "hello must be first");
    let framed = req["params"]["framed"] == true;
    let this = Arc::new(Connection {
        core,
        hello,
        framed: false,
        out: Mutex::new(io::stdout()),
        channels: Mutex::new(HashMap::new()),
        closed: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    this.call(req)?;
    // Handshake has been sent as one JSON line. All later bytes use the negotiated mode.
    let mut this = Arc::try_unwrap(this).ok().unwrap();
    this.framed = framed;
    let this = Arc::new(this);
    let tick = Arc::downgrade(&this);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let Some(c) = tick.upgrade() else { break };
            if c.closed.load(Ordering::Relaxed) {
                break;
            }
            if c.framed
                && c.frame(
                    0,
                    DATA,
                    br#"{"jsonrpc":"2.0","method":"heartbeat","params":{}}"#,
                )
                .is_err()
            {
                break;
            }
        }
    });
    let run = || -> Result<()> {
        loop {
            let req: Value = if framed {
                let Some((ch, kind, p)) = read_frame(&mut input)? else {
                    break;
                };
                if ch != 0 {
                    this.control(ch, kind, &p)?;
                    continue;
                }
                ensure!(kind == DATA, "channel zero only carries JSON-RPC");
                serde_json::from_slice(&p)?
            } else {
                line.clear();
                if input
                    .by_ref()
                    .take((MAX_PAYLOAD + 1) as u64)
                    .read_line(&mut line)?
                    == 0
                {
                    break;
                };
                ensure!(line.len() <= MAX_PAYLOAD, "request too large");
                serde_json::from_str(&line)?
            };
            if this.calls.fetch_add(1, Ordering::Relaxed) >= 64 {
                this.calls.fetch_sub(1, Ordering::Relaxed);
                this.reply(
                    req["id"].clone(),
                    Err(anyhow::anyhow!("too many outstanding calls")),
                )?;
                continue;
            }
            let c = this.clone();
            std::thread::spawn(move || {
                if let Err(e) = c.call(req) {
                    tracing::debug!("request failed: {e:#}");
                }
                c.calls.fetch_sub(1, Ordering::Relaxed);
            });
        }
        Ok(())
    };
    let mut run = run;
    let result = run();
    this.shutdown();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversize_and_truncated_frames_before_allocating() {
        let mut h = vec![0; 5];
        h.extend(((MAX_PAYLOAD + 1) as u32).to_le_bytes());
        assert!(read_frame(&mut h.as_slice()).is_err());
        assert!(read_frame(&mut [1u8, 0].as_slice()).is_err());
        assert!(read_frame(&mut [].as_slice()).unwrap().is_none());
    }
    #[test]
    fn credit_blocks_only_its_stream_and_close_unblocks_it() {
        let g = Arc::new(Gate::new());
        assert!(g.take(256 << 10));
        let worker = g.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || tx.send(worker.take(1)).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        g.close();
        assert!(!rx.recv_timeout(Duration::from_secs(1)).unwrap());
        handle.join().unwrap();
        let g = Gate::new();
        assert!(g.take(256 << 10));
        g.add(4);
        assert!(g.take(4));
    }
}
