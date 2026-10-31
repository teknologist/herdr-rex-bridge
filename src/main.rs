use anyhow::{Context, Result, bail};
use herdr_rex_bridge::{
    rex::{adapter, transport::Endpoint},
    wire,
};
use std::path::PathBuf;
fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    if let Err(e) = run() {
        eprintln!("herdr-rex-bridge: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut server = None;
    let mut state = None;
    let mut mode = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = Some(args.next().context("--server needs an endpoint")?),
            "--state-dir" => {
                state = Some(PathBuf::from(
                    args.next().context("--state-dir needs a path")?,
                ))
            }
            "--version" | "-V" => {
                println!("herdr-rex-bridge {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "herdr-rex-bridge [--server ORIGIN] [--state-dir PATH] connect\nherdr-rex-bridge [--server ORIGIN] [--state-dir PATH] via <hc remote command>\n\nRegister without changing Herdr: hc machine add --name rex --via '/absolute/path/herdr-rex-bridge --server unix:///path/rex.sock via'\nOne bridge registration per Rex endpoint. See README.md."
                );
                return Ok(());
            }
            "connect" => {
                mode = Some("connect");
                if args.next().is_some() {
                    bail!("unexpected argument after connect");
                }
                break;
            }
            "via" => {
                let command = args
                    .next()
                    .context("via needs the remote command supplied by hc")?;
                if !command.ends_with("connect'")
                    && !command.ends_with(" connect")
                    && !command.ends_with("connect\"")
                {
                    bail!("via only serves hc connect; it never executes shell commands");
                }
                if args.next().is_some() {
                    bail!("unexpected argument after via command");
                }
                mode = Some("via");
                break;
            }
            _ => bail!("unknown argument {arg}; use --help"),
        }
    }
    if mode.is_none() {
        bail!("choose connect or via; use --help");
    }
    let endpoint = Endpoint::discover(server.as_deref())?;
    let key = endpoint
        .origin
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
    let dir = state
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .map(|p| p.join("herdr-rex-bridge"))
        })
        .or_else(|| std::env::home_dir().map(|p| p.join(".local/state/herdr-rex-bridge")))
        .context("set --state-dir")?;
    std::fs::create_dir_all(&dir)?;
    let core = adapter::connect_to(endpoint, dir.join(format!("{key:016x}.json")))?;
    let mut hello = core.hello.clone();
    hello["core"] = serde_json::json!({"id":format!("rex-{key:016x}"),"name":"Rex"});
    hello["bridge_version"] = env!("CARGO_PKG_VERSION").into();
    wire::serve(core, hello)
}
