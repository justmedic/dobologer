use std::collections::HashMap;
use std::io::{Read, Write};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::IDX_ALIGN;

pub const IDX_FOOTER_SIZE: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockMeta {
    pub block_id: u64,
    pub num_lines: u32,
    pub rows_per_pack: u32,
    pub data_size: u64,
    pub idx_size: u64,
}

#[derive(Debug, Clone)]
pub struct DictEntry {
    pub term: String,
    pub posting_offset: u64,
    pub num_ids: u32,
    pub num_chunks: u32,
}

pub fn align_offset(offset: u64) -> u64 {
    ((offset + IDX_ALIGN as u64 - 1) / IDX_ALIGN as u64) * IDX_ALIGN as u64
}

pub fn write_u32<W: Write>(w: &mut W, value: u32) -> Result<()> {
    w.write_all(&value.to_le_bytes())?;
    Ok(())
}

pub fn write_u64<W: Write>(w: &mut W, value: u64) -> Result<()> {
    w.write_all(&value.to_le_bytes())?;
    Ok(())
}

pub fn read_u32<R: Read>(r: &mut R) -> Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

pub fn read_u64<R: Read>(r: &mut R) -> Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

pub fn write_dict_entry<W: Write>(w: &mut W, entry: &DictEntry) -> Result<()> {
    let term_bytes = entry.term.as_bytes();
    write_u32(w, term_bytes.len() as u32)?;
    w.write_all(term_bytes)?;
    write_u64(w, entry.posting_offset)?;
    write_u32(w, entry.num_ids)?;
    write_u32(w, entry.num_chunks)?;
    Ok(())
}

pub fn read_dict_entry<R: Read>(r: &mut R) -> Result<DictEntry> {
    let term_len = read_u32(r)? as usize;
    let mut term_bytes = vec![0u8; term_len];
    r.read_exact(&mut term_bytes)?;
    let term = String::from_utf8(term_bytes).context("invalid utf8 in dictionary term")?;
    let posting_offset = read_u64(r)?;
    let num_ids = read_u32(r)?;
    let num_chunks = read_u32(r)?;
    Ok(DictEntry {
        term,
        posting_offset,
        num_ids,
        num_chunks,
    })
}

pub fn write_idx_footer<W: Write>(w: &mut W, dict_offset: u64, num_terms: u32) -> Result<()> {
    write_u64(w, dict_offset)?;
    write_u32(w, num_terms)?;
    Ok(())
}

pub fn parse_idx_footer(data: &[u8]) -> Result<(u64, u32)> {
    if data.len() < IDX_FOOTER_SIZE {
        bail!("idx file too small for footer");
    }
    let footer_start = data.len() - IDX_FOOTER_SIZE;
    let dict_offset = u64::from_le_bytes(data[footer_start..footer_start + 8].try_into()?);
    let num_terms = u32::from_le_bytes(data[footer_start + 8..footer_start + 12].try_into()?);
    Ok((dict_offset, num_terms))
}

pub fn load_dictionary(data: &[u8], dict_offset: u64, num_terms: u32) -> Result<HashMap<String, DictEntry>> {
    let mut cursor = &data[dict_offset as usize..];
    let mut dict = HashMap::with_capacity(num_terms as usize);
    for _ in 0..num_terms {
        let entry = read_dict_entry(&mut cursor)?;
        dict.insert(entry.term.clone(), entry);
    }
    Ok(dict)
}

pub struct DataHeader {
    pub num_lines: u32,
    pub rows_per_pack: u32,
    pub num_packs: u32,
    pub pack_offsets: Vec<u64>,
    pub line_offsets: Vec<u32>,
    pub zstd_section_offset: u64,
}

pub fn parse_data_header(data: &[u8]) -> Result<DataHeader> {
    if data.len() < 12 {
        bail!("data file too small");
    }

    let num_lines = u32::from_le_bytes(data[0..4].try_into()?);
    let rows_per_pack = u32::from_le_bytes(data[4..8].try_into()?);
    let num_packs = u32::from_le_bytes(data[8..12].try_into()?);

    let pack_offsets_start = 12;
    let pack_offsets_end = pack_offsets_start + ((num_packs as usize) + 1) * 8;
    if data.len() < pack_offsets_end {
        bail!("data file missing pack offsets");
    }

    let mut pack_offsets = Vec::with_capacity(num_packs as usize + 1);
    for i in 0..=num_packs as usize {
        let start = pack_offsets_start + i * 8;
        pack_offsets.push(u64::from_le_bytes(data[start..start + 8].try_into()?));
    }

    let line_offsets_start = pack_offsets_end;
    let line_offsets_end = line_offsets_start + num_lines as usize * 4;
    if data.len() < line_offsets_end {
        bail!("data file missing line offsets");
    }

    let mut line_offsets = Vec::with_capacity(num_lines as usize);
    for i in 0..num_lines as usize {
        let start = line_offsets_start + i * 4;
        line_offsets.push(u32::from_le_bytes(data[start..start + 4].try_into()?));
    }

    Ok(DataHeader {
        num_lines,
        rows_per_pack,
        num_packs,
        pack_offsets,
        line_offsets,
        zstd_section_offset: line_offsets_end as u64,
    })
}

pub fn block_paths(data_dir: &std::path::Path, block_id: u64) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let meta = data_dir.join(format!("{block_id}.meta"));
    let data = data_dir.join(format!("{block_id}.data"));
    let idx = data_dir.join(format!("{block_id}.idx"));
    (meta, data, idx)
}
