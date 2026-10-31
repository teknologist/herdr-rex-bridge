use anyhow::Result;
use serde_json::Value;
use std::time::Duration;
pub const GENERATION: u64 = 1;
pub const VERSION: &str = "0.1.14";
pub const USES: &[&str] = &[
    "terminal.run",
    "terminal.list",
    "terminal.get",
    "terminal.kill",
    "terminal.label",
    "terminal.attach",
    "terminal.read",
    "terminal.keep",
    "doc.get",
    "doc.set",
    "events.subscribe",
];
pub enum Stream {
    Data(Vec<u8>),
    Snapshot(Vec<u8>),
    Closed(Value),
    Lost,
}
pub struct Attached {
    pub channel: u32,
    pub stream: async_channel::Receiver<Stream>,
    pub terminal: Value,
}
#[derive(Debug)]
pub struct RpcError(pub Value);
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for RpcError {}
#[derive(Clone, Copy)]
pub enum Kind {
    Rex,
}
pub trait Link: Send + Sync {
    fn call_timeout(&self, method: &str, params: Value, timeout: Duration) -> Result<Value>;
    fn subscribe(&self) -> Result<(u32, async_channel::Receiver<Stream>, Value)>;
    fn attach(&self, terminal: &str, cols: u16, rows: u16, display: Value) -> Result<Attached>;
    fn input(&self, channel: u32, bytes: &[u8]);
    fn reply(&self, channel: u32, bytes: &[u8]);
    fn credit(&self, channel: u32, n: usize);
    fn resize(&self, channel: u32, cols: u16, rows: u16);
    fn facts(&self, channel: u32, display: &Value);
    fn focus(&self, channel: u32, focused: bool);
    fn detach(&self, channel: u32);
    fn lost(&self) -> async_channel::Receiver<()>;
    fn close(&self);
}
pub struct Core {
    pub hello: Value,
    pub link: Box<dyn Link>,
}
impl Core {
    pub fn new(hello: Value, _kind: Kind, link: Box<dyn Link>) -> Self {
        Self { hello, link }
    }
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.link
            .call_timeout(method, params, Duration::from_secs(10))
    }
}
