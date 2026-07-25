use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{Client, header::USER_AGENT};
use serde::{Deserialize, Serialize};
use tokio::{fs, sync::RwLock, time::interval};

use crate::catalog::Catalog;

const BYMYKEL_REPO: &str = "ByMykel/CSGO-API";
const STEAMTRACKING_REPO: &str = "SteamTracking/GameTracking-CS2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsVerification {
    pub compared: usize,
    pub matching: usize,
    pub mismatching: usize,
    pub not_directly_verifiable: usize,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub data_dir: String,
    pub running: bool,
    pub last_started_at: Option<u64>,
    pub last_success_at: Option<u64>,
    pub last_error: Option<String>,
    pub bymykel_commit: Option<String>,
    pub steamtracking_commit: Option<String>,
    pub imported_skins: usize,
    pub imported_collections: usize,
    pub caps_verification: Option<CapsVerification>,
    pub stored_contract_regressions: usize,
}

impl SyncStatus {
    fn initial(enabled: bool, interval_seconds: u64, data_dir: &Path) -> Self {
        Self {
            enabled,
            interval_seconds,
            data_dir: data_dir.display().to_string(),
            running: false,
            last_started_at: None,
            last_success_at: None,
            last_error: None,
            bymykel_commit: None,
            steamtracking_commit: None,
            imported_skins: 0,
            imported_collections: 0,
            caps_verification: None,
            stored_contract_regressions: 0,
        }
    }
}

#[derive(Clone)]
pub struct SyncService {
    client: Client,
    data_dir: PathBuf,
    state: Arc<RwLock<SyncStatus>>,
    catalog: Arc<RwLock<Catalog>>,
    interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct GitHubCommit {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct UpstreamSkin {
    paint_index: Option<String>,
    min_float: Option<f32>,
    max_float: Option<f32>,
}

impl SyncService {
    pub fn from_environment(catalog: Arc<RwLock<Catalog>>) -> Result<Self, String> {
        let data_dir = std::env::var("SYNC_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("runtime"));
        let interval_seconds = std::env::var("CATALOG_SYNC_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(86_400)
            .max(60);
        let enabled = std::env::var("CATALOG_SYNC_ENABLED")
            .map(|value| value != "false" && value != "0")
            .unwrap_or(true);
        let client = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|error| format!("cannot create sync HTTP client: {error}"))?;
        let state = Arc::new(RwLock::new(SyncStatus::initial(
            enabled,
            interval_seconds,
            &data_dir,
        )));
        Ok(Self {
            client,
            data_dir,
            state,
            catalog,
            interval_seconds,
        })
    }

    pub async fn status(&self) -> SyncStatus {
        self.state.read().await.clone()
    }

    pub fn start(self: Arc<Self>) {
        tokio::spawn(async move {
            let enabled = self.state.read().await.enabled;
            if !enabled {
                return;
            }
            self.sync_now().await;
            let mut ticker = interval(Duration::from_secs(self.interval_seconds));
            ticker.tick().await;
            loop {
                ticker.tick().await;
                self.sync_now().await;
            }
        });
    }

    pub async fn sync_now(&self) {
        {
            let mut state = self.state.write().await;
            if state.running {
                return;
            }
            state.running = true;
            state.last_started_at = Some(now_epoch());
            state.last_error = None;
        }

        let result = self.sync_once().await;
        let mut state = self.state.write().await;
        state.running = false;
        match result {
            Ok(result) => {
                *self.catalog.write().await = result.catalog;
                state.last_success_at = Some(now_epoch());
                state.bymykel_commit = Some(result.bymykel_commit);
                state.steamtracking_commit = Some(result.steamtracking_commit);
                state.imported_skins = result.imported_skins;
                state.imported_collections = result.imported_collections;
                state.caps_verification = Some(result.verification);
                state.stored_contract_regressions = read_regression_count(&self.data_dir).await;
            }
            Err(error) => state.last_error = Some(error),
        }
        let _ = persist_status(&self.data_dir, &state).await;
    }

    async fn sync_once(&self) -> Result<SyncResult, String> {
        fs::create_dir_all(&self.data_dir)
            .await
            .map_err(|error| error.to_string())?;
        let bymykel_commit = self.resolve_commit(BYMYKEL_REPO, "main").await?;
        let steamtracking_commit = self.resolve_commit(STEAMTRACKING_REPO, "master").await?;
        let bymykel_base = format!(
            "https://raw.githubusercontent.com/{BYMYKEL_REPO}/{bymykel_commit}/public/api/en"
        );
        let skins = self
            .get_bytes(&format!("{bymykel_base}/skins.json"))
            .await?;
        let collections = self
            .get_bytes(&format!("{bymykel_base}/collections.json"))
            .await?;
        let items_game = self.get_bytes(&format!("https://raw.githubusercontent.com/{STEAMTRACKING_REPO}/{steamtracking_commit}/game/csgo/pak01_dir/scripts/items/items_game.txt")).await?;

        let retrieved_at = now_epoch();
        let result = materialize_sync_result(
            &bymykel_commit,
            &steamtracking_commit,
            &skins,
            &collections,
            &items_game,
            retrieved_at.to_string(),
        )?;

        let snapshot_dir = self.data_dir.join("catalog").join(&bymykel_commit);
        fs::create_dir_all(&snapshot_dir)
            .await
            .map_err(|error| error.to_string())?;
        fs::write(snapshot_dir.join("skins.json"), &skins)
            .await
            .map_err(|error| error.to_string())?;
        fs::write(snapshot_dir.join("collections.json"), &collections)
            .await
            .map_err(|error| error.to_string())?;
        fs::write(
            snapshot_dir.join("verification.json"),
            serde_json::to_vec_pretty(&result.verification).map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
        fs::write(
            self.data_dir.join("active-catalog.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "bymykel_commit": bymykel_commit,
                "steamtracking_commit": steamtracking_commit,
                "skins_path": snapshot_dir.join("skins.json"),
                "collections_path": snapshot_dir.join("collections.json"),
                "verified_at": retrieved_at
            }))
            .map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
        result.catalog.store_runtime(&self.data_dir)?;

        Ok(result)
    }

    async fn resolve_commit(&self, repository: &str, branch: &str) -> Result<String, String> {
        let url = format!("https://api.github.com/repos/{repository}/commits/{branch}");
        let response = self
            .client
            .get(url)
            .header(USER_AGENT, "cs-float-planner/0.1")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "GitHub commit lookup failed for {repository}: {}",
                response.status()
            ));
        }
        response
            .json::<GitHubCommit>()
            .await
            .map(|commit| commit.sha)
            .map_err(|error| error.to_string())
    }

