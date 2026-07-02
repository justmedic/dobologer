use std::sync::Arc;

use rayon::prelude::*;

use crate::engine::{spawn_detached_flushes, FlushCoordinator, SharedEngine};

/// Normalize a raw ingest line: extract text from NDJSON or pass through plain text.
pub fn normalize_ingest_line(raw: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_string();
    };

    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) => {
            for key in ["message", "@message", "msg", "log", "text"] {
                if let Some(serde_json::Value::String(s)) = map.get(key) {
                    return s.clone();
                }
            }
            raw.to_string()
        }
        _ => raw.to_string(),
    }
}

/// Shared ingest entry point for HTTP, TCP, and UDP.
pub async fn ingest_batch(
    engine: SharedEngine,
    flushes: Arc<FlushCoordinator>,
    raw_lines: Vec<String>,
) -> usize {
    if raw_lines.is_empty() {
        return 0;
    }

    let normalized = tokio::task::spawn_blocking(move || {
        raw_lines
            .par_iter()
            .map(|line| normalize_ingest_line(line))
            .collect::<Vec<String>>()
    })
    .await
    .unwrap_or_default();

    let (ingested, flushed_blocks) = {
        let mut guard = engine.write().await;
        let result = guard.ingest(normalized);
        (result.ingested, result.flushed_blocks)
    };

    spawn_detached_flushes(&flushes, engine, flushed_blocks);
    ingested
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passthrough() {
        assert_eq!(normalize_ingest_line("hello world"), "hello world");
    }

    #[test]
    fn json_string_extracts_content() {
        assert_eq!(normalize_ingest_line(r#""log line content""#), "log line content");
    }

    #[test]
    fn json_object_extracts_message() {
        let raw = r#"{"level":"info","message":"user logged in"}"#;
        assert_eq!(normalize_ingest_line(raw), "user logged in");
    }

    #[test]
    fn json_object_extracts_at_message() {
        let raw = r#"{"@message":"cloudwatch line"}"#;
        assert_eq!(normalize_ingest_line(raw), "cloudwatch line");
    }

    #[test]
    fn json_object_no_known_field_returns_raw() {
        let raw = r#"{"foo":"bar"}"#;
        assert_eq!(normalize_ingest_line(raw), raw);
    }

    #[test]
    fn invalid_json_passthrough() {
        assert_eq!(normalize_ingest_line("not { json"), "not { json");
    }

    #[test]
    fn json_array_passthrough() {
        let raw = r#"["a","b"]"#;
        assert_eq!(normalize_ingest_line(raw), raw);
    }
}
