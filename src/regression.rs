use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

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
    pub evidence_url: Option<String>,
    pub note: Option<String>,
    pub predicted_float: Float32Value,
    pub exact_float32_match: bool,
    pub absolute_difference: Float32Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegressionStatus {
    pub count: usize,
    pub exact_matches: usize,
    pub mismatches: usize,
    pub ready_for_target: bool,
    pub target_count: usize,
}

#[derive(Clone)]
pub struct RegressionStore {
    path: PathBuf,
    gate: Arc<Mutex<()>>,
}

impl RegressionStore {
    pub fn from_environment() -> Self {
        let data_dir = std::env::var("SYNC_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("runtime"));
        Self {
            path: data_dir.join("actual-contracts.json"),
            gate: Arc::new(Mutex::new(())),
        }
    }

    pub async fn status(&self) -> RegressionStatus {
        let records = self.read_all().await;
        RegressionStatus {
            count: records.len(),
            exact_matches: records
                .iter()
                .filter(|record| record.exact_float32_match)
                .count(),
            mismatches: records
                .iter()
                .filter(|record| !record.exact_float32_match)
                .count(),
            ready_for_target: records.len() >= 20,
            target_count: 20,
        }
    }

    pub async fn submit(
        &self,
        catalog: &Catalog,
        submission: ActualContractSubmission,
    ) -> Result<RegressionRecord, PlannerError> {
        let _guard = self.gate.lock().await;
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
        let record = RegressionRecord {
            id: format!("contract-{}-{}", now_epoch(), actual.bits),
            submitted_at: now_epoch(),
            inputs: submission.inputs,
            actual_output_skin_id: submission.actual_output_skin_id,
            actual_output_float: actual.clone(),
            evidence_url: submission.evidence_url,
            note: submission.note,
            predicted_float: outcome.predicted_float.clone(),
            exact_float32_match: outcome.predicted_float.bits == actual.bits,
            absolute_difference: Float32Value::new(difference),
        };
        let mut records = self.read_all().await;
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

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
