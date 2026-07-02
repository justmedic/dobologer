use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use dobologer::api::router;
use dobologer::config::{data_dir, DEFAULT_BIND_ADDR};
use dobologer::engine::{flush_active_on_shutdown, Engine};
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bind_addr: SocketAddr = std::env::var("DOBOLOGER_BIND_ADDR")
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string())
        .parse()
        .context("invalid DOBOLOGER_BIND_ADDR")?;

    let engine = Arc::new(RwLock::new(Engine::open(data_dir())?));
    let app = router(engine.clone());

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind {bind_addr}"))?;

    println!("dobologer listening on http://{bind_addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve")?;

    println!("shutdown signal received, flushing active block...");
    match flush_active_on_shutdown(&engine).await {
        Ok(()) => println!("active block flushed, exiting cleanly"),
        Err(err) => eprintln!("failed to flush active block on shutdown: {err:#}"),
    }

    Ok(())
}

/// Resolve when the process receives Ctrl+C (SIGINT) or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
