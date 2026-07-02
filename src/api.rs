use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::engine::{flush_block_async, SharedEngine};

#[derive(Deserialize)]
pub struct SearchQuery {
    pub query: String,
}

#[derive(Serialize)]
pub struct IngestResponse {
    pub ingested: usize,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub count: usize,
    pub results: Vec<String>,
}

pub fn router(engine: SharedEngine) -> Router {
    Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/search", get(search_handler))
        .with_state(engine)
}

async fn ingest_handler(
    State(engine): State<SharedEngine>,
    Json(lines): Json<Vec<String>>,
) -> Result<Json<IngestResponse>, StatusCode> {
    let (ingested, flushed_blocks) = {
        let mut guard = engine.write().await;
        let result = guard.ingest(lines);
        (result.ingested, result.flushed_blocks)
    };

    for block in flushed_blocks {
        flush_block_async(engine.clone(), block)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(Json(IngestResponse { ingested }))
}

async fn search_handler(
    State(engine): State<SharedEngine>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let results = {
        let guard = engine.read().await;
        guard
            .search(&params.query)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };

    Ok(Json(SearchResponse {
        query: params.query,
        count: results.len(),
        results,
    }))
}
