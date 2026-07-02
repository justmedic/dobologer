use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::block::active::ActiveBlock;
use crate::block::reader::{list_block_ids, BlockReader};
use crate::block::writer::flush_block;
use crate::config::block_rows;
use crate::tokenizer::normalize_token;

pub struct Engine {
    pub data_dir: PathBuf,
    pub active: ActiveBlock,
    pub flushing: Vec<Arc<ActiveBlock>>,
    pub sealed: Vec<BlockReader>,
    block_rows: usize,
    next_block_id: u64,
}

pub struct IngestResult {
    pub ingested: usize,
    pub flushed_blocks: Vec<Arc<ActiveBlock>>,
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
            sealed.push(BlockReader::open(&data_dir, *block_id)?);
        }

        let next_block_id = block_ids.last().map(|id| id + 1).unwrap_or(0);

        Ok(Self {
            data_dir,
            active: ActiveBlock::new(next_block_id),
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
            self.active.push(line);
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
        let full = Arc::new(std::mem::replace(
            &mut self.active,
            ActiveBlock::new(self.next_block_id),
        ));
        self.flushing.push(Arc::clone(&full));
        full
    }

    pub fn complete_flush(&mut self, block_id: u64, reader: BlockReader) {
        self.flushing.retain(|block| block.id != block_id);
        self.sealed.push(reader);
    }

    pub fn search(&self, query: &str) -> Result<Vec<String>> {
        let mut scratch = String::new();
        let token = crate::tokenizer::tokenize(query)
            .next()
            .map(|t| normalize_token(t, &mut scratch))
            .unwrap_or("");
        let mut results = Vec::new();

        results.extend(self.active.search_token(token));
        for block in &self.flushing {
            results.extend(block.search_token(token));
        }
        for reader in &self.sealed {
            let ids = reader.search_token(token)?;
            results.extend(reader.fetch_lines(&ids)?);
        }

        Ok(results)
    }
}

pub type SharedEngine = Arc<RwLock<Engine>>;

/// Flush the still-open active block to disk so buffered logs survive shutdown.
/// Blocks in the `flushing` layer are written by their own background tasks
/// (awaited by the graceful-shutdown request drain), so only the active block
/// needs an explicit final flush here.
pub async fn flush_active_on_shutdown(engine: &SharedEngine) -> Result<()> {
    let guard = engine.read().await;
    if guard.active.num_lines() == 0 {
        return Ok(());
    }
    flush_block(&guard.data_dir, &guard.active)?;
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
    .await??;

    let mut guard = engine.write().await;
    guard.complete_flush(block_id, reader);
    Ok(())
}