    async fn get_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = self
            .client
            .get(url)
            .header(USER_AGENT, "cs-float-planner/0.1")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("download failed: {url} ({})", response.status()));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    }
}

struct SyncResult {
    bymykel_commit: String,
    steamtracking_commit: String,
    imported_skins: usize,
    imported_collections: usize,
    verification: CapsVerification,
    catalog: Catalog,
}

/// Converts immutable upstream bytes into the exact catalog artifact used by
/// the planner.  Network and filesystem work deliberately remain outside this
/// function, so the importer is reproducible from checked-in fixtures.
fn materialize_sync_result(
    bymykel_commit: &str,
    steamtracking_commit: &str,
    skins: &[u8],
    collections: &[u8],
    items_game: &[u8],
    retrieved_at: String,
) -> Result<SyncResult, String> {
    let upstream_skins: Vec<UpstreamSkin> = serde_json::from_slice(skins)
        .map_err(|error| format!("cannot parse ByMykel skins.json: {error}"))?;
    let imported_collections: Vec<serde_json::Value> = serde_json::from_slice(collections)
        .map_err(|error| format!("cannot parse ByMykel collections.json: {error}"))?;
    let valve_caps =
        parse_valve_paint_caps(std::str::from_utf8(items_game).map_err(|error| error.to_string())?);
    let verification = verify_caps(&upstream_skins, &valve_caps);
    let catalog = Catalog::from_upstream_snapshot(skins, bymykel_commit, retrieved_at)?;
    Ok(SyncResult {
        bymykel_commit: bymykel_commit.to_owned(),
        steamtracking_commit: steamtracking_commit.to_owned(),
        imported_skins: upstream_skins.len(),
        imported_collections: imported_collections.len(),
        verification,
        catalog,
    })
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn persist_status(data_dir: &Path, status: &SyncStatus) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .await
        .map_err(|error| error.to_string())?;
    fs::write(
        data_dir.join("sync-status.json"),
        serde_json::to_vec_pretty(status).map_err(|error| error.to_string())?,
    )
    .await
    .map_err(|error| error.to_string())
}

async fn read_regression_count(data_dir: &Path) -> usize {
    let path = data_dir.join("actual-contracts.json");
    fs::read(path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).ok())
        .map_or(0, |records| records.len())
}

fn parse_valve_paint_caps(text: &str) -> BTreeMap<String, (f32, f32)> {
    let Some(section) = named_block(text, "paint_kits") else {
        return BTreeMap::new();
    };
    named_blocks(section)
        .into_iter()
        .filter_map(|(id, body)| {
            if id.parse::<u32>().is_err() {
                return None;
            }
            let (Some(min), Some(max)) = (
                field_float(body, "wear_remap_min"),
                field_float(body, "wear_remap_max"),
            ) else {
                return None;
            };
            Some((id.to_owned(), (min, max)))
        })
        .collect()
}

