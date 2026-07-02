use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use dobologer::api::router;
use dobologer::config::{data_dir, tcp_addr, udp_addr, DEFAULT_BIND_ADDR};
use dobologer::engine::{flush_active_on_shutdown, FlushCoordinator, Engine};
use dobologer::net::{run_tcp, run_udp};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bind_addr: SocketAddr = std::env::var("DOBOLOGER_BIND_ADDR")
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string())
        .parse()
        .context("invalid DOBOLOGER_BIND_ADDR")?;

    let tcp_bind: SocketAddr = tcp_addr().parse().context("invalid DOBOLOGER_TCP_ADDR")?;
    let udp_bind: SocketAddr = udp_addr().parse().context("invalid DOBOLOGER_UDP_ADDR")?;

    // Bind all ports up front — fail fast, no orphan listeners on partial startup.
    let http_listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind HTTP {bind_addr} (port busy? try: lsof -i :{})", bind_addr.port()))?;
    let tcp_listener = TcpListener::bind(tcp_bind)
        .await
        .with_context(|| format!("bind TCP {tcp_bind} (port busy? try: lsof -i :{})", tcp_bind.port()))?;
    let udp_socket = UdpSocket::bind(udp_bind)
        .await
        .with_context(|| format!("bind UDP {udp_bind} (port busy? try: lsof -i :{})", udp_bind.port()))?;

    println!("dobologer listening on http://{bind_addr}");
    println!("dobologer tcp listening on {tcp_bind}");
    println!("dobologer udp listening on {udp_bind}");

    let engine = Arc::new(RwLock::new(Engine::open(data_dir())?));
    let flushes = Arc::new(FlushCoordinator::new());
    let app = router(engine.clone(), flushes.clone());

    tokio::spawn(run_tcp(tcp_listener, engine.clone(), flushes.clone()));
    tokio::spawn(run_udp(udp_socket, engine.clone(), flushes.clone()));

    axum::serve(http_listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve")?;

    println!("shutdown signal received, waiting for background flushes...");
    flushes.wait_all().await;

    println!("flushing active block...");
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
