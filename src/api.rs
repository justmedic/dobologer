use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::engine::{search_async, spawn_detached_flushes, FlushCoordinator, SharedEngine};

pub struct AppState {
    pub engine: SharedEngine,
    pub flushes: Arc<FlushCoordinator>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct IngestResponse {
    pub ingested: usize,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub count: usize,
    pub total: usize,
    pub results: Vec<String>,
}

pub fn router(engine: SharedEngine, flushes: Arc<FlushCoordinator>) -> Router {
    Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/search", get(search_handler))
        .with_state(Arc::new(AppState { engine, flushes }))
}

async fn ingest_handler(
    State(state): State<Arc<AppState>>,
    Json(lines): Json<Vec<String>>,
) -> Result<Json<IngestResponse>, StatusCode> {
    let (ingested, flushed_blocks) = {
        let mut guard = state.engine.write().await;
        let result = guard.ingest(lines);
        (result.ingested, result.flushed_blocks)
    };

    spawn_detached_flushes(&state.flushes, state.engine.clone(), flushed_blocks);

    Ok(Json(IngestResponse { ingested }))
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let search = search_async(&state.engine, &params.query, params.limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SearchResponse {
        query: params.query,
        count: search.results.len(),
        total: search.total,
        results: search.results,
    }))
}
