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
        /// Ingester name (e.g. "fs").
        name: String,
        /// Path option, passed to the ingester.
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Start the MCP server on stdio.
    Serve,

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
        Cmd::Status => cmd_status(&db_path, cli.json),
        Cmd::Recent { limit, source } => cmd_recent(&db_path, limit, source, cli.json),
        Cmd::Redact { id, query } => cmd_redact(&db_path, id, query, cli.json),
        Cmd::Doctor => cmd_doctor(&home, &db_path),
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
        options.insert("path".into(), p.display().to_string());
    }
    let ctx = IngestContext {
        store: store.clone(),
        since: None,
        options,
    };
    let report = match name.as_str() {
        "fs" => FsIngester.ingest(&ctx).await?,
        other => anyhow::bail!("unknown ingester: {} (available: fs)", other),
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
