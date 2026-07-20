use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    catalog::Catalog,
    domain::{AnalyzeRequest, PlanRequest, PlannerError, analyze_contract, plan_reverse},
    market::{MarketProvider, MarketStatus},
    regression::{ActualContractSubmission, RegressionStore},
    sync::SyncService,
};

#[derive(Clone)]
pub struct AppState {
    pub catalog: Arc<RwLock<Catalog>>,
    pub sync: Arc<SyncService>,
    pub regressions: Arc<RegressionStore>,
    pub market: Arc<MarketProvider>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    math: &'static str,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub fn app(
    catalog: Arc<RwLock<Catalog>>,
    sync: Arc<SyncService>,
    regressions: Arc<RegressionStore>,
    market: Arc<MarketProvider>,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:5173".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    Router::new()
        .route("/api/health", get(health))
        .route("/api/catalog", get(catalog_handler))
        .route("/api/sync-status", get(sync_status))
        .route("/api/sync-now", post(sync_now))
        .route("/api/market-status", get(market_status))
        .route("/api/regressions/status", get(regression_status))
        .route("/api/regressions/contracts", post(record_contract))
        .route("/api/analyze", post(analyze))
        .route("/api/plan", post(plan))
        .with_state(AppState {
            catalog,
            sync,
            regressions,
            market,
        })
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "cs-float-planner-api",
        math: "sequential IEEE-754 float32",
    })
}

async fn catalog_handler(State(state): State<AppState>) -> Json<crate::catalog::CatalogResponse> {
    Json(state.catalog.read().await.public_response())
}

async fn sync_status(State(state): State<AppState>) -> Json<crate::sync::SyncStatus> {
    Json(state.sync.status().await)
}

async fn sync_now(State(state): State<AppState>) -> Json<crate::sync::SyncStatus> {
    state.sync.sync_now().await;
    Json(state.sync.status().await)
}

async fn market_status(State(state): State<AppState>) -> Json<MarketStatus> {
    Json(state.market.status().await)
}

async fn regression_status(
    State(state): State<AppState>,
) -> Json<crate::regression::RegressionStatus> {
    Json(state.regressions.status().await)
}

async fn record_contract(
    State(state): State<AppState>,
    Json(submission): Json<ActualContractSubmission>,
) -> Result<Json<crate::regression::RegressionRecord>, ApiError> {
    let catalog = state.catalog.read().await;
    state
        .regressions
        .submit(&catalog, submission)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn analyze(
    State(state): State<AppState>,
    Json(request): Json<AnalyzeRequest>,
) -> Result<Json<crate::domain::AnalyzeResponse>, ApiError> {
    let catalog = state.catalog.read().await;
    analyze_contract(&catalog, request)
        .map(Json)
        .map_err(ApiError::from)
}

async fn plan(
    State(state): State<AppState>,
    Json(request): Json<PlanRequest>,
) -> Result<Json<crate::domain::PlanResponse>, ApiError> {
    let catalog = state.catalog.read().await.clone();
    let market_listings = state
        .market
        .listings_for_plan(&catalog, &request)
        .await
        .map_err(ApiError::from)?;
    let planning_catalog = market_listings
        .map(|listings| catalog.with_listings(listings))
        .unwrap_or_else(|| catalog.clone());
    plan_reverse(&planning_catalog, request)
        .map(Json)
        .map_err(ApiError::from)
}

struct ApiError(PlannerError);

impl From<PlannerError> for ApiError {
    fn from(value: PlannerError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = match self.0 {
            PlannerError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            PlannerError::Catalog(ref message) if message.starts_with("Steam Market listing") => {
                StatusCode::BAD_GATEWAY
            }
            PlannerError::Catalog(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(ErrorResponse {
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}
