use std::sync::Arc;
use std::time::Duration;

use dobologer::block::active::ActiveBlock;
use dobologer::block::reader::BlockReader;
use dobologer::block::writer::flush_block;
use dobologer::engine::{flush_active_on_shutdown, FlushCoordinator, Engine};
use dobologer::ingest::ingest_batch;
use dobologer::net::handle_tcp_connection;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::RwLock;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dobologer_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn cross_block_search_smoke() {
    let dir = temp_dir("cross_block");
    let mut engine = Engine::open_with_block_rows(&dir, 2).expect("open engine");

    let result = engine.ingest(vec![
        "alpha first block".to_string(),
        "beta first block".to_string(),
        "gamma second block".to_string(),
    ]);
    assert_eq!(result.ingested, 3);
    assert_eq!(result.flushed_blocks.len(), 1);

    for block in result.flushed_blocks {
        let data_dir = engine.data_dir.clone();
        let block_id = block.id;
        flush_block(&data_dir, block.as_ref()).expect("flush");
        let reader = BlockReader::open(&data_dir, block_id).expect("open reader");
        engine.complete_flush(block_id, reader);
    }

    let beta_results = engine.search("beta").expect("search beta");
    assert_eq!(beta_results.results, vec!["beta first block".to_string()]);

    let gamma_results = engine.search("gamma").expect("search gamma");
    assert_eq!(gamma_results.results, vec!["gamma second block".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sealed_block_search_after_flush() {
    let dir = temp_dir("sealed");
    let mut engine = Engine::open_with_block_rows(&dir, 8).expect("open engine");
    engine.ingest(vec![
        "auth_error login failed".to_string(),
        "auth_error token expired".to_string(),
        "user created successfully".to_string(),
    ]);

    let data_dir = engine.data_dir.clone();
    let flushed = std::mem::replace(&mut engine.active, Arc::new(ActiveBlock::new(99)));
    flush_block(&data_dir, flushed.as_ref()).expect("flush");
    let reader = BlockReader::open(&data_dir, flushed.id).expect("open");
    engine.sealed.push(Arc::new(reader));

    let results = engine.search("auth_error").expect("search");
    assert_eq!(results.total, 2);
    assert_eq!(results.results.len(), 2);
    assert!(results.results.iter().all(|line| line.contains("auth_error")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flushing_layer_visible_during_search() {
    let dir = temp_dir("flushing");
    let mut engine = Engine::open_with_block_rows(&dir, 2).expect("open engine");
    let result = engine.ingest(vec!["needle one".to_string(), "needle two".to_string()]);
    assert_eq!(result.flushed_blocks.len(), 1);

    let results = engine.search("needle").expect("search flushing");
    assert_eq!(results.total, 2);
    assert_eq!(results.results.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_limit_returns_subset_with_total() {
    let dir = temp_dir("limit");
    let mut engine = Engine::open_with_block_rows(&dir, 100).expect("open engine");
    engine.ingest((1..=10).map(|i| format!("error line {i}")).collect());

    let results = engine
        .search_with_limit("error", Some(3))
        .expect("search with limit");
    assert_eq!(results.total, 10);
    assert_eq!(results.results.len(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tokenizer_splits_key_value_pairs() {
    let dir = temp_dir("tokenizer");
    let mut engine = Engine::open_with_block_rows(&dir, 100).expect("open engine");
    engine.ingest(vec!["env=prod region=us-east-1".to_string()]);

    let prod = engine.search("prod").expect("search prod");
    assert_eq!(prod.total, 1);

    let env = engine.search("env").expect("search env");
    assert_eq!(env.total, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn shutdown_flushes_active_block_and_survives_restart() {
    let dir = temp_dir("shutdown");

    {
        let engine = Arc::new(RwLock::new(
            Engine::open_with_block_rows(&dir, 1000).expect("open engine"),
        ));
        {
            let mut guard = engine.write().await;
            guard.ingest(vec![
                "persist me alpha".to_string(),
                "persist me beta".to_string(),
            ]);
        }
        flush_active_on_shutdown(&engine)
            .await
            .expect("flush active on shutdown");
    }

    let reopened = Engine::open_with_block_rows(&dir, 1000).expect("reopen engine");
    let results = reopened.search("persist").expect("search after restart");
    assert_eq!(results.total, 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn shutdown_with_empty_active_block_is_noop() {
    let dir = temp_dir("shutdown_empty");

    let engine = Arc::new(RwLock::new(
        Engine::open_with_block_rows(&dir, 1000).expect("open engine"),
    ));
    flush_active_on_shutdown(&engine)
        .await
        .expect("flush empty active");

    let reopened = Engine::open_with_block_rows(&dir, 1000).expect("reopen engine");
    assert!(reopened.sealed.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn boolean_search_and_or_not() {
    let dir = temp_dir("boolean");
    let mut engine = Engine::open_with_block_rows(&dir, 100).expect("open engine");
    engine.ingest(vec![
        "auth error login failed".to_string(),
        "auth success login ok".to_string(),
        "db error connection timeout".to_string(),
        "user created successfully".to_string(),
    ]);

    let and_results = engine.search("auth AND error").expect("and search");
    assert_eq!(and_results.total, 1);
    assert!(and_results.results[0].contains("login failed"));

    let or_results = engine.search("error OR success").expect("or search");
    assert_eq!(or_results.total, 3);

    let not_results = engine.search("auth AND NOT error").expect("not search");
    assert_eq!(not_results.total, 1);
    assert!(not_results.results[0].contains("success"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn boolean_search_cross_block() {
    let dir = temp_dir("boolean_cross");
    let mut engine = Engine::open_with_block_rows(&dir, 2).expect("open engine");

    let result = engine.ingest(vec![
        "alpha needle".to_string(),
        "beta needle".to_string(),
        "gamma needle".to_string(),
    ]);
    assert_eq!(result.flushed_blocks.len(), 1);

    for block in result.flushed_blocks {
        let data_dir = engine.data_dir.clone();
        let block_id = block.id;
        flush_block(&data_dir, block.as_ref()).expect("flush");
        let reader = BlockReader::open(&data_dir, block_id).expect("open reader");
        engine.complete_flush(block_id, reader);
    }

    let results = engine.search("alpha AND needle").expect("cross block and");
    assert_eq!(results.total, 1);
    assert_eq!(results.results[0], "alpha needle");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn tcp_ingest_and_search() {
    let dir = temp_dir("tcp");
    let engine = Arc::new(RwLock::new(
        Engine::open_with_block_rows(&dir, 100).expect("open engine"),
    ));
    let flushes = Arc::new(FlushCoordinator::new());

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind tcp");
    let addr = listener.local_addr().expect("local addr");

    let engine_bg = engine.clone();
    let flushes_bg = flushes.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        handle_tcp_connection(stream, engine_bg, flushes_bg)
            .await
            .expect("handle tcp");
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"tcp_alpha line\ntcp_beta line\n")
        .await
        .expect("write");
    stream.shutdown().await.expect("shutdown");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let results = engine.read().await.search("tcp_alpha").expect("search");
    assert_eq!(results.total, 1);
    assert_eq!(results.results[0], "tcp_alpha line");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn udp_ingest_and_search() {
    let dir = temp_dir("udp");
    let engine = Arc::new(RwLock::new(
        Engine::open_with_block_rows(&dir, 100).expect("open engine"),
    ));
    let flushes = Arc::new(FlushCoordinator::new());

    ingest_batch(
        engine.clone(),
        flushes,
        vec!["udp_gamma line".to_string(), "udp_delta line".to_string()],
    )
    .await;

    let results = engine.read().await.search("udp_gamma").expect("search");
    assert_eq!(results.total, 1);
    assert_eq!(results.results[0], "udp_gamma line");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn udp_socket_ingest_and_search() {
    let dir = temp_dir("udp_socket");
    let engine = Arc::new(RwLock::new(
        Engine::open_with_block_rows(&dir, 100).expect("open engine"),
    ));
    let flushes = Arc::new(FlushCoordinator::new());

    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
    let addr = socket.local_addr().expect("local addr");

    let engine_bg = engine.clone();
    let flushes_bg = flushes.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        let (len, _) = socket.recv_from(&mut buf).await.expect("recv");
        let text = String::from_utf8_lossy(&buf[..len]);
        let lines: Vec<String> = text
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        ingest_batch(engine_bg, flushes_bg, lines).await;
    });

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("bind client");
    client
        .send_to(b"udp_needle line\n", addr)
        .await
        .expect("send");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let results = engine.read().await.search("udp_needle").expect("search");
    assert_eq!(results.total, 1);

    let _ = std::fs::remove_dir_all(&dir);
}
