use std::sync::Arc;

use dobologer::block::active::ActiveBlock;
use dobologer::block::reader::BlockReader;
use dobologer::block::writer::flush_block;
use dobologer::engine::{flush_active_on_shutdown, Engine};
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
