use anyhow::{bail, Result};
use bitpacking::{BitPacker, BitPacker4x};

use crate::config::PACK_CHUNK;

const BLOCK_LEN: usize = BitPacker4x::BLOCK_LEN;

pub enum PostingChunk {
    BitPacked { num_bits: u8, data: Vec<u8> },
    VarInt { count: u32, data: Vec<u8> },
}

/// Pack a sorted posting list: full blocks of 128 via bitpacking, tail via varint deltas.
pub fn pack_postings(ids: &[u32]) -> (Vec<PostingChunk>, u32) {
    let packer = BitPacker4x::new();
    let mut chunks = Vec::new();
    let mut initial = 0u32;
    let mut offset = 0usize;

    while offset + BLOCK_LEN <= ids.len() {
        let mut block = [0u32; BLOCK_LEN];
        block.copy_from_slice(&ids[offset..offset + BLOCK_LEN]);

        let num_bits = packer.num_bits_sorted(initial, &block);
        let mut compressed = vec![0u8; BitPacker4x::compressed_block_size(num_bits)];
        let len = packer.compress_sorted(initial, &block, &mut compressed, num_bits);
        compressed.truncate(len);
        chunks.push(PostingChunk::BitPacked {
            num_bits,
            data: compressed,
        });

        initial = block[BLOCK_LEN - 1];
        offset += BLOCK_LEN;
    }

    if offset < ids.len() {
        let tail = &ids[offset..];
        chunks.push(PostingChunk::VarInt {
            count: tail.len() as u32,
            data: encode_varint_deltas(initial, tail),
        });
    }

    (chunks, ids.len() as u32)
}

pub fn write_posting_chunk(chunk: &PostingChunk) -> Vec<u8> {
    let mut out = Vec::new();
    match chunk {
        PostingChunk::BitPacked { num_bits, data } => {
            out.push(*num_bits);
            out.extend_from_slice(data);
        }
        PostingChunk::VarInt { count, data } => {
            out.push(0);
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(data);
        }
    }
    out
}

pub fn posting_chunk_byte_len(chunk: &PostingChunk) -> usize {
    match chunk {
        PostingChunk::BitPacked { data, .. } => 1 + data.len(),
        PostingChunk::VarInt { data, .. } => 1 + 4 + data.len(),
    }
}

/// Decode all chunks for one term into ids (truncated to `num_ids`).
pub fn unpack_posting_chunks(chunks: &[PostingChunk], num_ids: u32) -> Result<Vec<u32>> {
    let packer = BitPacker4x::new();
    let mut result = Vec::with_capacity(num_ids as usize);
    let mut initial = 0u32;

    for chunk in chunks {
        if result.len() >= num_ids as usize {
            break;
        }

        match chunk {
            PostingChunk::BitPacked { num_bits, data } => {
                if *num_bits == 0 {
                    bail!("invalid bitpacked chunk with num_bits=0");
                }
                if data.len() < BitPacker4x::compressed_block_size(*num_bits) {
                    bail!("compressed chunk too small");
                }

                let mut block = [0u32; BLOCK_LEN];
                packer.decompress_sorted(initial, data, &mut block, *num_bits);
                initial = block[BLOCK_LEN - 1];

                let remaining = num_ids as usize - result.len();
                let take = remaining.min(BLOCK_LEN);
                result.extend_from_slice(&block[..take]);
            }
            PostingChunk::VarInt { count, data } => {
                let (ids, _) = decode_varint_deltas(initial, data, *count as usize)?;
                if let Some(&last) = ids.last() {
                    initial = last;
                }
                let remaining = num_ids as usize - result.len();
                let take = remaining.min(ids.len());
                result.extend_from_slice(&ids[..take]);
            }
        }
    }

    result.truncate(num_ids as usize);
    Ok(result)
}

/// Parse one on-disk posting chunk starting at `data[0]`. Returns chunk and bytes consumed.
pub fn parse_posting_chunk(data: &[u8]) -> Result<(PostingChunk, usize)> {
    if data.is_empty() {
        bail!("empty posting chunk");
    }

    if data[0] == 0 {
        if data.len() < 5 {
            bail!("truncated varint posting header");
        }
        let count = u32::from_le_bytes(data[1..5].try_into()?);
        let payload = &data[5..];
        let (_, consumed) = decode_varint_deltas(0, payload, count as usize)?;
        Ok((
            PostingChunk::VarInt {
                count,
                data: payload[..consumed].to_vec(),
            },
            1 + 4 + consumed,
        ))
    } else {
        let num_bits = data[0];
        let size = BitPacker4x::compressed_block_size(num_bits);
        if data.len() < 1 + size {
            bail!("truncated bitpacked posting chunk");
        }
        Ok((
            PostingChunk::BitPacked {
                num_bits,
                data: data[1..1 + size].to_vec(),
            },
            1 + size,
        ))
    }
}

fn encode_varint_deltas(initial: u32, ids: &[u32]) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut prev = initial;
    for &id in ids {
        encode_varint(id - prev, &mut buf);
        prev = id;
    }
    buf
}

fn decode_varint_deltas(initial: u32, data: &[u8], count: usize) -> Result<(Vec<u32>, usize)> {
    let mut result = Vec::with_capacity(count);
    let mut prev = initial;
    let mut offset = 0usize;

    for _ in 0..count {
        let (delta, n) = decode_varint(&data[offset..])?;
        offset += n;
        prev = prev.wrapping_add(delta);
        result.push(prev);
    }

    Ok((result, offset))
}

fn encode_varint(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn decode_varint(data: &[u8]) -> Result<(u32, usize)> {
    let mut value = 0u32;
    let mut shift = 0u32;
    for (i, &byte) in data.iter().enumerate() {
        value |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
        shift += 7;
        if shift > 35 {
            bail!("varint overflow");
        }
    }
    bail!("truncated varint");
}

pub fn chunk_block_len() -> usize {
    PACK_CHUNK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_id_uses_varint_not_bitpacking() {
        let ids = vec![42u32];
        let (chunks, count) = pack_postings(&ids);
        assert_eq!(count, 1);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0], PostingChunk::VarInt { .. }));
        assert!(posting_chunk_byte_len(&chunks[0]) < 20);
    }

    #[test]
    fn tail_uses_varint_full_blocks_bitpacking() {
        let ids: Vec<u32> = (0..130).map(|i| i * 2).collect();
        let (chunks, _) = pack_postings(&ids);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(chunks[0], PostingChunk::BitPacked { .. }));
        assert!(matches!(chunks[1], PostingChunk::VarInt { count: 2, .. }));

        let unpacked = unpack_posting_chunks(&chunks, ids.len() as u32).unwrap();
        assert_eq!(unpacked, ids);
    }

    #[test]
    fn roundtrip_large_list() {
        let ids: Vec<u32> = (0..500).map(|i| i * 3 + 1).collect();
        let (chunks, _) = pack_postings(&ids);
        let unpacked = unpack_posting_chunks(&chunks, ids.len() as u32).unwrap();
        assert_eq!(unpacked, ids);
    }
}
