use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use heirloom_core::{Memory, Store};
use heirloom_fs::FsIngester;
use heirloom_ingester::{IngestContext, Ingester};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Local-first, MCP-native personal memory for AI.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Override the data directory (default: platform user data dir).
    #[arg(long, global = true, env = "HEIRLOOM_HOME")]
    home: Option<PathBuf>,

    /// Emit machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Initialize a fresh Heirloom home and print MCP client snippets.
    Init,

    /// Add a memory directly from the command line.
    Add {
        /// The memory content. If omitted, read from stdin.
        content: Option<String>,
        /// Source tag (default: "cli").
        #[arg(long, default_value = "cli")]
        source: String,
        /// Kind tag (default: "note").
        #[arg(long, default_value = "note")]
        kind: String,
    },

    /// Search memories with a free-text query.
    Search {
        /// The search query.
        query: String,
        /// Max results.
        #[arg(short, long, default_value_t = 10)]
        k: usize,
        /// Restrict to these sources.
        #[arg(short, long)]
        source: Vec<String>,
    },

    /// Run an ingester.
    Ingest {
        /// Ingester name. One of: fs, browser, claude, chatgpt, claude-code, slack, obsidian, firefox.
        name: String,
        /// Path option, passed to the ingester.
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Start the MCP server on stdio.
    Serve,

    /// Start the local web viewer on http://127.0.0.1:7878.
    Viewer {
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: String,
    },

    /// Start the viewer and open it in your default browser.
    Desktop {
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: String,
    },

    /// Start the auto-capture daemon (reads config.toml).
    Watch,

    /// Encrypt the database file with a passphrase, then remove the plaintext.
    Seal {
        /// Read passphrase from this env var instead of prompting.
        #[arg(long, env = "HEIRLOOM_PASSPHRASE")]
        passphrase: String,
    },

    /// Decrypt the sealed database back to its plaintext working form.
    Unseal {
        #[arg(long, env = "HEIRLOOM_PASSPHRASE")]
        passphrase: String,
    },

    /// Encrypted multi-device sync (requires a relay — see docs/design/sync-protocol.md).
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },

    /// Connect to and interact with a self-hosted Heirloom Teams server.
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },

    /// Export your store as JSONL to stdout (or --output FILE).
    Export {
        #[arg(long, short)]
        output: Option<PathBuf>,
        #[arg(long)]
        source: Option<String>,
    },

    /// Show store statistics.
    Status,

    /// Recent memories, newest first.
    Recent {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
        #[arg(short, long)]
        source: Option<String>,
    },

    /// Redact memories. Either pass --id or --query.
    Redact {
        #[arg(long, conflicts_with = "query")]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        query: Option<String>,
    },

    /// Run basic environment checks.
    Doctor,

    /// Backfill hybrid-search vectors for memories that don't have one yet.
    /// Run this once after upgrading from a v0.1.x store.
    Reindex,
}

#[derive(Subcommand, Debug)]
enum TeamAction {
    /// Show local team config (server URL, member id, whether a token is set).
    Status,
    /// Save the team server URL and bearer token locally.
    Join {
        /// Base URL of the team server (e.g. http://team.acme.internal:7900).
        url: String,
        #[arg(long, env = "HEIRLOOM_TEAM_TOKEN")]
        token: String,
    },
    /// Forget the configured team server and token.
    Leave,
    /// Verify the configured server is reachable.
    Ping,
    /// List members of the team (admin only).
    Members,
    /// Show the team's audit log (admin only).
    Audit {
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// Encrypt and upload memories from the local store to the team server.
    Push {
        #[arg(long, env = "HEIRLOOM_PASSPHRASE")]
        passphrase: String,
        /// Only push memories tagged with this source (e.g. --source notes).
        #[arg(long)]
        source: Option<String>,
        /// Cap the number of memories pushed in one run.
        #[arg(long, default_value_t = 500)]
        limit: usize,
    },
    /// Fetch memories from the team server and merge into the local store.
    Pull {
        #[arg(long, env = "HEIRLOOM_PASSPHRASE")]
        passphrase: String,
        #[arg(long, default_value_t = 200)]
        limit: i64,
    },
}

#[derive(Subcommand, Debug)]
enum SyncAction {
    /// Show the local sync state (device id, relay URL, last pull).
    Status,
    /// Set or clear the relay URL.
    SetRelay {
        /// Relay base URL (e.g. https://relay.heirloom.web.app). Omit to clear.
        url: Option<String>,
    },
    /// Encrypt and upload a snapshot of the local store to the configured relay.
    /// Hosted relay lands in v1.1 — for now this produces a local snapshot
    /// at `~/.heirloom/snapshots/` you can copy to another device.
    Push {
        #[arg(long, env = "HEIRLOOM_PASSPHRASE")]
        passphrase: String,
    },
    /// Decrypt and merge a snapshot file produced by `sync push` on another device.
    Pull {
        /// Path to the `.hlm` snapshot file.
        #[arg(long)]
        from: PathBuf,
        #[arg(long, env = "HEIRLOOM_PASSPHRASE")]
        passphrase: String,
    },
    /// Generate a fresh device id (rotates this device's identity).
    Reset,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing();

