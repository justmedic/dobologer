use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};

use crate::block::active::ActiveBlock;
use crate::block::format::{
    align_offset, block_paths, write_dict_entry, write_idx_footer, write_u32, write_u64, BlockMeta,
    DictEntry,
};
use crate::config::{pack_rows, ZSTD_LEVEL};
use crate::index::pack_postings;

pub fn flush_block(data_dir: &Path, block: &ActiveBlock) -> Result<()> {
    fs::create_dir_all(data_dir).context("create data directory")?;

    let (meta_path, data_path, idx_path) = block_paths(data_dir, block.id);
    let rows_per_pack = pack_rows() as u32;

    let data_size = write_data_file(&data_path, block, rows_per_pack)?;
    let idx_size = write_idx_file(&idx_path, &block.inverted)?;
    write_meta_file(
        &meta_path,
        BlockMeta {
            block_id: block.id,
            num_lines: block.lines.len() as u32,
            rows_per_pack,
            data_size,
            idx_size,
        },
    )?;

    Ok(())
}

fn write_meta_file(path: &Path, meta: BlockMeta) -> Result<()> {
    let json = serde_json::to_vec_pretty(&meta)?;
    fs::write(path, json).context("write meta file")?;
    Ok(())
}

fn write_data_file(path: &Path, block: &ActiveBlock, rows_per_pack: u32) -> Result<u64> {
    let num_lines = block.lines.len() as u32;
    let num_packs = num_lines.div_ceil(rows_per_pack);

    let mut pack_plaintexts: Vec<Vec<u8>> = Vec::with_capacity(num_packs as usize);
    let mut line_offsets = vec![0u32; num_lines as usize];

    for pack_idx in 0..num_packs as usize {
        let start_line = pack_idx * rows_per_pack as usize;
        let end_line = ((pack_idx + 1) * rows_per_pack as usize).min(block.lines.len());
        let mut pack_bytes = Vec::new();

        for line_id in start_line..end_line {
            line_offsets[line_id] = pack_bytes.len() as u32;
            pack_bytes.extend_from_slice(block.lines[line_id].as_bytes());
        }

        pack_plaintexts.push(pack_bytes);
    }

    let mut zstd_frames = Vec::with_capacity(num_packs as usize);
    for plaintext in &pack_plaintexts {
        let compressed = zstd::encode_all(plaintext.as_slice(), ZSTD_LEVEL)
            .context("zstd compress data pack")?;
        zstd_frames.push(compressed);
    }

    let header_size = 12usize + (num_packs as usize + 1) * 8 + num_lines as usize * 4;
    let mut pack_offsets = Vec::with_capacity(num_packs as usize + 1);
    let mut current_offset = header_size as u64;
    for frame in &zstd_frames {
        pack_offsets.push(current_offset);
        current_offset += frame.len() as u64;
    }
    pack_offsets.push(current_offset);

    let file = File::create(path).context("create data file")?;
    let mut writer = BufWriter::new(file);

    write_u32(&mut writer, num_lines)?;
    write_u32(&mut writer, rows_per_pack)?;
    write_u32(&mut writer, num_packs)?;
    for offset in &pack_offsets {
        write_u64(&mut writer, *offset)?;
    }
    for offset in &line_offsets {
        write_u32(&mut writer, *offset)?;
    }
    for frame in &zstd_frames {
        writer.write_all(frame)?;
    }
    writer.flush()?;

    Ok(fs::metadata(path)?.len())
}

fn write_idx_file(path: &Path, inverted: &std::collections::HashMap<String, Vec<u32>>) -> Result<u64> {
    let file = File::create(path).context("create idx file")?;
    let mut writer = BufWriter::new(file);
    let mut file_offset: u64 = 0;

    let mut dict_entries: Vec<DictEntry> = Vec::new();
    let mut sorted_terms: Vec<_> = inverted.iter().collect();
    sorted_terms.sort_by_key(|(term, _)| term.as_str());

    for (term, ids) in sorted_terms {
        let aligned_offset = align_offset(file_offset);
        let padding = aligned_offset - file_offset;
        if padding > 0 {
            writer.write_all(&vec![0u8; padding as usize])?;
            file_offset += padding;
        }

        let posting_offset = file_offset;
        let (chunks, num_ids) = pack_postings(ids);
        let num_chunks = chunks.len() as u32;

        for chunk in &chunks {
            writer.write_all(&[chunk.num_bits])?;
            file_offset += 1;
            writer.write_all(&chunk.data)?;
            file_offset += chunk.data.len() as u64;
        }

        dict_entries.push(DictEntry {
            term: term.clone(),
            posting_offset,
            num_ids,
            num_chunks,
        });
    }

    let dict_offset = file_offset;
    for entry in &dict_entries {
        write_dict_entry(&mut writer, entry)?;
    }
    write_idx_footer(&mut writer, dict_offset, dict_entries.len() as u32)?;
    writer.flush()?;

    Ok(fs::metadata(path)?.len())
}
