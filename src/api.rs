use std::sync::Arc;

use axum::{
    extract::{Query as AxumQuery, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::config::resolve_search_limit;
use crate::engine::{search_async, FlushCoordinator, SharedEngine};
use crate::ingest::ingest_batch;
use crate::query::{parse_query, Query, QueryJson};

pub struct AppState {
    pub engine: SharedEngine,
    pub flushes: Arc<FlushCoordinator>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct SearchPostBody {
    pub query: QueryJson,
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
        .route("/", get(root_handler))
        .route("/ingest", post(ingest_handler))
        .route("/search", get(search_handler).post(search_post_handler))
        .with_state(Arc::new(AppState { engine, flushes }))
}

async fn root_handler() -> Html<&'static str> {
    Html(include_str!("../ui/index.html"))
}

async fn ingest_handler(
    State(state): State<Arc<AppState>>,
    Json(lines): Json<Vec<String>>,
) -> Result<Json<IngestResponse>, StatusCode> {
    let ingested = ingest_batch(state.engine.clone(), state.flushes.clone(), lines).await;
    Ok(Json(IngestResponse { ingested }))
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    AxumQuery(params): AxumQuery<SearchQuery>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let parsed = parse_query(&params.query).map_err(|_| StatusCode::BAD_REQUEST)?;
    let limit = resolve_search_limit(params.limit);
    let search = search_async(&state.engine, parsed, Some(limit))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SearchResponse {
        query: params.query,
        count: search.results.len(),
        total: search.total,
        results: search.results,
    }))
}

async fn search_post_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SearchPostBody>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let parsed: Query = body.query.into();
    let query_display = parsed.to_string();
    let limit = resolve_search_limit(body.limit);
    let search = search_async(&state.engine, parsed, Some(limit))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SearchResponse {
        query: query_display,
        count: search.results.len(),
        total: search.total,
        results: search.results,
    }))
}