    let home = resolve_home(cli.home.clone())?;
    std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    let db_path = home.join("heirloom.db");

    match cli.cmd {
        Cmd::Init => cmd_init(&home, &db_path, cli.json),
        Cmd::Add {
            content,
            source,
            kind,
        } => cmd_add(&db_path, content, source, kind, cli.json),
        Cmd::Search { query, k, source } => cmd_search(&db_path, query, k, source, cli.json),
        Cmd::Ingest { name, path } => cmd_ingest(&db_path, name, path, cli.json).await,
        Cmd::Serve => cmd_serve(&db_path).await,
        Cmd::Viewer { addr } => cmd_viewer(&db_path, addr).await,
        Cmd::Desktop { addr } => cmd_desktop(&db_path, addr).await,
        Cmd::Watch => cmd_watch(&home, &db_path).await,
        Cmd::Seal { passphrase } => cmd_seal(&db_path, passphrase),
        Cmd::Unseal { passphrase } => cmd_unseal(&db_path, passphrase),
        Cmd::Sync { action } => cmd_sync(&home, &db_path, action).await,
        Cmd::Team { action } => cmd_team(&home, &db_path, action).await,
        Cmd::Export { output, source } => cmd_export(&db_path, output, source),
        Cmd::Status => cmd_status(&db_path, cli.json),
        Cmd::Recent { limit, source } => cmd_recent(&db_path, limit, source, cli.json),
        Cmd::Redact { id, query } => cmd_redact(&db_path, id, query, cli.json),
        Cmd::Doctor => cmd_doctor(&home, &db_path),
        Cmd::Reindex => cmd_reindex(&db_path),
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("HEIRLOOM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // MCP server speaks JSON on stdout; logs MUST go to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn resolve_home(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = PathBuf::from(home).join(".heirloom");
        return Ok(candidate);
    }
    let dirs = ProjectDirs::from("dev", "heirloom", "heirloom")
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    Ok(dirs.data_dir().to_path_buf())
}

fn open_store(db_path: &std::path::Path) -> Result<Arc<Store>> {
    let store = Store::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;
    // Attach the hybrid-search embedder so add()/search() use BM25 + cosine.
    store.set_embedder(Box::new(heirloom_vector::HashEmbedder::new()));
    Ok(Arc::new(store))
}

fn cmd_init(home: &std::path::Path, db_path: &std::path::Path, json: bool) -> Result<()> {
    let store = open_store(db_path)?;
    let count = store.count()?;
    let claude_snippet = serde_json::json!({
        "mcpServers": {
            "heirloom": {
                "command": "heirloom",
                "args": ["serve"]
            }
        }
    });

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "home": home.display().to_string(),
                "db": db_path.display().to_string(),
                "memories": count,
                "claude_desktop_config": claude_snippet,
            }))?
        );
        return Ok(());
    }

    println!("✓ Heirloom initialized");
    println!("  home:      {}", home.display());
    println!("  database:  {}", db_path.display());
    println!("  memories:  {}", count);
    println!();
    println!("Next steps:");
    println!("  1. Ingest something:        heirloom ingest fs --path ~/Documents");
    println!("  2. Try a search:            heirloom search \"some words\"");
    println!("  3. Start the MCP server:    heirloom serve");
    println!();
    println!("Drop this into your Claude Desktop config file:");
    println!("(macOS: ~/Library/Application Support/Claude/claude_desktop_config.json)");
    println!();
    println!("{}", serde_json::to_string_pretty(&claude_snippet)?);
    Ok(())
}

