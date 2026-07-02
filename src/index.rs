use anyhow::{bail, Result};
use bitpacking::{BitPacker, BitPacker4x};

use crate::config::PACK_CHUNK;

const BLOCK_LEN: usize = BitPacker4x::BLOCK_LEN;

pub struct PackedChunk {
    pub num_bits: u8,
    pub data: Vec<u8>,
}

/// Pack a sorted posting list into bitpacking chunks (128 values each, padded tail).
pub fn pack_postings(ids: &[u32]) -> (Vec<PackedChunk>, u32) {
    let packer = BitPacker4x::new();
    let mut chunks = Vec::new();
    let mut initial = 0u32;
    let mut offset = 0usize;

    while offset < ids.len() {
        let remaining = ids.len() - offset;
        let chunk_len = remaining.min(BLOCK_LEN);

        let mut block = [0u32; BLOCK_LEN];
        block[..chunk_len].copy_from_slice(&ids[offset..offset + chunk_len]);
        if chunk_len < BLOCK_LEN {
            let last = block[chunk_len - 1];
            block[chunk_len..].fill(last);
        }

        let num_bits = packer.num_bits_sorted(initial, &block);
        let mut compressed = vec![0u8; BitPacker4x::compressed_block_size(num_bits)];
        let len = packer.compress_sorted(initial, &block, &mut compressed, num_bits);
        compressed.truncate(len);
        chunks.push(PackedChunk { num_bits, data: compressed });

        initial = ids[offset + chunk_len - 1];
        offset += chunk_len;
    }

    (chunks, ids.len() as u32)
}

/// Unpack posting list chunks back into ids (truncated to `num_ids`).
pub fn unpack_postings(chunks: &[(u8, &[u8])], num_ids: u32) -> Result<Vec<u32>> {
    let packer = BitPacker4x::new();
    let mut result = Vec::with_capacity(num_ids as usize);
    let mut initial = 0u32;

    for &(num_bits, data) in chunks {
        if data.len() < BitPacker4x::compressed_block_size(num_bits) {
            bail!("compressed chunk too small");
        }

        let mut block = [0u32; BLOCK_LEN];
        packer.decompress_sorted(initial, data, &mut block, num_bits);
        initial = block[BLOCK_LEN - 1];

        let remaining = num_ids as usize - result.len();
        if remaining >= BLOCK_LEN {
            result.extend_from_slice(&block);
        } else if remaining > 0 {
            result.extend_from_slice(&block[..remaining]);
        }
    }

    result.truncate(num_ids as usize);
    Ok(result)
}

pub fn chunk_block_len() -> usize {
    PACK_CHUNK
}
