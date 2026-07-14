use std::net::SocketAddr;

use std::sync::Arc;

use cs_float_planner::{api, catalog::Catalog, regression::RegressionStore, sync::SyncService};

#[tokio::main]
async fn main() {
    let catalog = Catalog::load_embedded().expect("embedded catalog must be valid JSON");
    let sync = Arc::new(SyncService::from_environment().expect("sync service must be configured"));
    sync.clone().start();
    let regressions = Arc::new(RegressionStore::from_environment());
    let app = api::app(catalog, sync, regressions);
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
