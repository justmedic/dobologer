use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::block::active::ActiveBlock;
use crate::block::reader::{list_block_ids, BlockReader};
use crate::block::writer::flush_block;
use crate::config::block_rows;
use crate::tokenizer::first_token;

pub struct Engine {
    pub data_dir: PathBuf,
    pub active: Arc<ActiveBlock>,
    pub flushing: Vec<Arc<ActiveBlock>>,
    pub sealed: Vec<Arc<BlockReader>>,
    block_rows: usize,
    next_block_id: u64,
}

pub struct IngestResult {
    pub ingested: usize,
    pub flushed_blocks: Vec<Arc<ActiveBlock>>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub total: usize,
    pub results: Vec<String>,
}

struct SearchSnapshot {
    active: Arc<ActiveBlock>,
    flushing: Vec<Arc<ActiveBlock>>,
    sealed: Vec<Arc<BlockReader>>,
    token: String,
    limit: Option<usize>,
}

impl SearchSnapshot {
    fn execute(self) -> Result<SearchResult> {
        let token = self.token.as_str();
        let max = self.limit.unwrap_or(usize::MAX);

        let mut total = 0usize;
        total += self.active.count_for_token(token);
        for block in &self.flushing {
            total += block.count_for_token(token);
        }

        let mut sealed_jobs = Vec::with_capacity(self.sealed.len());
        for reader in &self.sealed {
            let ids = reader.search_token(token)?;
            total += ids.len();
            if !ids.is_empty() {
                sealed_jobs.push((Arc::clone(reader), ids));
            }
        }

        let mut results = self.active.lines_for_token(token, max);
        if results.len() < max {
            for block in &self.flushing {
                let remaining = max - results.len();
                results.extend(block.lines_for_token(token, remaining));
                if results.len() >= max {
                    break;
                }
            }
        }

        if results.len() < max {
            for (reader, ids) in sealed_jobs {
                let remaining = max - results.len();
                let fetch_ids = if ids.len() > remaining {
                    &ids[..remaining]
                } else {
                    &ids[..]
                };
                results.extend(reader.fetch_lines(fetch_ids)?);
                if results.len() >= max {
                    break;
                }
            }
        }

        Ok(SearchResult { total, results })
    }
}

impl Engine {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_block_rows(data_dir, block_rows())
    }

    pub fn open_with_block_rows(data_dir: impl AsRef<Path>, block_rows: usize) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        let block_ids = list_block_ids(&data_dir)?;
        let mut sealed = Vec::with_capacity(block_ids.len());
        for block_id in &block_ids {
            sealed.push(Arc::new(BlockReader::open(&data_dir, *block_id)?));
        }

        let next_block_id = block_ids.last().map(|id| id + 1).unwrap_or(0);

        Ok(Self {
            data_dir,
            active: Arc::new(ActiveBlock::new(next_block_id)),
            flushing: Vec::new(),
            sealed,
            block_rows: block_rows.max(1),
            next_block_id,
        })
    }

    pub fn ingest(&mut self, lines: Vec<String>) -> IngestResult {
        let mut ingested = 0usize;
        let mut flushed_blocks = Vec::new();

        for line in lines {
            Arc::make_mut(&mut self.active).push(line);
            ingested += 1;

            if self.active.num_lines() >= self.block_rows {
                flushed_blocks.push(self.rotate_active());
            }
        }

        IngestResult {
            ingested,
            flushed_blocks,
        }
    }

    fn rotate_active(&mut self) -> Arc<ActiveBlock> {
        self.next_block_id = self.active.id.saturating_add(1);
        let full = std::mem::replace(
            &mut self.active,
            Arc::new(ActiveBlock::new(self.next_block_id)),
        );
        self.flushing.push(Arc::clone(&full));
        full
    }

    pub fn complete_flush(&mut self, block_id: u64, reader: BlockReader) {
        self.flushing.retain(|block| block.id != block_id);
        self.sealed.push(Arc::new(reader));
    }

    pub fn search(&self, query: &str) -> Result<SearchResult> {
        self.search_with_limit(query, None)
    }

    pub fn search_with_limit(&self, query: &str, limit: Option<usize>) -> Result<SearchResult> {
        let mut scratch = String::new();
        let token = first_token(query, &mut scratch).to_string();

        SearchSnapshot {
            active: Arc::clone(&self.active),
            flushing: self.flushing.clone(),
            sealed: self.sealed.clone(),
            token,
            limit,
        }
        .execute()
    }
}

pub type SharedEngine = Arc<RwLock<Engine>>;

pub async fn search_async(
    engine: &SharedEngine,
    query: &str,
    limit: Option<usize>,
) -> Result<SearchResult> {
    let mut scratch = String::new();
    let token = first_token(query, &mut scratch).to_string();

    let snapshot = {
        let guard = engine.read().await;
        SearchSnapshot {
            active: Arc::clone(&guard.active),
            flushing: guard.flushing.clone(),
            sealed: guard.sealed.clone(),
            token,
            limit,
        }
    };

    tokio::task::spawn_blocking(move || snapshot.execute())
        .await
        .map_err(|err| anyhow::anyhow!("search task failed: {err}"))?
}

/// Tracks in-flight background block flushes.
pub struct FlushCoordinator {
    pending: AtomicUsize,
}

impl FlushCoordinator {
    pub fn new() -> Self {
        Self {
            pending: AtomicUsize::new(0),
        }
    }

    pub fn spawn_detached_flush(
        self: &Arc<Self>,
        engine: SharedEngine,
        block: Arc<ActiveBlock>,
    ) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        let coordinator = Arc::clone(self);

        tokio::spawn(async move {
            let result = flush_block_async(engine, block).await;
            coordinator.pending.fetch_sub(1, Ordering::SeqCst);
            if let Err(err) = result {
                eprintln!("background flush failed: {err:#}");
            }
        });
    }

    pub async fn wait_all(&self) {
        while self.pending.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

impl Default for FlushCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Flush the still-open active block to disk so buffered logs survive shutdown.
pub async fn flush_active_on_shutdown(engine: &SharedEngine) -> Result<()> {
    let guard = engine.read().await;
    if guard.active.num_lines() == 0 {
        return Ok(());
    }
    flush_block(&guard.data_dir, guard.active.as_ref())?;
    Ok(())
}

pub async fn flush_block_async(engine: SharedEngine, block: Arc<ActiveBlock>) -> Result<()> {
    let data_dir = {
        let guard = engine.read().await;
        guard.data_dir.clone()
    };

    let block_id = block.id;
    let reader = tokio::task::spawn_blocking(move || {
        flush_block(&data_dir, block.as_ref())?;
        BlockReader::open(&data_dir, block_id)
    })
    .await
    .map_err(|err| anyhow::anyhow!("flush task failed: {err}"))??;

    let mut guard = engine.write().await;
    guard.complete_flush(block_id, reader);
    Ok(())
}

pub fn spawn_detached_flushes(
    coordinator: &Arc<FlushCoordinator>,
    engine: SharedEngine,
    blocks: Vec<Arc<ActiveBlock>>,
) {
    for block in blocks {
        coordinator.spawn_detached_flush(engine.clone(), block);
    }
}
