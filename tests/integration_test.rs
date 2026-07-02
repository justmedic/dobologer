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
    assert_eq!(beta_results, vec!["beta first block".to_string()]);

    let gamma_results = engine.search("gamma").expect("search gamma");
    assert_eq!(gamma_results, vec!["gamma second block".to_string()]);

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
    let flushed = std::mem::replace(&mut engine.active, ActiveBlock::new(99));
    flush_block(&data_dir, &flushed).expect("flush");
    let reader = BlockReader::open(&data_dir, flushed.id).expect("open");
    engine.sealed.push(reader);

    let results = engine.search("auth_error").expect("search");
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|line| line.contains("auth_error")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flushing_layer_visible_during_search() {
    let dir = temp_dir("flushing");
    let mut engine = Engine::open_with_block_rows(&dir, 2).expect("open engine");
    let result = engine.ingest(vec!["needle one".to_string(), "needle two".to_string()]);
    assert_eq!(result.flushed_blocks.len(), 1);

    // Block is in flushing, not yet sealed
    let results = engine.search("needle").expect("search flushing");
    assert_eq!(results.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn shutdown_flushes_active_block_and_survives_restart() {
    let dir = temp_dir("shutdown");

    // Ingest fewer lines than block_rows so the block stays active (never sealed).
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

    // Reopen the engine from disk: the buffered block must be recovered.
    let reopened = Engine::open_with_block_rows(&dir, 1000).expect("reopen engine");
    let results = reopened.search("persist").expect("search after restart");
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|line| line == "persist me alpha"));
    assert!(results.iter().any(|line| line == "persist me beta"));

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

    // No block files should have been written for an empty active block.
    let reopened = Engine::open_with_block_rows(&dir, 1000).expect("reopen engine");
    assert!(reopened.sealed.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