fn named_block<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("\"{name}\"");
    let start = text.find(&key)? + key.len();
    let open = text[start..].find('{')? + start;
    block_contents(&text[open..])
}

fn named_blocks(text: &str) -> Vec<(&str, &str)> {
    let mut results = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(relative_quote) = text[cursor..].find('"') else {
            break;
        };
        let name_start = cursor + relative_quote + 1;
        let Some(name_end_relative) = text[name_start..].find('"') else {
            break;
        };
        let name_end = name_start + name_end_relative;
        let name = &text[name_start..name_end];
        let after_name = name_end + 1;
        let Some(open_relative) = text[after_name..].find('{') else {
            break;
        };
        let open = after_name + open_relative;
        let Some(body) = block_contents(&text[open..]) else {
            break;
        };
        results.push((name, body));
        cursor = open + body.len() + 2;
    }
    results
}

fn block_contents(text: &str) -> Option<&str> {
    let mut depth = 0_i32;
    let mut quoted = false;
    let mut escape = false;
    for (index, character) in text.char_indices() {
        if quoted {
            if escape {
                escape = false;
            } else if character == '\\' {
                escape = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        if character == '"' {
            quoted = true;
            continue;
        }
        if character == '{' {
            depth += 1;
        }
        if character == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(&text[1..index]);
            }
        }
    }
    None
}

fn field_float(text: &str, field: &str) -> Option<f32> {
    let key = format!("\"{field}\"");
    let start = text.find(&key)? + key.len();
    let value_start = text[start..].find('"')? + start + 1;
    let value_end = text[value_start..].find('"')? + value_start;
    text[value_start..value_end].parse().ok()
}

fn verify_caps(
    skins: &[UpstreamSkin],
    valve_caps: &BTreeMap<String, (f32, f32)>,
) -> CapsVerification {
    let mut result = CapsVerification {
        compared: 0,
        matching: 0,
        mismatching: 0,
        not_directly_verifiable: 0,
        examples: Vec::new(),
    };
    for skin in skins {
        let (Some(paint_index), Some(min), Some(max)) =
            (skin.paint_index.as_ref(), skin.min_float, skin.max_float)
        else {
            continue;
        };
        let Some((valve_min, valve_max)) = valve_caps.get(paint_index) else {
            result.not_directly_verifiable += 1;
            continue;
        };
        result.compared += 1;
        if (min - valve_min).abs() <= 0.000_001 && (max - valve_max).abs() <= 0.000_001 {
            result.matching += 1;
        } else {
            result.mismatching += 1;
            if result.examples.len() < 20 {
                result.examples.push(format!(
                    "paint {}: ByMykel [{min}, {max}] vs Valve [{valve_min}, {valve_max}]",
                    paint_index
                ));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valve_paint_cap_block() {
        let vdf = r#""paint_kits" { "282" { "wear_remap_min" "0.10" "wear_remap_max" "0.70" } }"#;
        assert_eq!(parse_valve_paint_caps(vdf).get("282"), Some(&(0.1, 0.7)));
    }

    #[test]
    fn materializes_a_versioned_catalog_from_fixed_source_bytes() {
        let skins = br#"[
          {"id":"input","name":"Input","min_float":0.1,"max_float":0.7,"rarity":{"id":"rarity_legendary_weapon"},"stattrak":true,"paint_index":"282","collections":[{"id":"collection-test","name":"Test"}]},
          {"id":"output","name":"Output","min_float":0.0,"max_float":1.0,"rarity":{"id":"rarity_ancient_weapon"},"stattrak":true,"paint_index":"283","collections":[{"id":"collection-test","name":"Test"}]}
        ]"#;
        let collections = br#"[{"id":"collection-test"}]"#;
        let items_game = br#""paint_kits" {
          "282" { "wear_remap_min" "0.10" "wear_remap_max" "0.70" }
          "283" { "wear_remap_min" "0.00" "wear_remap_max" "1.00" }
        }"#;

        let result = materialize_sync_result(
            "bymykel-fixture",
            "steamtracking-fixture",
            skins,
            collections,
            items_game,
            "1700000000".to_owned(),
        )
        .unwrap();

        assert_eq!(result.bymykel_commit, "bymykel-fixture");
        assert_eq!(result.steamtracking_commit, "steamtracking-fixture");
        assert_eq!(result.imported_skins, 2);
        assert_eq!(result.imported_collections, 1);
        assert_eq!(result.verification.matching, 2);
        assert_eq!(result.catalog.schema_version, "bymykel-bymykel-fixture");
        assert_eq!(result.catalog.source.retrieved_at, "1700000000");
        assert!(result.catalog.skin("collection-test/output").is_some());
    }
}
