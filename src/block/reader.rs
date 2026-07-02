use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use bitpacking::{BitPacker, BitPacker4x};
use memmap2::Mmap;

use crate::block::format::{
    block_paths, load_dictionary, parse_data_header, parse_idx_footer, DataHeader, DictEntry,
};
use crate::index::unpack_postings;

pub struct BlockReader {
    pub block_id: u64,
    idx_mmap: Mmap,
    data_mmap: Mmap,
    dictionary: HashMap<String, DictEntry>,
    data_header: DataHeader,
}

impl BlockReader {
    pub fn open(data_dir: &Path, block_id: u64) -> Result<Self> {
        let (_meta_path, data_path, idx_path) = block_paths(data_dir, block_id);

        let idx_file = File::open(&idx_path).with_context(|| format!("open idx {idx_path:?}"))?;
        let idx_mmap = unsafe { Mmap::map(&idx_file)? };

        let data_file = File::open(&data_path).with_context(|| format!("open data {data_path:?}"))?;
        let data_mmap = unsafe { Mmap::map(&data_file)? };

        let (dict_offset, num_terms) = parse_idx_footer(&idx_mmap)?;
        let dictionary = load_dictionary(&idx_mmap, dict_offset, num_terms)?;
        let data_header = parse_data_header(&data_mmap)?;

        Ok(Self {
            block_id,
            idx_mmap,
            data_mmap,
            dictionary,
            data_header,
        })
    }

    pub fn search_token(&self, token: &str) -> Result<Vec<u32>> {
        let Some(entry) = self.dictionary.get(token) else {
            return Ok(Vec::new());
        };

        let mut chunks = Vec::with_capacity(entry.num_chunks as usize);
        let mut offset = entry.posting_offset as usize;

        for _ in 0..entry.num_chunks {
            if offset >= self.idx_mmap.len() {
                break;
            }
            let num_bits = self.idx_mmap[offset];
            offset += 1;
            let chunk_size = BitPacker4x::compressed_block_size(num_bits);
            let end = offset + chunk_size;
            if end > self.idx_mmap.len() {
                break;
            }
            let data = &self.idx_mmap[offset..end];
            let owned = data.to_vec();
            chunks.push((num_bits, owned));
            offset = end;
        }

        let chunk_refs: Vec<(u8, &[u8])> = chunks.iter().map(|(nb, data)| (*nb, data.as_slice())).collect();
        unpack_postings(&chunk_refs, entry.num_ids)
    }

    pub fn fetch_lines(&self, ids: &[u32]) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows_per_pack = self.data_header.rows_per_pack as usize;
        let mut packs_needed: HashSet<usize> = HashSet::new();
        for &id in ids {
            packs_needed.insert((id as usize) / rows_per_pack);
        }

        let mut decompressed_packs: HashMap<usize, Vec<u8>> = HashMap::new();
        for pack_idx in packs_needed {
            decompressed_packs.insert(pack_idx, self.decompress_pack(pack_idx)?);
        }

        let mut results = Vec::with_capacity(ids.len());
        for &id in ids {
            let line_id = id as usize;
            let pack_idx = line_id / rows_per_pack;
            let pack = decompressed_packs
                .get(&pack_idx)
                .context("missing decompressed pack")?;

            let start = self.data_header.line_offsets[line_id] as usize;
            let end = if line_id + 1 < self.data_header.num_lines as usize
                && (line_id + 1) / rows_per_pack == pack_idx
            {
                self.data_header.line_offsets[line_id + 1] as usize
            } else {
                pack.len()
            };

            let line = std::str::from_utf8(&pack[start..end])
                .context("invalid utf8 in stored line")?
                .to_string();
            results.push(line);
        }

        Ok(results)
    }

    fn decompress_pack(&self, pack_idx: usize) -> Result<Vec<u8>> {
        let start = self.data_header.pack_offsets[pack_idx] as usize;
        let end = self.data_header.pack_offsets[pack_idx + 1] as usize;
        let compressed = &self.data_mmap[start..end];
        zstd::decode_all(compressed).context("zstd decompress data pack")
    }
}

pub fn list_block_ids(data_dir: &Path) -> Result<Vec<u64>> {
    let mut block_ids = Vec::new();
    if !data_dir.exists() {
        return Ok(block_ids);
    }

    for entry in std::fs::read_dir(data_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("meta") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(id) = stem.parse::<u64>() {
                    block_ids.push(id);
                }
            }
        }
    }

    block_ids.sort_unstable();
    Ok(block_ids)
}
