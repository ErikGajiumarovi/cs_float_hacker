use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cs_float_planner::{
    catalog::{Catalog, CatalogResponse},
    domain::{AnalyzeRequest, AnalyzeResponse, PlanRequest, PlanResponse, plan_reverse},
    market::{MarketProvider, MarketStatus},
    regression::{
        ActualContractSubmission, RegressionRecord, RegressionStatus, prepare_regression_record,
        regression_status,
    },
    sync::{SyncPersistence, SyncService, SyncStatus},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use tauri::{Manager, State};
use tokio::sync::RwLock;

const DATABASE_FILE: &str = "floatcraft.sqlite3";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopSettings {
    market_enabled: bool,
    live_market_notice_acknowledged: bool,
}

struct LocalStore {
    connection: Mutex<Connection>,
}

impl LocalStore {
    fn open(data_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
        let database_path = data_dir.join(DATABASE_FILE);
        let backup_path = data_dir.join("floatcraft.sqlite3.backup");
        let mut connection = open_connection(&database_path)?;
        let integrity = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap_or_else(|_| "corrupt".to_owned());
        if integrity != "ok" {
            drop(connection);
            let corrupt_path = data_dir.join(format!("floatcraft.sqlite3.corrupt-{}", now_epoch()));
            fs::rename(&database_path, corrupt_path).map_err(|error| error.to_string())?;
            if backup_path.exists() {
                fs::copy(&backup_path, &database_path).map_err(|error| error.to_string())?;
            }
            connection = open_connection(&database_path)?;
        }
        configure_connection(&connection)?;
        migrate(&connection, &database_path)?;
        import_legacy_json(&connection, data_dir)?;
        connection
            .backup(rusqlite::MAIN_DB, &backup_path, None)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

fn open_connection(database_path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(database_path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
        .map_err(|error| error.to_string())
}

impl LocalStore {
    fn settings(&self) -> Result<DesktopSettings, String> {
        Ok(DesktopSettings {
            market_enabled: self.get_bool("market_enabled")?,
            live_market_notice_acknowledged: self.get_bool("live_market_notice_acknowledged")?,
        })
    }

    fn set_market_enabled(&self, enabled: bool) -> Result<DesktopSettings, String> {
        self.set_bool("market_enabled", enabled)?;
        self.settings()
    }

    fn acknowledge_live_market_notice(&self) -> Result<DesktopSettings, String> {
        self.set_bool("live_market_notice_acknowledged", true)?;
        self.settings()
    }

    fn regression_status(&self) -> Result<RegressionStatus, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "database lock poisoned")?;
        let mut statement = connection
            .prepare("SELECT record_json FROM regression_contracts ORDER BY submitted_at")
            .map_err(|error| error.to_string())?;
        let records = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .map(|result| {
                result
                    .map_err(|error| error.to_string())
                    .and_then(|json| serde_json::from_str(&json).map_err(|error| error.to_string()))
            })
            .collect::<Result<Vec<RegressionRecord>, _>>()?;
        Ok(regression_status(&records))
    }

    fn save_regression(&self, record: &RegressionRecord) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "database lock poisoned")?;
        let json = serde_json::to_string(record).map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO regression_contracts (id, submitted_at, record_json) VALUES (?1, ?2, ?3)",
                params![record.id, record.submitted_at, json],
            )
            .map_err(|error| match error.sqlite_error_code() {
                Some(rusqlite::ErrorCode::ConstraintViolation) => {
                    "this exact evidence-backed contract has already been recorded".to_owned()
                }
                _ => error.to_string(),
            })?;
        Ok(())
    }

    fn get_bool(&self, key: &str) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "database lock poisoned")?;
        let value: String = connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("missing local setting: {key}"))?;
        Ok(value == "true")
    }

    fn set_bool(&self, key: &str, value: bool) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "database lock poisoned")?;
        connection
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value.to_string()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn migrate(connection: &Connection, database_path: &Path) -> Result<(), String> {
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < 2 && database_path.exists() {
        let backup = database_path.with_extension("sqlite3.pre-migration.bak");
        fs::copy(database_path, backup).map_err(|error| error.to_string())?;
    }
    if version < 1 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS regression_contracts (
                id TEXT PRIMARY KEY NOT NULL,
                submitted_at INTEGER NOT NULL,
                record_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_events (
                started_at INTEGER NOT NULL,
                completed_at INTEGER,
                result TEXT NOT NULL,
                detail TEXT
             );
             INSERT OR IGNORE INTO settings (key, value) VALUES ('market_enabled', 'true');
             INSERT OR IGNORE INTO settings (key, value) VALUES ('live_market_notice_acknowledged', 'false');
             PRAGMA user_version = 1;
             COMMIT;",
            )
            .map_err(|error| error.to_string())?;
    }
    if version < 2 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS catalog_versions (
                    schema_version TEXT PRIMARY KEY NOT NULL,
                    retrieved_at INTEGER NOT NULL,
                    catalog_json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS active_catalog (
                    slot INTEGER PRIMARY KEY NOT NULL CHECK (slot = 1),
                    schema_version TEXT NOT NULL REFERENCES catalog_versions(schema_version)
                 );
                 CREATE TABLE IF NOT EXISTS sync_state (
                    slot INTEGER PRIMARY KEY NOT NULL CHECK (slot = 1),
                    status_json TEXT NOT NULL
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Imports data written by the pre-SQLite desktop/runtime builds exactly once.
/// The source files are deliberately left intact so a failed import is
/// recoverable by the user.
fn import_legacy_json(connection: &Connection, data_dir: &Path) -> Result<(), String> {
    let active_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM active_catalog WHERE slot = 1)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !active_exists {
        if let Ok(json) = fs::read_to_string(data_dir.join("planner-catalog.json")) {
            if let Ok(catalog) = Catalog::from_persisted_json(&json) {
                connection
                    .execute(
                        "INSERT OR IGNORE INTO catalog_versions (schema_version, retrieved_at, catalog_json) VALUES (?1, ?2, ?3)",
                        params![catalog.schema_version, now_epoch(), json],
                    )
                    .map_err(|error| error.to_string())?;
                connection
                    .execute(
                        "INSERT OR REPLACE INTO active_catalog (slot, schema_version) VALUES (1, ?1)",
                        [catalog.schema_version],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    let state_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_state WHERE slot = 1)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !state_exists {
        if let Ok(json) = fs::read_to_string(data_dir.join("sync-status.json")) {
            if serde_json::from_str::<SyncStatus>(&json).is_ok() {
                connection
                    .execute(
                        "INSERT INTO sync_state (slot, status_json) VALUES (1, ?1)",
                        [json],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    if let Ok(json) = fs::read_to_string(data_dir.join("actual-contracts.json")) {
        if let Ok(records) = serde_json::from_str::<Vec<RegressionRecord>>(&json) {
            for record in records {
                connection
                    .execute(
                        "INSERT OR IGNORE INTO regression_contracts (id, submitted_at, record_json) VALUES (?1, ?2, ?3)",
                        params![record.id, record.submitted_at, serde_json::to_string(&record).map_err(|error| error.to_string())?],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

struct DesktopRuntime {
    catalog: Arc<RwLock<Catalog>>,
    sync: Arc<SyncService>,
    market: Arc<MarketProvider>,
    store: Arc<LocalStore>,
}

impl DesktopRuntime {
    fn open(data_dir: PathBuf) -> Result<Self, String> {
        let store = Arc::new(LocalStore::open(&data_dir)?);
        let catalog = Arc::new(RwLock::new(store.load_catalog().unwrap_or_else(|| {
            Catalog::load_embedded().expect("embedded catalog must be valid JSON")
        })));
        let sync = Arc::new(SyncService::from_persistence(
            catalog.clone(),
            data_dir.display().to_string(),
            store.clone(),
        )?);
        let market = Arc::new(MarketProvider::from_environment()?);
        tauri::async_runtime::spawn(sync.clone().run());
        Ok(Self {
            catalog,
            sync,
            market,
            store,
        })
    }
}

impl SyncPersistence for LocalStore {
    fn load_catalog(&self) -> Option<Catalog> {
        let connection = self.connection.lock().ok()?;
        let json: String = connection
            .query_row(
                "SELECT catalog_json FROM catalog_versions JOIN active_catalog USING (schema_version) WHERE active_catalog.slot = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()??;
        Catalog::from_persisted_json(&json).ok()
    }

    fn load_status(&self) -> Option<SyncStatus> {
        let connection = self.connection.lock().ok()?;
        let json: String = connection
            .query_row(
                "SELECT status_json FROM sync_state WHERE slot = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()??;
        serde_json::from_str(&json).ok()
    }

    fn record_success(&self, catalog: &Catalog, status: &SyncStatus) -> Result<(), String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "database lock poisoned")?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let catalog_json = catalog.to_persisted_json()?;
        let status_json = serde_json::to_string(status).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO catalog_versions (schema_version, retrieved_at, catalog_json) VALUES (?1, ?2, ?3)",
                params![catalog.schema_version, status.last_success_at.unwrap_or_else(now_epoch), catalog_json],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO active_catalog (slot, schema_version) VALUES (1, ?1)",
                [&catalog.schema_version],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO sync_state (slot, status_json) VALUES (1, ?1)",
                [&status_json],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO sync_events (started_at, completed_at, result, detail) VALUES (?1, ?2, 'success', NULL)",
                params![status.last_started_at.unwrap_or_else(now_epoch), status.last_success_at.unwrap_or_else(now_epoch)],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    fn record_status(&self, status: &SyncStatus) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "database lock poisoned")?;
        let json = serde_json::to_string(status).map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT OR REPLACE INTO sync_state (slot, status_json) VALUES (1, ?1)",
                [&json],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO sync_events (started_at, completed_at, result, detail) VALUES (?1, ?2, 'error', ?3)",
                params![status.last_started_at.unwrap_or_else(now_epoch), now_epoch(), status.last_error],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn regression_count(&self) -> usize {
        self.connection
            .lock()
            .ok()
            .and_then(|connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM regression_contracts", [], |row| {
                        row.get(0)
                    })
                    .ok()
            })
            .unwrap_or(0)
    }
}

#[tauri::command]
async fn get_catalog(state: State<'_, DesktopRuntime>) -> Result<CatalogResponse, String> {
    Ok(state.catalog.read().await.public_response())
}

#[tauri::command]
async fn get_sync_status(state: State<'_, DesktopRuntime>) -> Result<SyncStatus, String> {
    Ok(state.sync.status().await)
}

#[tauri::command]
async fn sync_catalog_now(state: State<'_, DesktopRuntime>) -> Result<SyncStatus, String> {
    state.sync.sync_now().await;
    Ok(state.sync.status().await)
}

#[tauri::command]
async fn get_market_status(state: State<'_, DesktopRuntime>) -> Result<MarketStatus, String> {
    let settings = state.store.settings()?;
    let mut status = state.market.status().await;
    status.enabled = settings.market_enabled && settings.live_market_notice_acknowledged;
    if !status.enabled {
        status.provider = if settings.market_enabled {
            "waiting_for_notice_acknowledgement".to_owned()
        } else {
            "disabled_by_user".to_owned()
        };
    }
    Ok(status)
}

#[tauri::command]
fn get_settings(state: State<'_, DesktopRuntime>) -> Result<DesktopSettings, String> {
    state.store.settings()
}

#[tauri::command]
fn set_market_enabled(
    state: State<'_, DesktopRuntime>,
    enabled: bool,
) -> Result<DesktopSettings, String> {
    state.store.set_market_enabled(enabled)
}

#[tauri::command]
fn acknowledge_live_market_notice(
    state: State<'_, DesktopRuntime>,
) -> Result<DesktopSettings, String> {
    state.store.acknowledge_live_market_notice()
}

#[tauri::command]
fn get_regression_status(state: State<'_, DesktopRuntime>) -> Result<RegressionStatus, String> {
    state.store.regression_status()
}

#[tauri::command]
async fn analyze_contract(
    state: State<'_, DesktopRuntime>,
    request: AnalyzeRequest,
) -> Result<AnalyzeResponse, String> {
    let catalog = state.catalog.read().await;
    cs_float_planner::domain::analyze_contract(&catalog, request).map_err(|error| error.to_string())
}

#[tauri::command]
async fn preview_partial_contract(
    state: State<'_, DesktopRuntime>,
    request: cs_float_planner::domain::PartialAnalyzeRequest,
) -> Result<cs_float_planner::domain::PartialAnalyzeResponse, String> {
    let catalog = state.catalog.read().await;
    cs_float_planner::domain::preview_partial_contract(&catalog, request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn plan_contract(
    state: State<'_, DesktopRuntime>,
    request: PlanRequest,
) -> Result<PlanResponse, String> {
    let catalog = state.catalog.read().await.clone();
    let settings = state.store.settings()?;
    let market_warning = if settings.market_enabled && settings.live_market_notice_acknowledged {
        match tokio::time::timeout(
            Duration::from_secs(30),
            state.market.listings_for_plan(&catalog, &request),
        )
        .await
        {
            Ok(Ok(listings)) => (listings, None),
            Ok(Err(error)) => (
                None,
                Some(format!(
                    "Steam Market temporarily unavailable ({error}). The plan below is ideal_math and contains no price or listing claim."
                )),
            ),
            Err(_) => (
                None,
                Some(
                    "Steam Market did not answer within the 30-second safety limit. The plan below is ideal_math and contains no price or listing claim."
                        .to_owned(),
                ),
            ),
        }
    } else {
        (None, None)
    };
    let planning_catalog = market_warning
        .0
        .map(|listings| catalog.with_listings(listings))
        .unwrap_or(catalog);
    let mut plan = plan_reverse(&planning_catalog, request).map_err(|error| error.to_string())?;
    if let Some(warning) = market_warning.1 {
        plan.message =
            "Live Steam data is unavailable; a local ideal_math plan is shown.".to_owned();
        plan.warnings.insert(0, warning);
    }
    Ok(plan)
}

#[tauri::command]
async fn record_contract(
    state: State<'_, DesktopRuntime>,
    submission: ActualContractSubmission,
) -> Result<RegressionRecord, String> {
    let catalog = state.catalog.read().await;
    let record = prepare_regression_record(&catalog, submission, now_epoch())
        .map_err(|error| error.to_string())?;
    state.store.save_regression(&record)?;
    Ok(record)
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app.path().app_data_dir().map_err(|error| {
                eprintln!("Floatcraft could not resolve its app-data directory: {error}");
                error
            })?;
            let runtime = DesktopRuntime::open(data_dir).map_err(|error| {
                eprintln!("Floatcraft could not initialize its local runtime: {error}");
                std::io::Error::other(error)
            })?;
            app.manage(runtime);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_catalog,
            get_sync_status,
            sync_catalog_now,
            get_market_status,
            get_settings,
            set_market_enabled,
            acknowledge_live_market_notice,
            get_regression_status,
            analyze_contract,
            preview_partial_contract,
            plan_contract,
            record_contract
        ])
        .run(tauri::generate_context!())
        .expect("error while running Floatcraft");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_data_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("floatcraft-{name}-{}", now_epoch()))
    }

    #[test]
    fn local_store_initializes_settings_and_a_backup() {
        let directory = temporary_data_dir("sqlite");
        let store = LocalStore::open(&directory).unwrap();
        let settings = store.settings().unwrap();
        assert!(settings.market_enabled);
        assert!(!settings.live_market_notice_acknowledged);
        assert!(directory.join(DATABASE_FILE).exists());
        assert!(directory.join("floatcraft.sqlite3.backup").exists());
        drop(store);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn corrupt_database_is_quarantined_and_recreated() {
        let directory = temporary_data_dir("corrupt");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(DATABASE_FILE), b"not a sqlite database").unwrap();
        let store = LocalStore::open(&directory).unwrap();
        assert!(store.settings().unwrap().market_enabled);
        drop(store);
        let quarantined = fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(quarantined);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn catalog_and_sync_state_round_trip_through_sqlite() {
        let directory = temporary_data_dir("catalog-state");
        let store = LocalStore::open(&directory).unwrap();
        let catalog = Catalog::load_embedded().unwrap();
        let status = SyncStatus {
            enabled: true,
            interval_seconds: 86_400,
            data_dir: directory.display().to_string(),
            running: false,
            last_started_at: Some(10),
            last_success_at: Some(11),
            last_error: None,
            bymykel_commit: Some("commit".to_owned()),
            steamtracking_commit: Some("steam".to_owned()),
            bymykel_commit_etag: Some("etag-a".to_owned()),
            steamtracking_commit_etag: Some("etag-b".to_owned()),
            imported_skins: 2,
            imported_collections: 1,
            caps_verification: None,
            stored_contract_regressions: 0,
        };
        store.record_success(&catalog, &status).unwrap();
        assert_eq!(
            store.load_catalog().unwrap().schema_version,
            catalog.schema_version
        );
        assert_eq!(store.load_status().unwrap().last_success_at, Some(11));
        drop(store);
        let _ = fs::remove_dir_all(directory);
    }
}