fn cmd_add(
    db_path: &std::path::Path,
    content: Option<String>,
    source: String,
    kind: String,
    json: bool,
) -> Result<()> {
    let content = match content {
        Some(c) => c,
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };
    let content = content.trim().to_string();
    if content.is_empty() {
        anyhow::bail!("empty content");
    }
    let store = open_store(db_path)?;
    let m = Memory::new(source, kind, &content);
    let inserted = store.add(&m)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": m.id,
                "inserted": inserted,
            }))?
        );
    } else if inserted {
        println!("✓ added {}", m.id);
    } else {
        println!("· skipped — duplicate of an existing memory");
    }
    Ok(())
}

fn cmd_search(
    db_path: &std::path::Path,
    query: String,
    k: usize,
    sources: Vec<String>,
    json: bool,
) -> Result<()> {
    let store = open_store(db_path)?;
    let filters = if sources.is_empty() {
        None
    } else {
        Some(heirloom_core::SearchFilters {
            sources: Some(sources),
            ..Default::default()
        })
    };
    let results = store.search(&query, k, filters)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }
    if results.is_empty() {
        println!("No matches.");
        return Ok(());
    }
    for (i, r) in results.iter().enumerate() {
        let title = r
            .memory
            .metadata
            .get("title")
            .cloned()
            .unwrap_or_else(|| r.memory.kind.clone());
        println!(
            "{}. [{}] {}  ({:.3})",
            i + 1,
            r.memory.source,
            title,
            r.score
        );
        if let Some(snip) = &r.snippet {
            // Strip the FTS5 <mark> tags for the terminal — easier to read.
            let stripped = snip.replace("<mark>", "").replace("</mark>", "");
            println!("   {}", stripped);
        }
        if let Some(path) = r.memory.metadata.get("path") {
            println!("   ↳ {}", path);
        }
        println!("   id: {}", r.memory.id);
        println!();
    }
    Ok(())
}

async fn cmd_ingest(
    db_path: &std::path::Path,
    name: String,
    path: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let store = open_store(db_path)?;
    let mut options: HashMap<String, String> = HashMap::new();
    if let Some(p) = path {
        // Different ingesters use different keys for the path/root option.
        let display = p.display().to_string();
        options.insert("path".into(), display.clone());
        options.insert("paths".into(), display.clone());
        options.insert("root".into(), display);
    }
    let ctx = IngestContext {
        store: store.clone(),
        since: None,
        options,
    };
    let report = match name.as_str() {
        "fs" => FsIngester.ingest(&ctx).await?,
        "browser" => heirloom_browser::BrowserIngester.ingest(&ctx).await?,
        "claude" => heirloom_claude::ClaudeIngester.ingest(&ctx).await?,
        "chatgpt" => heirloom_chatgpt::ChatGPTIngester.ingest(&ctx).await?,
        "claude-code" => {
            heirloom_claude_code::ClaudeCodeIngester
                .ingest(&ctx)
                .await?
        }
        "slack" => heirloom_slack::SlackIngester.ingest(&ctx).await?,
        "obsidian" => heirloom_obsidian::ObsidianIngester.ingest(&ctx).await?,
        "firefox" => heirloom_firefox::FirefoxIngester.ingest(&ctx).await?,
        other => anyhow::bail!(
            "unknown ingester: {} (available: fs, browser, claude, chatgpt, claude-code, slack, obsidian, firefox)",
            other
        ),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("✓ ingest({}) complete", name);
        println!("  scanned:  {}", report.scanned);
        println!("  inserted: {}", report.inserted);
        println!("  skipped:  {}", report.skipped);
        println!("  errors:   {}", report.errors);
    }
    Ok(())
}

async fn cmd_viewer(db_path: &std::path::Path, addr: String) -> Result<()> {
    let store = open_store(db_path)?;
    let addr: std::net::SocketAddr = addr.parse().context("invalid --addr")?;
    heirloom_viewer::serve(store, addr).await
}

