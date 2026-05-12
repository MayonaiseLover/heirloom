//! # heirloom-watch
//!
//! Background daemon that runs configured ingesters on a schedule. Reads
//! `~/.heirloom/config.toml` for the schedule and per-ingester options.
//!
//! ## Example config
//!
//! ```toml
//! [watch]
//! interval_minutes = 60
//!
//! [[watch.tasks]]
//! ingester = "fs"
//! path = "/Users/me/Documents/notes"
//!
//! [[watch.tasks]]
//! ingester = "browser"
//!
//! [[watch.tasks]]
//! ingester = "claude-code"
//! ```

use anyhow::{Context, Result};
use heirloom_core::Store;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub watch: WatchConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchConfig {
    #[serde(default = "default_interval")]
    pub interval_minutes: u64,
    #[serde(default)]
    pub tasks: Vec<Task>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            interval_minutes: default_interval(),
            tasks: Vec::new(),
        }
    }
}

fn default_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub ingester: String,
    #[serde(flatten)]
    pub options: HashMap<String, toml::Value>,
}

impl Task {
    fn string_options(&self) -> HashMap<String, String> {
        self.options
            .iter()
            .map(|(k, v)| {
                let s = match v {
                    toml::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), s)
            })
            .collect()
    }
}

pub fn load_config(home: &Path) -> Result<Config> {
    let path = home.join("config.toml");
    if !path.exists() {
        return Ok(Config {
            watch: WatchConfig::default(),
        });
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: Config =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(cfg)
}

/// Run the watch loop until interrupted.
pub async fn run(store: Arc<Store>, config: Config) -> Result<()> {
    let interval = Duration::from_secs(config.watch.interval_minutes.max(1) * 60);
    info!(
        "watch loop starting — {} tasks, interval {}m",
        config.watch.tasks.len(),
        config.watch.interval_minutes
    );
    if config.watch.tasks.is_empty() {
        warn!("no [[watch.tasks]] in config.toml — daemon will idle");
    }

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                run_once(&store, &config).await;
            }
            _ = &mut shutdown => {
                info!("watch loop received Ctrl-C, exiting");
                return Ok(());
            }
        }
    }
}

async fn run_once(store: &Arc<Store>, config: &Config) {
    for task in &config.watch.tasks {
        let store = store.clone();
        let opts = task.string_options();
        let ingester = task.ingester.clone();
        match run_task(store, &ingester, opts).await {
            Ok(report) => {
                info!(target: "heirloom_watch", ingester = %ingester, ?report, "task complete")
            }
            Err(e) => error!(target: "heirloom_watch", ingester = %ingester, "task failed: {}", e),
        }
    }
}

async fn run_task(
    store: Arc<Store>,
    name: &str,
    options: HashMap<String, String>,
) -> Result<IngestReport> {
    let ctx = IngestContext {
        store,
        since: None,
        options,
    };
    match name {
        "fs" => Ok(heirloom_fs::FsIngester.ingest(&ctx).await?),
        "browser" => Ok(heirloom_browser::BrowserIngester.ingest(&ctx).await?),
        "claude-code" => Ok(heirloom_claude_code::ClaudeCodeIngester
            .ingest(&ctx)
            .await?),
        other => anyhow::bail!(
            "unknown ingester '{}'. Built-in watch ingesters: fs, browser, claude-code",
            other
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_with_tasks() {
        let toml_text = r#"
[watch]
interval_minutes = 15

[[watch.tasks]]
ingester = "fs"
path = "/home/me/notes"

[[watch.tasks]]
ingester = "browser"
"#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(cfg.watch.interval_minutes, 15);
        assert_eq!(cfg.watch.tasks.len(), 2);
        assert_eq!(cfg.watch.tasks[0].ingester, "fs");
        assert_eq!(
            cfg.watch.tasks[0]
                .options
                .get("path")
                .unwrap()
                .as_str()
                .unwrap(),
            "/home/me/notes"
        );
    }

    #[test]
    fn empty_config_yields_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.watch.interval_minutes, 60);
        assert!(cfg.watch.tasks.is_empty());
    }
}
