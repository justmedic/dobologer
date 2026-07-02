pub const BLOCK_ROWS: usize = 65_536;
pub const PACK_ROWS: usize = 1_024;
pub const PACK_CHUNK: usize = 128;
pub const IDX_ALIGN: usize = 16;
pub const ZSTD_LEVEL: i32 = 3;
pub const DEFAULT_DATA_DIR: &str = "./data";
pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";
pub const DEFAULT_TCP_ADDR: &str = "0.0.0.0:8081";
pub const DEFAULT_UDP_ADDR: &str = "0.0.0.0:8082";
pub const TCP_BATCH_MAX_LINES: usize = 1_000;
pub const TCP_BATCH_FLUSH_MS: u64 = 300;

pub const DEFAULT_SEARCH_LIMIT: usize = 1_000;

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

pub fn tcp_addr() -> String {
    std::env::var("DOBOLOGER_TCP_ADDR").unwrap_or_else(|_| DEFAULT_TCP_ADDR.to_string())
}

pub fn udp_addr() -> String {
    std::env::var("DOBOLOGER_UDP_ADDR").unwrap_or_else(|_| DEFAULT_UDP_ADDR.to_string())
}

pub fn tcp_batch_lines() -> usize {
    std::env::var("DOBOLOGER_TCP_BATCH_LINES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(TCP_BATCH_MAX_LINES)
}

pub fn tcp_flush_ms() -> u64 {
    std::env::var("DOBOLOGER_TCP_FLUSH_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(TCP_BATCH_FLUSH_MS)
}

/// Resolve search limit: omitted -> default, 0 -> unlimited.
pub fn resolve_search_limit(limit: Option<usize>) -> usize {
    match limit {
        Some(0) => usize::MAX,
        Some(n) => n,
        None => std::env::var("DOBOLOGER_SEARCH_LIMIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SEARCH_LIMIT),
    }
}