async fn cmd_desktop(db_path: &std::path::Path, addr: String) -> Result<()> {
    let store = open_store(db_path)?;
    let addr: std::net::SocketAddr = addr.parse().context("invalid --addr")?;
    heirloom_desktop::launch(store, addr).await
}

fn cmd_seal(db_path: &std::path::Path, passphrase: String) -> Result<()> {
    if !db_path.exists() {
        anyhow::bail!("nothing to seal: {} does not exist", db_path.display());
    }
    let sealed = heirloom_crypto::seal(db_path, &passphrase, false)?;
    println!("✓ sealed: {}", sealed.display());
    println!("  plaintext database removed.");
    println!("  to use again, run: heirloom unseal");
    Ok(())
}

fn cmd_unseal(db_path: &std::path::Path, passphrase: String) -> Result<()> {
    heirloom_crypto::unseal(db_path, &passphrase)?;
    println!("✓ unsealed: {}", db_path.display());
    println!("  use as normal; remember to `heirloom seal` again when done.");
    Ok(())
}

async fn cmd_sync(
    home: &std::path::Path,
    db_path: &std::path::Path,
    action: SyncAction,
) -> Result<()> {
    let mut state = heirloom_sync::SyncState::load(home)?;
    match action {
        SyncAction::Status => {
            println!("device id:    {}", state.device_id.0);
            match &state.relay_url {
                Some(u) => println!("relay url:    {}", u),
                None => {
                    println!("relay url:    (not configured — run `heirloom sync set-relay URL`)")
                }
            }
            match state.last_pulled {
                Some(t) => println!("last pulled:  {}", t.format("%Y-%m-%d %H:%M:%S UTC")),
                None => println!("last pulled:  never"),
            }
            println!("snapshots:    {} known", state.known_snapshots.len());
            println!();
            println!("Note: v0.1 ships the local snapshot pipeline. Network transport against");
            println!("a hosted relay lands in v0.3 — see docs/design/sync-protocol.md.");
            Ok(())
        }
        SyncAction::SetRelay { url } => {
            state.relay_url = url.clone();
            state.save(home)?;
            match url {
                Some(u) => println!("✓ relay set to {}", u),
                None => println!("✓ relay cleared"),
            }
            Ok(())
        }
        SyncAction::Reset => {
            state.device_id = heirloom_sync::DeviceId::new();
            state.save(home)?;
            println!("✓ new device id: {}", state.device_id.0);
            Ok(())
        }
        SyncAction::Push { passphrase } => {
            if !db_path.exists() {
                anyhow::bail!("no database to push at {}", db_path.display());
            }
            let device_id = state.device_id.clone();
            let (header, ciphertext) =
                heirloom_sync::prepare_snapshot(db_path, &passphrase, device_id)?;
            let snapshots_dir = home.join("snapshots");
            std::fs::create_dir_all(&snapshots_dir)?;
            let out = snapshots_dir.join(format!("{}.hlm", header.snapshot_id));
            std::fs::write(&out, ciphertext)?;
            std::fs::write(
                snapshots_dir.join(format!("{}.json", header.snapshot_id)),
                serde_json::to_string_pretty(&header)?,
            )?;
            state.known_snapshots.push(header.snapshot_id.clone());
            state.save(home)?;
            println!("✓ snapshot prepared");
            println!("  id:    {}", header.snapshot_id);
            println!("  size:  {} bytes", header.size_bytes);
            println!("  file:  {}", out.display());
            if state.relay_url.is_some() {
                println!();
                println!("Note: hosted relay upload lands in v1.1 — the snapshot is");
                println!("stored locally. Copy to another device manually for now.");
            }
            Ok(())
        }
        SyncAction::Pull { from, passphrase } => {
            if !from.exists() {
                anyhow::bail!("snapshot file does not exist: {}", from.display());
            }
            let ciphertext = std::fs::read(&from)?;
            // Decrypt the snapshot to a temp DB, then iterate every memory and merge.
            let tmp = tempfile::tempdir()?;
            let incoming_db = heirloom_sync::apply_snapshot(tmp.path(), &ciphertext, &passphrase)?;
            // Open the incoming DB without an embedder — we'll re-embed locally on merge.
            let incoming = Store::open(incoming_db)?;
            let local = open_store(db_path)?;
            let memories = incoming.recent(None, i64::MAX as usize)?;
            let total = memories.len();
            let (inserted, _errs, skipped) =
                heirloom_sync::merge_memories(&local, memories.into_iter())?;
            println!("✓ pulled snapshot from {}", from.display());
            println!("  total in snapshot:  {}", total);
            println!("  newly merged:       {}", inserted);
            println!("  already present:    {}", skipped);
            // Mark the snapshot id (if known) as pulled.
            let meta_path = from.with_extension("json");
            if let Ok(raw) = std::fs::read_to_string(meta_path) {
                if let Ok(header) = serde_json::from_str::<heirloom_sync::SnapshotHeader>(&raw) {
                    if !state.known_snapshots.contains(&header.snapshot_id) {
                        state.known_snapshots.push(header.snapshot_id);
                    }
                    state.last_pulled = Some(chrono::Utc::now());
                    state.save(home)?;
                }
            }
            Ok(())
        }
    }
}

