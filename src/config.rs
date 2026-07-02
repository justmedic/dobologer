pub const BLOCK_ROWS: usize = 65_536;
pub const PACK_ROWS: usize = 1_024;
pub const PACK_CHUNK: usize = 128;
pub const IDX_ALIGN: usize = 16;
pub const ZSTD_LEVEL: i32 = 3;
pub const DEFAULT_DATA_DIR: &str = "./data";
pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

pub fn block_rows() -> usize {
    std::env::var("DOBOLOGER_BLOCK_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(BLOCK_ROWS)
}

pub fn pack_rows() -> usize {
    std::env::var("DOBOLOGER_PACK_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(PACK_ROWS)
}

pub fn data_dir() -> String {
    std::env::var("DOBOLOGER_DATA_DIR").unwrap_or_else(|_| DEFAULT_DATA_DIR.to_string())
}
