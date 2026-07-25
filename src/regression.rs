use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::{fs, sync::Mutex};

use crate::{
    catalog::Catalog,
    domain::{
        AnalyzeRequest, Float32Value, InputRequest, PlannerError, analyze_contract, f32_abs,
        f32_sub,
    },
};

#[derive(Debug, Clone, Deserialize)]
pub struct ActualContractSubmission {
    pub inputs: Vec<InputRequest>,
    pub actual_output_skin_id: String,
    pub actual_output_float: f32,
    #[serde(default)]
    pub evidence_url: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionRecord {
    pub id: String,
    pub submitted_at: u64,
    pub inputs: Vec<InputRequest>,
    pub actual_output_skin_id: String,
    pub actual_output_float: Float32Value,
    /// A public capture, video, Steam profile, or other durable record that
    /// shows the submitted contract and its output.  This is required for new
    /// records; the default keeps previously persisted local records readable.
    #[serde(default)]
    pub evidence_url: Option<String>,
    pub note: Option<String>,
    /// The exact catalog revision used for the prediction.  Float caps and
    /// collection paths can change upstream, so a regression without this
    /// provenance is not reproducible after a catalog sync.
    #[serde(default)]
    pub catalog_schema_version: String,
    #[serde(default)]
    pub catalog_source_url: String,
    pub predicted_float: Float32Value,
    pub exact_float32_match: bool,
    pub absolute_difference: Float32Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegressionStatus {
    pub count: usize,
    pub evidence_backed_records: usize,
    pub evidence_backed_exact_matches: usize,
    pub records_missing_evidence: usize,
    pub exact_matches: usize,
    pub mismatches: usize,
    pub ready_for_target: bool,
    pub target_count: usize,
}

#[derive(Clone)]
pub struct RegressionStore {
    path: PathBuf,
    gate: Arc<Mutex<()>>,
    clock: Arc<dyn RegressionClock>,
}

trait RegressionClock: Send + Sync {
    fn now_epoch(&self) -> u64;
}

struct SystemRegressionClock;

impl RegressionClock for SystemRegressionClock {
    fn now_epoch(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

impl RegressionStore {
    pub fn from_environment() -> Self {
        let data_dir = std::env::var("SYNC_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("runtime"));
        Self::from_path(data_dir.join("actual-contracts.json"))
    }

    fn from_path(path: PathBuf) -> Self {
        Self {
            path,
            gate: Arc::new(Mutex::new(())),
            clock: Arc::new(SystemRegressionClock),
        }
    }

    pub async fn status(&self) -> RegressionStatus {
        regression_status(&self.read_all().await)
    }

    pub async fn submit(
        &self,
        catalog: &Catalog,
        submission: ActualContractSubmission,
    ) -> Result<RegressionRecord, PlannerError> {
        let _guard = self.gate.lock().await;
        let evidence_url = validate_evidence_url(submission.evidence_url.as_deref())?;
        let analysis = analyze_contract(
            catalog,
            AnalyzeRequest {
                contract_size: 10,
                stattrak: false,
                inputs: submission.inputs.clone(),
            },
        )?;
        let outcome = analysis
            .outcomes
            .iter()
            .find(|outcome| outcome.skin_id == submission.actual_output_skin_id)
            .ok_or_else(|| {
                PlannerError::Validation(
                    "actual output is not a possible outcome of the submitted contract".to_owned(),
                )
            })?;
        let actual = Float32Value::new(submission.actual_output_float);
        let difference = f32_abs(f32_sub(
            outcome.predicted_float.value,
            submission.actual_output_float,
        ));
        let id = regression_id(&submission, &evidence_url);
        let mut records = self.read_all().await;
        if records.iter().any(|record| record.id == id) {
            return Err(PlannerError::Validation(
                "this exact evidence-backed contract has already been recorded".to_owned(),
            ));
        }
        let record = RegressionRecord {
            id,
            submitted_at: self.clock.now_epoch(),
            inputs: submission.inputs,
            actual_output_skin_id: submission.actual_output_skin_id,
            actual_output_float: actual.clone(),
            evidence_url: Some(evidence_url),
            note: submission.note,
            catalog_schema_version: catalog.schema_version.clone(),
            catalog_source_url: catalog.source.url.clone(),
            predicted_float: outcome.predicted_float.clone(),
            exact_float32_match: outcome.predicted_float.bits == actual.bits,
            absolute_difference: Float32Value::new(difference),
        };
        records.push(record.clone());
        let directory = self.path.parent().expect("regression path has a parent");
        fs::create_dir_all(directory)
            .await
            .map_err(|error| PlannerError::Catalog(error.to_string()))?;
        fs::write(
            &self.path,
            serde_json::to_vec_pretty(&records)
                .map_err(|error| PlannerError::Catalog(error.to_string()))?,
        )
        .await
        .map_err(|error| PlannerError::Catalog(error.to_string()))?;
        Ok(record)
    }

    async fn read_all(&self) -> Vec<RegressionRecord> {
        fs::read(&self.path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

fn regression_status(records: &[RegressionRecord]) -> RegressionStatus {
    let evidence_backed_records = records
        .iter()
        .filter(|record| record.evidence_url.is_some())
        .count();
    let exact_matches = records
        .iter()
        .filter(|record| record.exact_float32_match)
        .count();
    let evidence_backed_exact_matches = records
        .iter()
        .filter(|record| record.evidence_url.is_some() && record.exact_float32_match)
        .count();
    RegressionStatus {
        count: records.len(),
        evidence_backed_records,
        evidence_backed_exact_matches,
        records_missing_evidence: records.len() - evidence_backed_records,
        exact_matches,
        mismatches: records
            .iter()
            .filter(|record| !record.exact_float32_match)
            .count(),
        ready_for_target: evidence_backed_exact_matches >= 20,
        target_count: 20,
    }
}

fn regression_id(submission: &ActualContractSubmission, evidence_url: &str) -> String {
    // FNV-1a is deliberately simple here: it supplies a stable content key,
    // not a cryptographic proof.  The evidence URL and every input bit are
    // included, so retries get the same ID and cannot silently duplicate data.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for input in &submission.inputs {
        fnv1a(&mut hash, input.skin_id.as_bytes());
        fnv1a(&mut hash, &[0]);
        fnv1a(&mut hash, &input.float_value.to_bits().to_le_bytes());
    }
    fnv1a(&mut hash, submission.actual_output_skin_id.as_bytes());
    fnv1a(
        &mut hash,
        &submission.actual_output_float.to_bits().to_le_bytes(),
    );
    fnv1a(&mut hash, evidence_url.as_bytes());
    format!("contract-{hash:016x}")
}

fn fnv1a(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn validate_evidence_url(value: Option<&str>) -> Result<String, PlannerError> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PlannerError::Validation(
                "evidence_url is required for an actual contract regression".to_owned(),
            )
        })?;
    let url = Url::parse(value).map_err(|_| {
        PlannerError::Validation("evidence_url must be an absolute http(s) URL".to_owned())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(PlannerError::Validation(
            "evidence_url must be an absolute http(s) URL".to_owned(),
        ));
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::catalog::Catalog;

    struct FixedClock(u64);

    impl RegressionClock for FixedClock {
        fn now_epoch(&self) -> u64 {
            self.0
        }
    }

    fn temporary_path() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("floatcraft-regression-{stamp}-{sequence}.json"))
    }

    fn test_store(path: PathBuf) -> RegressionStore {
        RegressionStore {
            path,
            gate: Arc::new(Mutex::new(())),
            clock: Arc::new(FixedClock(1_700_000_000)),
        }
    }

    fn dragon_lore_submission() -> ActualContractSubmission {
        ActualContractSubmission {
            inputs: (0..10)
                .map(|_| InputRequest {
                    skin_id: "m4a1s-knight".to_owned(),
                    float_value: 0.01,
                    inspect_link: None,
                })
                .collect(),
            actual_output_skin_id: "awp-dragon-lore".to_owned(),
            actual_output_float: 0.07,
            evidence_url: Some("https://example.test/evidence/dragon-lore".to_owned()),
            note: None,
        }
    }

    #[test]
    fn evidence_url_must_be_a_public_http_url() {
        assert!(validate_evidence_url(Some("https://example.com/proof/123")).is_ok());
        assert!(validate_evidence_url(None).is_err());
        assert!(validate_evidence_url(Some("steam://inspect/123")).is_err());
        assert!(validate_evidence_url(Some("not a URL")).is_err());
    }

    #[tokio::test]
    async fn record_uses_a_stable_content_id_and_rejects_duplicate_evidence() {
        let path = temporary_path();
        let store = test_store(path.clone());
        let catalog = Catalog::load_embedded().unwrap();
        let submission = dragon_lore_submission();

        let record = store.submit(&catalog, submission.clone()).await.unwrap();
        assert_eq!(
            record.id,
            regression_id(&submission, "https://example.test/evidence/dragon-lore")
        );
        assert_eq!(record.submitted_at, 1_700_000_000);
        assert!(record.exact_float32_match);

        let error = store.submit(&catalog, submission).await.unwrap_err();
        assert!(error.to_string().contains("already been recorded"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn readiness_requires_twenty_evidence_backed_exact_records() {
        let base = RegressionRecord {
            id: "fixture".to_owned(),
            submitted_at: 0,
            inputs: Vec::new(),
            actual_output_skin_id: "output".to_owned(),
            actual_output_float: Float32Value::new(0.1),
            evidence_url: Some("https://example.test/evidence".to_owned()),
            note: None,
            catalog_schema_version: "fixture".to_owned(),
            catalog_source_url: "https://example.test/catalog".to_owned(),
            predicted_float: Float32Value::new(0.1),
            exact_float32_match: true,
            absolute_difference: Float32Value::new(0.0),
        };
        let nineteen = vec![base.clone(); 19];
        assert!(!regression_status(&nineteen).ready_for_target);
        let twenty = vec![base; 20];
        assert!(regression_status(&twenty).ready_for_target);
    }
}