async fn cmd_watch(home: &std::path::Path, db_path: &std::path::Path) -> Result<()> {
    let store = open_store(db_path)?;
    let config = heirloom_watch::load_config(home)?;
    heirloom_watch::run(store, config).await
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct TeamConfig {
    url: String,
    token: String,
}

impl TeamConfig {
    fn load(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading team config {}", path.display()))?;
        Ok(serde_json::from_str(&raw)?)
    }
}

/// Minimal HTTP/1.1 client over tokio TcpStream. We use it instead of a
/// dependency like `ureq` to keep the build dep tree small and avoid the
/// `url`/`idna` crates that need recent Rust editions. HTTPS is intentionally
/// not supported here — self-hosted team servers should sit behind a reverse
/// proxy that terminates TLS (nginx, caddy, traefik, Cloudflare Tunnel, etc.).
mod team_http {
    use anyhow::{anyhow, Context, Result};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::time::{timeout, Duration};

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    const READ_TIMEOUT: Duration = Duration::from_secs(30);
    const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

    pub struct Response {
        pub status: u16,
        pub body: Vec<u8>,
    }

    impl Response {
        pub fn body_text(&self) -> &str {
            std::str::from_utf8(&self.body).unwrap_or("")
        }
        pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
            serde_json::from_slice(&self.body)
                .with_context(|| format!("bad JSON: {}", self.body_text()))
        }
    }

    /// Parse a `http://host:port/path` URL into the pieces we need.
    /// Refuses `https://` so users get a clear error to set up a reverse proxy.
    fn parse_url(url: &str) -> Result<(String, u16, String)> {
        let rest = match url.strip_prefix("http://") {
            Some(r) => r,
            None => {
                if url.starts_with("https://") {
                    anyhow::bail!(
                        "https:// is not supported by the built-in team HTTP client.\n\
                         Run a reverse proxy (nginx, caddy, Cloudflare Tunnel) in front of the team server,\n\
                         then point `heirloom team join` at the http:// address on your private network."
                    );
                }
                anyhow::bail!("url must start with http://");
            }
        };
        let (authority, path_part) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().context("invalid port")?),
            None => (authority.to_string(), 80u16),
        };
        Ok((host, port, path_part.to_string()))
    }

    pub async fn get(url: &str, bearer: Option<&str>) -> Result<Response> {
        request("GET", url, bearer, None).await
    }

    pub async fn post_json(
        url: &str,
        bearer: Option<&str>,
        body: &serde_json::Value,
    ) -> Result<Response> {
        let body_bytes = serde_json::to_vec(body)?;
        request("POST", url, bearer, Some(("application/json", body_bytes))).await
    }

    async fn request(
        method: &str,
        url: &str,
        bearer: Option<&str>,
        body: Option<(&str, Vec<u8>)>,
    ) -> Result<Response> {
        let (host, port, path) = parse_url(url)?;
        let addr = format!("{host}:{port}");
        let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
            .await
            .map_err(|_| anyhow!("connection to {} timed out", addr))?
            .with_context(|| format!("connecting to {}", addr))?;
        let mut stream = stream;

        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\nUser-Agent: heirloom-cli/{ver}\r\n",
            ver = env!("CARGO_PKG_VERSION"),
        );
        if let Some(token) = bearer {
            req.push_str(&format!("Authorization: Bearer {}\r\n", token));
        }
        if let Some((ctype, ref bytes)) = body {
            req.push_str(&format!(
                "Content-Type: {}\r\nContent-Length: {}\r\n",
                ctype,
                bytes.len()
            ));
        }
        req.push_str("\r\n");

        stream.write_all(req.as_bytes()).await?;
        if let Some((_, bytes)) = body {
            stream.write_all(&bytes).await?;
        }
        stream.flush().await?;

        // Read until EOF (we sent Connection: close so the server closes after the body).
        let mut raw = Vec::with_capacity(8192);
        let read_fut = stream.read_to_end(&mut raw);
        match timeout(READ_TIMEOUT, read_fut).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => anyhow::bail!("read from {} timed out", addr),
        }
        if raw.len() > MAX_BODY_BYTES {
            anyhow::bail!("response too large ({} bytes)", raw.len());
        }
        parse_response(&raw)
    }

    fn parse_response(buf: &[u8]) -> Result<Response> {
        let header_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| anyhow!("malformed response (no header terminator)"))?;
        let header_str = std::str::from_utf8(&buf[..header_end])
            .map_err(|_| anyhow!("non-utf8 response headers"))?;
        let status_line = header_str
            .lines()
            .next()
            .ok_or_else(|| anyhow!("empty response"))?;
        // "HTTP/1.1 200 OK"
        let mut parts = status_line.split_whitespace();
        parts.next(); // version
        let status: u16 = parts
            .next()
            .ok_or_else(|| anyhow!("no status code"))?
            .parse()
            .context("bad status code")?;
        let body = buf[(header_end + 4)..].to_vec();
        Ok(Response { status, body })
    }
}

