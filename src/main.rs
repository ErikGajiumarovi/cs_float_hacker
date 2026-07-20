use std::net::SocketAddr;

use std::sync::Arc;

use cs_float_planner::{
    api, catalog::Catalog, market::MarketProvider, regression::RegressionStore, sync::SyncService,
};
use tokio::sync::RwLock;

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("SYNC_DATA_DIR").unwrap_or_else(|_| "runtime".to_owned());
    let catalog = Arc::new(RwLock::new(Catalog::load_runtime_or_embedded(
        data_dir.as_ref(),
    )));
    let sync = Arc::new(
        SyncService::from_environment(catalog.clone()).expect("sync service must be configured"),
    );
    sync.clone().start();
    let regressions = Arc::new(RegressionStore::from_environment());
    let market =
        Arc::new(MarketProvider::from_environment().expect("market provider must be configured"));
    let app = api::app(catalog, sync, regressions, market);
    let address: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()
        .expect("BIND_ADDR must be a valid socket address");

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind API listener");
    println!("CS Float Planner API listening on http://{address}");
    axum::serve(listener, app)
        .await
        .expect("API server terminated unexpectedly");
}
