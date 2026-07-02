use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpListener, UdpSocket};
use tokio::time::timeout;

use crate::config::{tcp_batch_lines, tcp_flush_ms};
use crate::engine::{FlushCoordinator, SharedEngine};
use crate::ingest::ingest_batch;

/// Accept TCP connections on a pre-bound listener.
pub async fn run_tcp(
    listener: TcpListener,
    engine: SharedEngine,
    flushes: Arc<FlushCoordinator>,
) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(err) => {
                eprintln!("tcp accept error: {err:#}");
                continue;
            }
        };

        let engine = engine.clone();
        let flushes = flushes.clone();

        tokio::spawn(async move {
            if let Err(err) = handle_tcp_connection(stream, engine, flushes).await {
                eprintln!("tcp connection from {peer} error: {err:#}");
            }
        });
    }
}

pub async fn handle_tcp_connection(
    stream: tokio::net::TcpStream,
    engine: SharedEngine,
    flushes: Arc<FlushCoordinator>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut buffer: Vec<String> = Vec::new();
    let max_lines = tcp_batch_lines();
    let flush_ms = tcp_flush_ms();

    loop {
        let mut line = String::new();
        let read_result = if buffer.is_empty() {
            reader.read_line(&mut line).await
        } else {
            match timeout(Duration::from_millis(flush_ms), reader.read_line(&mut line)).await {
                Ok(result) => result,
                Err(_) => {
                    ingest_batch(engine.clone(), flushes.clone(), std::mem::take(&mut buffer)).await;
                    continue;
                }
            }
        };

        match read_result {
            Ok(0) => {
                if !buffer.is_empty() {
                    ingest_batch(engine.clone(), flushes.clone(), std::mem::take(&mut buffer)).await;
                }
                return Ok(());
            }
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
                if !trimmed.is_empty() {
                    buffer.push(trimmed);
                }
                if buffer.len() >= max_lines {
                    ingest_batch(engine.clone(), flushes.clone(), std::mem::take(&mut buffer)).await;
                }
            }
            Err(err) => {
                if !buffer.is_empty() {
                    ingest_batch(engine.clone(), flushes.clone(), std::mem::take(&mut buffer)).await;
                }
                return Err(err.into());
            }
        }
    }
}

/// Receive UDP datagrams on a pre-bound socket.
pub async fn run_udp(
    socket: UdpSocket,
    engine: SharedEngine,
    flushes: Arc<FlushCoordinator>,
) {
    let mut buf = vec![0u8; 65_536];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, _peer)) => {
                let text = String::from_utf8_lossy(&buf[..len]);
                let lines: Vec<String> = text
                    .split('\n')
                    .map(|l| l.trim_end_matches('\r').to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                if !lines.is_empty() {
                    ingest_batch(engine.clone(), flushes.clone(), lines).await;
                }
            }
            Err(err) => {
                eprintln!("udp recv error: {err:#}");
            }
        }
    }
}