async fn cmd_team(
    home: &std::path::Path,
    db_path: &std::path::Path,
    action: TeamAction,
) -> Result<()> {
    let config_path = home.join("team.json");

    let require_config = || -> Result<TeamConfig> {
        if !config_path.exists() {
            anyhow::bail!("no team config — run `heirloom team join URL --token TOKEN` first");
        }
        TeamConfig::load(&config_path)
    };

    match action {
        TeamAction::Status => {
            if !config_path.exists() {
                println!("Not joined to a team server.");
                println!("Run `heirloom team join URL --token TOKEN` to connect.");
                return Ok(());
            }
            let cfg = TeamConfig::load(&config_path)?;
            println!("Team server:");
            println!("  url:     {}", cfg.url);
            println!("  token:   configured ({} chars)", cfg.token.len());
            println!();
            println!("Use `heirloom team ping` to verify the server is reachable.");
            Ok(())
        }
        TeamAction::Join { url, token } => {
            if !token.starts_with("hlmt_") {
                anyhow::bail!("token does not look like a Heirloom team token (expected hlmt_…)");
            }
            let cfg = TeamConfig {
                url: url.trim_end_matches('/').to_string(),
                token,
            };
            std::fs::write(&config_path, serde_json::to_string_pretty(&cfg)?)?;
            println!("✓ joined {}", cfg.url);
            println!("  config written to {}", config_path.display());
            println!();
            println!("Run `heirloom team ping` to verify the server is reachable.");
            Ok(())
        }
        TeamAction::Leave => {
            if config_path.exists() {
                std::fs::remove_file(&config_path)?;
                println!("✓ removed team config at {}", config_path.display());
            } else {
                println!("Not joined to any team server.");
            }
            Ok(())
        }
        TeamAction::Ping => {
            let cfg = require_config()?;
            let url = format!("{}/v1/health", cfg.url);
            let resp = team_http::get(&url, None)
                .await
                .with_context(|| format!("ping {}", url))?;
            println!("✓ {} → HTTP {}", url, resp.status);
            println!("  body: {}", resp.body_text().trim());
            Ok(())
        }
        TeamAction::Members => {
            let cfg = require_config()?;
            let url = format!("{}/v1/members", cfg.url);
            let resp = team_http::get(&url, Some(&cfg.token))
                .await
                .with_context(|| format!("GET {}", url))?;
            if resp.status >= 400 {
                anyhow::bail!("server returned HTTP {}: {}", resp.status, resp.body_text());
            }
            match serde_json::from_slice::<serde_json::Value>(&resp.body) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                Err(_) => println!("{}", resp.body_text()),
            }
            Ok(())
        }
        TeamAction::Audit { limit } => {
            let cfg = require_config()?;
            let url = format!("{}/v1/audit?limit={}", cfg.url, limit);
            let resp = team_http::get(&url, Some(&cfg.token))
                .await
                .with_context(|| format!("GET {}", url))?;
            if resp.status >= 400 {
                anyhow::bail!("server returned HTTP {}: {}", resp.status, resp.body_text());
            }
            match serde_json::from_slice::<serde_json::Value>(&resp.body) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                Err(_) => println!("{}", resp.body_text()),
            }
            Ok(())
        }
        TeamAction::Push {
            passphrase,
            source,
            limit,
        } => {
            let cfg = require_config()?;
            let store = open_store(db_path)?;
            let memories = store.recent(source.as_deref(), limit)?;
            if memories.is_empty() {
                println!("Nothing to push — store has no memories matching the filter.");
                return Ok(());
            }
            let url = format!("{}/v1/memories", cfg.url);
            let mut pushed = 0u64;
            let mut errors = 0u64;
            for m in &memories {
                let serialized = serde_json::to_vec(m)?;
                let ciphertext = heirloom_crypto::seal_bytes(&serialized, &passphrase)?;
                let tags: Vec<String> = m
                    .metadata
                    .get("tags")
                    .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
                    .unwrap_or_default();
                let body = serde_json::json!({
                    "source": m.source,
                    "kind": m.kind,
                    "ciphertext_hex": hex::encode(&ciphertext),
                    "tags": tags,
                });
                match team_http::post_json(&url, Some(&cfg.token), &body).await {
                    Ok(resp) if resp.status < 400 => pushed += 1,
                    Ok(resp) => {
                        eprintln!(
                            "  ! HTTP {} for memory {}: {}",
                            resp.status,
                            m.id,
                            resp.body_text()
                        );
                        errors += 1;
                    }
                    Err(e) => {
                        eprintln!("  ! failed to push {}: {}", m.id, e);
                        errors += 1;
                    }
                }
            }
            println!("✓ team push complete");
            println!("  pushed:  {}", pushed);
            println!("  errors:  {}", errors);
            Ok(())
        }
        TeamAction::Pull { passphrase, limit } => {
            let cfg = require_config()?;
            let store = open_store(db_path)?;
            let url = format!("{}/v1/memories?limit={}", cfg.url, limit);
            let resp = team_http::get(&url, Some(&cfg.token))
                .await
                .with_context(|| format!("GET {}", url))?;
            if resp.status >= 400 {
                anyhow::bail!("server returned HTTP {}: {}", resp.status, resp.body_text());
            }
            #[derive(serde::Deserialize)]
            struct ListResp {
                memories: Vec<MemoryWire>,
            }
            #[derive(serde::Deserialize)]
            struct MemoryWire {
                ciphertext_hex: String,
            }
            let parsed: ListResp = resp.json()?;
            let mut inserted = 0u64;
            let mut decrypt_failures = 0u64;
            for w in parsed.memories {
                let Ok(ct) = hex::decode(&w.ciphertext_hex) else {
                    decrypt_failures += 1;
                    continue;
                };
                let Ok(plain) = heirloom_crypto::unseal_bytes(&ct, &passphrase) else {
                    decrypt_failures += 1;
                    continue;
                };
                let Ok(memory) = serde_json::from_slice::<heirloom_core::Memory>(&plain) else {
                    decrypt_failures += 1;
                    continue;
                };
                if store.add(&memory)? {
                    inserted += 1;
                }
            }
            println!("✓ team pull complete");
            println!("  newly merged:        {}", inserted);
            println!("  decrypt failures:    {}", decrypt_failures);
            if decrypt_failures > 0 {
                println!();
                println!(
                    "Hint: decrypt failures usually mean the wrong passphrase. Each team member"
                );
                println!("must use the same shared team passphrase to read each other's memories.");
            }
            Ok(())
        }
    }
}

