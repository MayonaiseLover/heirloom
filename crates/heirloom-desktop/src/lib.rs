//! # heirloom-desktop
//!
//! Runs the embedded web viewer and opens it in the user's default browser.
//! No native GUI dependencies — works on every platform.
//!
//! A full native window (via `wry`/`tao`) is on the v1.0 roadmap; it
//! requires WebKitGTK on Linux which materially complicates packaging.
//! For now this is the right tradeoff: same UI, zero extra deps.

use anyhow::Result;
use heirloom_core::Store;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

pub async fn launch(store: Arc<Store>, addr: SocketAddr) -> Result<()> {
    let url = format!("http://{}", addr);
    // Spawn the viewer; give it a moment to bind before we open the browser.
    let viewer_store = store.clone();
    let viewer_addr = addr;
    let server = tokio::spawn(async move {
        let _ = heirloom_viewer::serve(viewer_store, viewer_addr).await;
    });

    tokio::time::sleep(Duration::from_millis(250)).await;
    if let Err(e) = open_browser(&url) {
        info!(
            "could not auto-open browser ({}). Open this manually: {}",
            e, url
        );
    }

    server.await?;
    Ok(())
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "windows")]
    let prog = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let prog = "xdg-open";

    let status = std::process::Command::new(prog).arg(url).status()?;
    if !status.success() {
        anyhow::bail!("browser launcher exited with {}", status);
    }
    Ok(())
}