fn cmd_export(
    db_path: &std::path::Path,
    output: Option<PathBuf>,
    source: Option<String>,
) -> Result<()> {
    use std::io::Write;
    let store = open_store(db_path)?;
    let memories = store.recent(source.as_deref(), i64::MAX as usize)?;
    let mut writer: Box<dyn Write> = match &output {
        Some(p) => Box::new(std::fs::File::create(p)?),
        None => Box::new(std::io::stdout().lock()),
    };
    let mut count = 0usize;
    for m in &memories {
        let line = serde_json::to_string(m)?;
        writeln!(writer, "{}", line)?;
        count += 1;
    }
    writer.flush()?;
    if output.is_some() {
        eprintln!(
            "✓ exported {} memor{}",
            count,
            if count == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

async fn cmd_serve(db_path: &std::path::Path) -> Result<()> {
    let store = open_store(db_path)?;
    heirloom_mcp::serve_stdio(store).await
}

fn cmd_status(db_path: &std::path::Path, json: bool) -> Result<()> {
    let store = open_store(db_path)?;
    let total = store.count()?;
    let sources = store.sources()?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "total": total,
                "sources": sources.iter().map(|(n, c)| serde_json::json!({ "name": n, "count": c })).collect::<Vec<_>>(),
                "db_path": db_path.display().to_string(),
            }))?
        );
        return Ok(());
    }
    println!("Heirloom status");
    println!("  database:  {}", db_path.display());
    println!("  memories:  {}", total);
    if sources.is_empty() {
        println!("  sources:   (none yet — try `heirloom ingest fs --path ~/Documents`)");
    } else {
        println!("  sources:");
        for (name, count) in &sources {
            println!("    {:<12} {}", name, count);
        }
    }
    Ok(())
}

fn cmd_recent(
    db_path: &std::path::Path,
    limit: usize,
    source: Option<String>,
    json: bool,
) -> Result<()> {
    let store = open_store(db_path)?;
    let memories = store.recent(source.as_deref(), limit)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&memories)?);
        return Ok(());
    }
    for m in &memories {
        let title = m
            .metadata
            .get("title")
            .cloned()
            .unwrap_or_else(|| m.kind.clone());
        println!(
            "• [{}] {}  ({})",
            m.source,
            title,
            m.created_at.format("%Y-%m-%d %H:%M")
        );
        println!("  id: {}", m.id);
    }
    Ok(())
}

fn cmd_redact(
    db_path: &std::path::Path,
    id: Option<String>,
    query: Option<String>,
    json: bool,
) -> Result<()> {
    let store = open_store(db_path)?;
    let (removed, mode) = match (id, query) {
        (Some(id), None) => {
            let removed = if store.redact(&id)? { 1 } else { 0 };
            (removed, "by-id")
        }
        (None, Some(q)) => {
            let removed = store.redact_query(&q)?;
            (removed, "by-query")
        }
        _ => anyhow::bail!("pass exactly one of --id or --query"),
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "removed": removed, "mode": mode }))?
        );
    } else {
        println!(
            "✓ removed {} memor{}",
            removed,
            if removed == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

fn cmd_reindex(db_path: &std::path::Path) -> Result<()> {
    let store = open_store(db_path)?;
    let n = store.reindex_vectors()?;
    println!("✓ reindex complete");
    println!("  vectors backfilled: {}", n);
    Ok(())
}

fn cmd_doctor(home: &std::path::Path, db_path: &std::path::Path) -> Result<()> {
    let mut ok = true;
    println!("Heirloom doctor");
    println!();

    print!("  home directory writable... ");
    match std::fs::metadata(home) {
        Ok(_) => println!("ok"),
        Err(e) => {
            println!("FAIL: {}", e);
            ok = false;
        }
    }

    print!("  database opens cleanly... ");
    match Store::open(db_path) {
        Ok(_) => println!("ok"),
        Err(e) => {
            println!("FAIL: {}", e);
            ok = false;
        }
    }

    print!("  FTS5 search works... ");
    match Store::in_memory().and_then(|s| {
        let m = Memory::new("doctor", "probe", "smoke test");
        s.add(&m)?;
        s.search("smoke", 1, None)
    }) {
        Ok(hits) if !hits.is_empty() => println!("ok"),
        Ok(_) => {
            println!("FAIL: search returned no hits");
            ok = false;
        }
        Err(e) => {
            println!("FAIL: {}", e);
            ok = false;
        }
    }

    println!();
    if ok {
        println!("All checks passed.");
        Ok(())
    } else {
        anyhow::bail!("doctor found problems");
    }
}
