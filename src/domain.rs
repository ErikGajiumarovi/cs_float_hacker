use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::catalog::Catalog;

const WEAR_BOUNDARIES: [(f32, &str); 4] = [
    (0.07, "Minimal Wear"),
    (0.15, "Field-Tested"),
    (0.38, "Well-Worn"),
    (0.45, "Battle-Scarred"),
];

#[derive(Debug, Error)]
pub enum PlannerError {
    #[error("{0}")]
    Validation(String),
    #[error("internal catalog error: {0}")]
    Catalog(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Consumer,
    Industrial,
    MilSpec,
    Restricted,
    Classified,
    Covert,
    Extraordinary,
}

impl Rarity {
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Consumer => Some(Self::Industrial),
            Self::Industrial => Some(Self::MilSpec),
            Self::MilSpec => Some(Self::Restricted),
            Self::Restricted => Some(Self::Classified),
            Self::Classified => Some(Self::Covert),
            Self::Covert | Self::Extraordinary => None,
        }
    }

    pub fn previous(self) -> Option<Self> {
        match self {
            Self::Industrial => Some(Self::Consumer),
            Self::MilSpec => Some(Self::Industrial),
            Self::Restricted => Some(Self::MilSpec),
            Self::Classified => Some(Self::Restricted),
            Self::Covert => Some(Self::Classified),
            Self::Consumer | Self::Extraordinary => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skin {
    pub id: String,
    pub name: String,
    pub collection_id: String,
    pub collection_name: String,
    pub rarity: Rarity,
    pub min_float: f32,
    pub max_float: f32,
    pub stattrak_supported: bool,
    #[serde(default)]
    pub def_index: Option<u32>,
    #[serde(default)]
    pub paint_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureListing {
    pub id: String,
    pub skin_id: String,
    pub float_value: f32,
    pub price_cents: u64,
    pub market_url: String,
    pub source: String,
    #[serde(default)]
    pub inspect_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Float32Value {
    pub value: f32,
    pub bits: u32,
    pub display: String,
}

impl Float32Value {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            bits: value.to_bits(),
            display: format!("{value:.9}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WearInfo {
    pub name: &'static str,
    pub distance_to_nearest_boundary: Float32Value,
    pub nearest_boundary: Float32Value,
    pub near_boundary: bool,
}

pub fn wear_info(value: f32) -> WearInfo {
    let name = if value < 0.07 {
        "Factory New"
    } else if value < 0.15 {
        "Minimal Wear"
    } else if value < 0.38 {
        "Field-Tested"
    } else if value < 0.45 {
        "Well-Worn"
    } else {
        "Battle-Scarred"
    };

    let (boundary, distance) = WEAR_BOUNDARIES
        .iter()
        .map(|(boundary, _)| (*boundary, f32_abs(f32_sub(value, *boundary))))
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .unwrap_or((0.0, 0.0));

    WearInfo {
        name,
        distance_to_nearest_boundary: Float32Value::new(distance),
        nearest_boundary: Float32Value::new(boundary),
        near_boundary: distance <= 0.000_01,
    }
}

// These small operations intentionally keep each mathematical stage as f32.
// Do not replace them with f64 math followed by a final cast: that diverges at
// contract boundaries from the behavior validated against FloatJitsu.
#[inline(never)]
pub fn f32_add(left: f32, right: f32) -> f32 {
    f32::from_bits((left + right).to_bits())
}

#[inline(never)]
pub fn f32_sub(left: f32, right: f32) -> f32 {
    f32::from_bits((left - right).to_bits())
}

#[inline(never)]
pub fn f32_mul(left: f32, right: f32) -> f32 {
    f32::from_bits((left * right).to_bits())
}

#[inline(never)]
pub fn f32_div(left: f32, right: f32) -> f32 {
    f32::from_bits((left / right).to_bits())
}

#[inline(never)]
pub fn f32_abs(value: f32) -> f32 {
    f32::from_bits(value.abs().to_bits())
}

pub fn adjusted_float(raw: f32, skin: &Skin) -> Result<f32, PlannerError> {
    if !raw.is_finite() {
        return Err(PlannerError::Validation(format!(
            "float for {} must be finite",
            skin.name
        )));
    }
    if raw < skin.min_float || raw > skin.max_float {
        return Err(PlannerError::Validation(format!(
            "float {raw:.9} is outside {} cap [{:.9}, {:.9}]",
            skin.name, skin.min_float, skin.max_float
        )));
    }
    let numerator = f32_sub(raw, skin.min_float);
    let denominator = f32_sub(skin.max_float, skin.min_float);
    if denominator <= 0.0 {
        return Err(PlannerError::Catalog(format!(
            "{} has an invalid float cap",
            skin.name
        )));
    }
    Ok(f32_div(numerator, denominator))
}

pub fn output_float(average_adjusted: f32, skin: &Skin) -> f32 {
    let span = f32_sub(skin.max_float, skin.min_float);
    let scaled = f32_mul(average_adjusted, span);
    f32_add(skin.min_float, scaled)
}

pub fn average_adjusted(values: impl IntoIterator<Item = f32>, count: usize) -> f32 {
    let sum = values.into_iter().fold(0.0_f32, f32_add);
    f32_div(sum, count as f32)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InputRequest {
    pub skin_id: String,
    pub float_value: f32,
    #[serde(default)]
    pub inspect_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnalyzeRequest {
    pub contract_size: usize,
    #[serde(default)]
    pub stattrak: bool,
    pub inputs: Vec<InputRequest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartialInputRequest {
    pub skin_id: String,
    #[serde(default)]
    pub float_value: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartialAnalyzeRequest {
    #[serde(default)]
    pub stattrak: bool,
    pub inputs: Vec<PartialInputRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputAnalysis {
    pub slot: usize,
    pub skin_id: String,
    pub skin_name: String,
    pub collection_name: String,
    pub rarity: Rarity,
    pub raw_float: Float32Value,
    pub adjusted_float: Float32Value,
    pub inspect_link_supplied: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeAnalysis {
    pub skin_id: String,
    pub skin_name: String,
    pub collection_name: String,
    pub probability: f32,
    pub probability_percent: String,
    pub predicted_float: Float32Value,
    pub wear: WearInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeResponse {
    pub valid: bool,
    pub contract_size: usize,
    pub input_rarity: Rarity,
    pub average_adjusted: Float32Value,
    pub inputs: Vec<InputAnalysis>,
    pub outcomes: Vec<OutcomeAnalysis>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartialOutcomeAnalysis {
    pub skin_id: String,
    pub skin_name: String,
    pub collection_name: String,
    pub known_inputs_from_collection: usize,
    /// Until all ten slots are chosen, the exact odds are unknown. These are
    /// the lower and upper bounds permitted by the already selected inputs.
    pub probability_min: f32,
    pub probability_max: f32,
    pub predicted_float_min: Float32Value,
    pub predicted_float_max: Float32Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartialAnalyzeResponse {
    pub input_rarity: Rarity,
    pub selected_slots: usize,
    pub floats_specified: usize,
    pub remaining_float_slots: usize,
    pub outcomes: Vec<PartialOutcomeAnalysis>,
}

#[derive(Debug, Clone)]
struct ResolvedInput<'a> {
    skin: &'a Skin,
    float_value: f32,
    inspect_link_supplied: bool,
}

fn resolve_inputs<'a>(
    catalog: &'a Catalog,
    request: &[InputRequest],
    stattrak: bool,
) -> Result<Vec<ResolvedInput<'a>>, PlannerError> {
    request
        .iter()
        .map(|input| {
            let skin = catalog.skin(&input.skin_id).ok_or_else(|| {
                PlannerError::Validation(format!("unknown skin_id: {}", input.skin_id))
            })?;
            if stattrak && !skin.stattrak_supported {
                return Err(PlannerError::Validation(format!(
                    "{} has no StatTrak variant in the catalog",
                    skin.name
                )));
            }
            adjusted_float(input.float_value, skin)?;
            Ok(ResolvedInput {
                skin,
                float_value: input.float_value,
                inspect_link_supplied: input.inspect_link.is_some(),
            })
        })
        .collect()
}

/// Previews only the information that is already implied by a partially filled
/// 10-item normal contract. Unknown slots are deliberately modeled as the full
/// adjusted-float interval [0, 1], so the displayed float range is possible,
/// not a fabricated exact prediction.
pub fn preview_partial_contract(
    catalog: &Catalog,
    request: PartialAnalyzeRequest,
) -> Result<PartialAnalyzeResponse, PlannerError> {
    if request.inputs.is_empty() || request.inputs.len() > 10 {
        return Err(PlannerError::Validation(
            "choose between 1 and 10 contract inputs for a preview".to_owned(),
        ));
    }
    let mut resolved = Vec::new();
    for input in &request.inputs {
        let skin = catalog.skin(&input.skin_id).ok_or_else(|| {
            PlannerError::Validation(format!("unknown skin_id: {}", input.skin_id))
        })?;
        if request.stattrak && !skin.stattrak_supported {
            return Err(PlannerError::Validation(format!(
                "{} has no StatTrak variant in the catalog", skin.name
            )));
        }
        if let Some(value) = input.float_value {
            adjusted_float(value, skin)?;
        }
        resolved.push((skin, input.float_value));
    }
    let input_rarity = resolved[0].0.rarity;
    if resolved.iter().any(|(skin, _)| skin.rarity != input_rarity) {
        return Err(PlannerError::Validation(
            "all selected contract inputs must have the same rarity".to_owned(),
        ));
    }
    let output_rarity = input_rarity.next().ok_or_else(|| {
        PlannerError::Validation("this input rarity has no standard 10-item trade-up tier".to_owned())
    })?;

    let mut known_sum = 0.0_f32;
    let mut floats_specified = 0_usize;
    let mut collection_counts = BTreeMap::<String, usize>::new();
    for (skin, float_value) in &resolved {
        *collection_counts.entry(skin.collection_id.clone()).or_default() += 1;
        if let Some(value) = float_value {
            known_sum = f32_add(known_sum, adjusted_float(*value, skin)?);
            floats_specified += 1;
        }
    }
    let remaining_float_slots = 10 - floats_specified;
    let min_average = f32_div(known_sum, 10.0);
    let max_average = f32_div(f32_add(known_sum, remaining_float_slots as f32), 10.0);

    let mut outcomes = BTreeMap::<String, (usize, &Skin)>::new();
    for (collection_id, count) in collection_counts {
        let possible = catalog.outcomes_for_collection(&collection_id, output_rarity);
        if possible.is_empty() {
            return Err(PlannerError::Validation(format!(
                "collection {collection_id} has no {:?} outcome for this trade-up", output_rarity
            )));
        }
        for skin in possible {
            let entry = outcomes.entry(skin.id.clone()).or_insert((0, skin));
            entry.0 += count;
        }
    }
    Ok(PartialAnalyzeResponse {
        input_rarity,
        selected_slots: resolved.len(),
        floats_specified,
        remaining_float_slots,
        outcomes: outcomes
            .into_values()
            .map(|(known_inputs_from_collection, skin)| {
                let outcomes_in_collection = catalog
                    .outcomes_for_collection(&skin.collection_id, output_rarity)
                    .len();
                let divisor = outcomes_in_collection as f32;
                PartialOutcomeAnalysis {
                    skin_id: skin.id.clone(),
                    skin_name: skin.name.clone(),
                    collection_name: skin.collection_name.clone(),
                    known_inputs_from_collection,
                    probability_min: f32_div(known_inputs_from_collection as f32, 10.0 * divisor),
                    probability_max: f32_div(1.0, divisor),
                    predicted_float_min: Float32Value::new(output_float(min_average, skin)),
                    predicted_float_max: Float32Value::new(output_float(max_average, skin)),
                }
            })
            .collect(),
    })
}

pub fn analyze_contract(
    catalog: &Catalog,
    request: AnalyzeRequest,
) -> Result<AnalyzeResponse, PlannerError> {
    if !matches!(request.contract_size, 5 | 10) {
        return Err(PlannerError::Validation(
            "contract_size must be either 5 or 10".to_owned(),
        ));
    }
    if request.inputs.len() != request.contract_size {
        return Err(PlannerError::Validation(format!(
            "contract requires exactly {} inputs, received {}",
            request.contract_size,
            request.inputs.len()
        )));
    }
    if request.contract_size == 5 {
        return Err(PlannerError::Validation(
            "5 Covert → knife/gloves is recognized, but this MVP catalog intentionally has no fabricated unusual/loot outcome pool yet".to_owned(),
        ));
    }

    let inputs = resolve_inputs(catalog, &request.inputs, request.stattrak)?;
    let input_rarity = inputs
        .first()
        .map(|input| input.skin.rarity)
        .ok_or_else(|| PlannerError::Validation("contract has no inputs".to_owned()))?;
    if inputs.iter().any(|input| input.skin.rarity != input_rarity) {
        return Err(PlannerError::Validation(
            "all contract inputs must have the same rarity".to_owned(),
        ));
    }
    let Some(output_rarity) = input_rarity.next() else {
        return Err(PlannerError::Validation(
            "this input rarity has no standard 10-item trade-up tier".to_owned(),
        ));
    };

    let adjusted: Vec<f32> = inputs
        .iter()
        .map(|input| adjusted_float(input.float_value, input.skin))
        .collect::<Result<_, _>>()?;
    let average = average_adjusted(adjusted.iter().copied(), inputs.len());

    let mut collection_counts = BTreeMap::<String, usize>::new();
    for input in &inputs {
        *collection_counts
            .entry(input.skin.collection_id.clone())
            .or_default() += 1;
    }

    let mut outcomes = BTreeMap::<String, (usize, &Skin)>::new();
    for (collection_id, count) in collection_counts {
        let possible = catalog.outcomes_for_collection(&collection_id, output_rarity);
        if possible.is_empty() {
            return Err(PlannerError::Validation(format!(
                "collection {collection_id} has no {:?} outcome for this trade-up",
                output_rarity
            )));
        }
        for skin in possible {
            let entry = outcomes.entry(skin.id.clone()).or_insert((0, skin));
            entry.0 += count;
        }
    }

    let analyzed_outcomes = outcomes
        .into_values()
        .map(|(inputs_from_collection, skin)| {
            let collection_outcome_count = catalog
                .outcomes_for_collection(&skin.collection_id, output_rarity)
                .len();
            let probability = (inputs_from_collection as f32 / inputs.len() as f32)
                / collection_outcome_count as f32;
            let predicted = output_float(average, skin);
            OutcomeAnalysis {
                skin_id: skin.id.clone(),
                skin_name: skin.name.clone(),
                collection_name: skin.collection_name.clone(),
                probability,
                probability_percent: format!("{:.2}%", probability * 100.0),
                predicted_float: Float32Value::new(predicted),
                wear: wear_info(predicted),
            }
        })
        .collect();

    Ok(AnalyzeResponse {
        valid: true,
        contract_size: request.contract_size,
        input_rarity,
        average_adjusted: Float32Value::new(average),
        inputs: inputs
            .iter()
            .zip(adjusted)
            .enumerate()
            .map(|(index, (input, adjusted_float))| InputAnalysis {
                slot: index + 1,
                skin_id: input.skin.id.clone(),
                skin_name: input.skin.name.clone(),
                collection_name: input.skin.collection_name.clone(),
                rarity: input.skin.rarity,
                raw_float: Float32Value::new(input.float_value),
                adjusted_float: Float32Value::new(adjusted_float),
                inspect_link_supplied: input.inspect_link_supplied,
            })
            .collect(),
        outcomes: analyzed_outcomes,
        warnings: vec![
            "Вычисления выполняются нативным Rust-ядром как последовательные IEEE-754 float32; bits сохранены в ответе.".to_owned(),
            "Порядок слотов сохранён. Его влияние на граничные реальные крафты пока считается непроверенной гипотезой.".to_owned(),
        ],
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TargetConstraint {
    Exact { value: f32 },
    Maximum { value: f32 },
    Range { min: f32, max: f32 },
}

impl TargetConstraint {
    fn requested_bounds(
        &self,
        delta: f32,
        cap_min: f32,
        cap_max: f32,
    ) -> Result<(f32, f32, f32), PlannerError> {
        if !delta.is_finite() || delta < 0.0 {
            return Err(PlannerError::Validation(
                "delta must be a finite non-negative float".to_owned(),
            ));
        }
        let (low, high, center) = match *self {
            Self::Exact { value } => (f32_sub(value, delta), f32_add(value, delta), value),
            Self::Maximum { value } => (cap_min, f32_add(value, delta), value),
            Self::Range { min, max } => {
                if min > max {
                    return Err(PlannerError::Validation(
                        "target range min must not exceed max".to_owned(),
                    ));
                }
                (
                    f32_sub(min, delta),
                    f32_add(max, delta),
                    f32_div(f32_add(min, max), 2.0),
                )
            }
        };
        if !low.is_finite() || !high.is_finite() || !center.is_finite() {
            return Err(PlannerError::Validation(
                "target constraint must contain finite floats".to_owned(),
            ));
        }
        Ok((
            low.max(cap_min),
            high.min(cap_max),
            center.clamp(cap_min, cap_max),
        ))
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanningPriority {
    #[default]
    Cheapest,
    Closest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanRequest {
    pub target_skin_id: String,
    pub target: TargetConstraint,
    #[serde(default = "default_delta")]
    pub delta: f32,
    #[serde(default)]
    pub stattrak: bool,
    #[serde(default)]
    pub owned_inputs: Vec<InputRequest>,
    #[serde(default)]
    pub priority: PlanningPriority,
    #[serde(default)]
    pub listing_limit: Option<usize>,
}

fn default_delta() -> f32 {
    0.0001
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedListing {
    pub slot: usize,
    pub listing_id: Option<String>,
    pub source: String,
    pub skin_id: String,
    pub skin_name: String,
    pub float_value: Float32Value,
    pub adjusted_float: Float32Value,
    pub price_cents: Option<u64>,
    pub market_url: Option<String>,
    pub inspect_link: Option<String>,
    pub owned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanResponse {
    pub status: String,
    pub message: String,
    pub target_skin: Skin,
    pub target_adjusted: Float32Value,
    pub acceptable_output_range: [Float32Value; 2],
    pub contract_size: usize,
    pub target_probability: String,
    pub candidate_input_skins: Vec<Skin>,
    pub selected_inputs: Vec<PlannedListing>,
    pub total_price_cents: u64,
    pub pricing_available: bool,
    pub predicted_target_float: Float32Value,
    pub target_distance: Float32Value,
    pub target_wear: WearInfo,
    pub all_outcomes: Vec<OutcomeAnalysis>,
    pub optimizer: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct Candidate<'a> {
    listing: &'a FixtureListing,
    skin: &'a Skin,
    adjusted: f32,
}

#[derive(Debug, Clone)]
struct ChosenPlan<'a> {
    candidates: Vec<&'a Candidate<'a>>,
    predicted: f32,
    total_price_cents: u64,
    within_target: bool,
    distance: f32,
}

const BEAM_WIDTH: usize = 4_096;

#[derive(Debug, Clone)]
struct BeamState<'a> {
    candidates: Vec<&'a Candidate<'a>>,
    adjusted_sum: f32,
    total_price_cents: u64,
}

fn better_choice(
    candidate: &ChosenPlan<'_>,
    current: Option<&ChosenPlan<'_>>,
    priority: &PlanningPriority,
) -> bool {
    let Some(current) = current else {
        return true;
    };
    if candidate.within_target != current.within_target {
        return candidate.within_target;
    }
    let candidate_price_key = candidate.total_price_cents;
    let current_price_key = current.total_price_cents;
    match priority {
        PlanningPriority::Cheapest => {
            if candidate.within_target && current.within_target {
                (candidate_price_key, candidate.distance) < (current_price_key, current.distance)
            } else {
                (candidate.distance, candidate_price_key) < (current.distance, current_price_key)
            }
        }
        PlanningPriority::Closest => {
            (candidate.distance, candidate_price_key) < (current.distance, current_price_key)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn search_combinations<'a>(
    candidates: &'a [Candidate<'a>],
    owned: &[ResolvedInput<'a>],
    required: usize,
    target: &'a Skin,
    acceptable_low: f32,
    acceptable_high: f32,
    target_center: f32,
    priority: &PlanningPriority,
    start: usize,
    selected: &mut Vec<&'a Candidate<'a>>,
    best: &mut Option<ChosenPlan<'a>>,
) {
    if selected.len() == required {
        let total_price_cents = selected
            .iter()
            .map(|candidate| candidate.listing.price_cents)
            .sum();
        let adjusted_values = owned
            .iter()
            .map(|input| {
                adjusted_float(input.float_value, input.skin).expect("validated owned item")
            })
            .chain(selected.iter().map(|candidate| candidate.adjusted));
        let average = average_adjusted(adjusted_values, owned.len() + selected.len());
        let predicted = output_float(average, target);
        let plan = ChosenPlan {
            candidates: selected.clone(),
            predicted,
            total_price_cents,
            within_target: predicted >= acceptable_low && predicted <= acceptable_high,
            distance: f32_abs(f32_sub(predicted, target_center)),
        };
        if better_choice(&plan, best.as_ref(), priority) {
            *best = Some(plan);
        }
        return;
    }

    let needed = required - selected.len();
    for index in start..=candidates.len().saturating_sub(needed) {
        selected.push(&candidates[index]);
        search_combinations(
            candidates,
            owned,
            required,
            target,
            acceptable_low,
            acceptable_high,
            target_center,
            priority,
            index + 1,
            selected,
            best,
        );
        selected.pop();
    }
}

#[allow(clippy::too_many_arguments)]
fn search_combinations_beam<'a>(
    candidates: &'a [Candidate<'a>],
    owned: &[ResolvedInput<'a>],
    required: usize,
    target: &'a Skin,
    target_adjusted: f32,
    acceptable_low: f32,
    acceptable_high: f32,
    target_center: f32,
    priority: &PlanningPriority,
) -> Option<ChosenPlan<'a>> {
    let owned_sum = owned.iter().fold(0.0_f32, |sum, input| {
        f32_add(
            sum,
            adjusted_float(input.float_value, input.skin).expect("validated owned item"),
        )
    });
    let desired_missing_sum = f32_sub(f32_mul(target_adjusted, 10.0), owned_sum);
    let mut levels = (0..=required)
        .map(|_| Vec::<BeamState<'a>>::new())
        .collect::<Vec<_>>();
    levels[0].push(BeamState {
        candidates: Vec::new(),
        adjusted_sum: 0.0,
        total_price_cents: 0,
    });

    for (candidate_index, candidate) in candidates.iter().enumerate() {
        let maximum_count = required.min(candidate_index + 1);
        for count in (1..=maximum_count).rev() {
            let additions = levels[count - 1]
                .iter()
                .filter_map(|state| {
                    let total_price_cents = state
                        .total_price_cents
                        .checked_add(candidate.listing.price_cents)?;
                    let mut chosen = state.candidates.clone();
                    chosen.push(candidate);
                    Some(BeamState {
                        candidates: chosen,
                        adjusted_sum: f32_add(state.adjusted_sum, candidate.adjusted),
                        total_price_cents,
                    })
                })
                .collect::<Vec<_>>();
            levels[count].extend(additions);
            prune_beam_level(&mut levels[count], count, required, desired_missing_sum);
        }
    }

    let mut best = None;
    for state in &levels[required] {
        let average = average_adjusted(
            owned
                .iter()
                .map(|input| {
                    adjusted_float(input.float_value, input.skin).expect("validated owned item")
                })
                .chain(state.candidates.iter().map(|candidate| candidate.adjusted)),
            10,
        );
        let predicted = output_float(average, target);
        let candidate = ChosenPlan {
            candidates: state.candidates.clone(),
            predicted,
            total_price_cents: state.total_price_cents,
            within_target: predicted >= acceptable_low && predicted <= acceptable_high,
            distance: f32_abs(f32_sub(predicted, target_center)),
        };
        if better_choice(&candidate, best.as_ref(), priority) {
            best = Some(candidate);
        }
    }
    best
}

fn prune_beam_level(
    level: &mut Vec<BeamState<'_>>,
    count: usize,
    required: usize,
    desired_missing_sum: f32,
) {
    if level.len() <= BEAM_WIDTH {
        return;
    }
    let expected_partial_sum = f32_mul(desired_missing_sum, f32_div(count as f32, required as f32));
    level.sort_by(|left, right| {
        let left_distance = f32_abs(f32_sub(left.adjusted_sum, expected_partial_sum));
        let right_distance = f32_abs(f32_sub(right.adjusted_sum, expected_partial_sum));
        left_distance
            .total_cmp(&right_distance)
            .then_with(|| left.total_price_cents.cmp(&right.total_price_cents))
            .then_with(|| {
                left.candidates
                    .iter()
                    .map(|candidate| candidate.listing.id.as_str())
                    .cmp(
                        right
                            .candidates
                            .iter()
                            .map(|candidate| candidate.listing.id.as_str()),
                    )
            })
    });
    level.truncate(BEAM_WIDTH);
}

pub fn plan_reverse(catalog: &Catalog, request: PlanRequest) -> Result<PlanResponse, PlannerError> {
    let target = catalog.skin(&request.target_skin_id).ok_or_else(|| {
        PlannerError::Validation(format!(
            "unknown target_skin_id: {}",
            request.target_skin_id
        ))
    })?;
    if request.stattrak && !target.stattrak_supported {
        return Err(PlannerError::Validation(format!(
            "{} has no StatTrak variant in the catalog",
            target.name
        )));
    }
    if target.rarity != Rarity::Covert {
        return Err(PlannerError::Validation(
            "this MVP reverse planner currently materializes normal 10-item paths to Covert targets".to_owned(),
        ));
    }
    let (acceptable_low, acceptable_high, target_center) =
        request
            .target
            .requested_bounds(request.delta, target.min_float, target.max_float)?;
    let target_adjusted = adjusted_float(target_center, target)?;
    let candidate_input_skins = catalog.input_skins_for_target(target);
    if candidate_input_skins.is_empty() {
        return Err(PlannerError::Validation(format!(
            "no {:?} inputs are known for {}",
            target.rarity.previous(),
            target.name
        )));
    }
    let owned = resolve_inputs(catalog, &request.owned_inputs, request.stattrak)?;
    let valid_input_ids: Vec<String> = candidate_input_skins
        .iter()
        .map(|skin| skin.id.clone())
        .collect();
    if owned
        .iter()
        .any(|input| !valid_input_ids.iter().any(|id| id == &input.skin.id))
    {
        return Err(PlannerError::Validation(format!(
            "owned items must be {:?} from {}",
            target.rarity.previous(),
            target.collection_name
        )));
    }
    if owned.len() > 10 {
        return Err(PlannerError::Validation(
            "a standard contract cannot contain more than 10 owned inputs".to_owned(),
        ));
    }
    let required = 10 - owned.len();
    // The limit is per input skin: a marketplace provider may deliberately
    // collect a broad pool for every valid trade-up path.  Applying this as a
    // global cap would silently discard all but the cheapest skin's candidates.
    let listing_limit = request.listing_limit.unwrap_or(200).clamp(1, 200);
    let raw_listings = catalog.fixture_listings_for_skins(&valid_input_ids, listing_limit);
    let candidates = raw_listings
        .iter()
        .filter_map(|listing| {
            let skin = catalog.skin(&listing.skin_id)?;
            if request.stattrak && !skin.stattrak_supported {
                return None;
            }
            Some(Candidate {
                listing,
                skin,
                adjusted: adjusted_float(listing.float_value, skin).ok()?,
            })
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() && required > 0 {
        return ideal_reverse_plan(
            catalog,
            &request,
            target,
            &candidate_input_skins,
            &owned,
            target_adjusted,
            acceptable_low,
            acceptable_high,
            target_center,
        );
    }
    if required > candidates.len() {
        return Err(PlannerError::Validation(format!(
            "only {} usable fixture listings are available for {} missing slots",
            candidates.len(),
            required
        )));
    }
    let (best, optimizer) = if candidates.len() <= 28 {
        let mut selected = Vec::with_capacity(required);
        let mut best = None;
        search_combinations(
            &candidates,
            &owned,
            required,
            target,
            acceptable_low,
            acceptable_high,
            target_center,
            &request.priority,
            0,
            &mut selected,
            &mut best,
        );
        (
            best,
            "exact combination enumeration over the available listing pool".to_owned(),
        )
    } else {
        (
            search_combinations_beam(
                &candidates,
                &owned,
                required,
                target,
                target_adjusted,
                acceptable_low,
                acceptable_high,
                target_center,
                &request.priority,
            ),
            format!(
                "bounded beam search over {} listings (beam width {})",
                candidates.len(),
                BEAM_WIDTH
            ),
        )
    };
    let Some(best) = best else {
        return Err(PlannerError::Validation(
            "no candidate combination could be selected".to_owned(),
        ));
    };

    let mut resolved_request = request
        .owned_inputs
        .iter()
        .map(|input| InputRequest {
            skin_id: input.skin_id.clone(),
            float_value: input.float_value,
            inspect_link: input.inspect_link.clone(),
        })
        .collect::<Vec<_>>();
    for candidate in &best.candidates {
        resolved_request.push(InputRequest {
            skin_id: candidate.skin.id.clone(),
            float_value: candidate.listing.float_value,
            inspect_link: None,
        });
    }
    let analysis = analyze_contract(
        catalog,
        AnalyzeRequest {
            contract_size: 10,
            stattrak: request.stattrak,
            inputs: resolved_request,
        },
    )?;
    let target_outcome = analysis
        .outcomes
        .iter()
        .find(|outcome| outcome.skin_id == target.id)
        .ok_or_else(|| {
            PlannerError::Catalog("target missing from its own contract outcomes".to_owned())
        })?;
    let target_probability = target_outcome.probability_percent.clone();
    let status = if best.within_target {
        match request.target {
            TargetConstraint::Exact { value } if best.predicted.to_bits() == value.to_bits() => {
                "exact"
            }
            _ => "within_tolerance",
        }
    } else {
        "closest"
    };
    let uses_steam_market = best
        .candidates
        .iter()
        .all(|candidate| candidate.listing.source == "steam_community_market");
    let message = match status {
        "exact" => "Найден bit-exact float32 результат для выбранной цели.".to_owned(),
        "within_tolerance" => {
            if uses_steam_market {
                "Найден набор активных лотов Steam Community Market в допустимом диапазоне."
                    .to_owned()
            } else {
                "Найден самый дешёвый набор доступных лотов в допустимом диапазоне.".to_owned()
            }
        }
        _ => "Набора в допустимом диапазоне нет; показан ближайший из доступных лотов.".to_owned(),
    };

    let owned_listings = owned
        .iter()
        .enumerate()
        .map(|(index, input)| PlannedListing {
            slot: index + 1,
            listing_id: None,
            source: "owned".to_owned(),
            skin_id: input.skin.id.clone(),
            skin_name: input.skin.name.clone(),
            float_value: Float32Value::new(input.float_value),
            adjusted_float: Float32Value::new(
                adjusted_float(input.float_value, input.skin).expect("validated"),
            ),
            price_cents: None,
            market_url: None,
            inspect_link: None,
            owned: true,
        });
    let fixture_listings = best
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| PlannedListing {
            slot: owned.len() + index + 1,
            listing_id: Some(candidate.listing.id.clone()),
            source: candidate.listing.source.clone(),
            skin_id: candidate.skin.id.clone(),
            skin_name: candidate.skin.name.clone(),
            float_value: Float32Value::new(candidate.listing.float_value),
            adjusted_float: Float32Value::new(candidate.adjusted),
            price_cents: Some(candidate.listing.price_cents),
            market_url: Some(candidate.listing.market_url.clone()),
            inspect_link: candidate.listing.inspect_link.clone(),
            owned: false,
        });

    Ok(PlanResponse {
        status: status.to_owned(),
        message,
        target_skin: target.clone(),
        target_adjusted: Float32Value::new(target_adjusted),
        acceptable_output_range: [
            Float32Value::new(acceptable_low),
            Float32Value::new(acceptable_high),
        ],
        contract_size: 10,
        target_probability,
        candidate_input_skins: candidate_input_skins.into_iter().cloned().collect(),
        selected_inputs: owned_listings.chain(fixture_listings).collect(),
        total_price_cents: best.total_price_cents,
        pricing_available: true,
        predicted_target_float: Float32Value::new(best.predicted),
        target_distance: Float32Value::new(best.distance),
        target_wear: target_outcome.wear.clone(),
        all_outcomes: analysis.outcomes,
        optimizer,
        warnings: [
            if uses_steam_market {
                "Steam Community Market listing содержит цену покупателя с комиссией, ссылку на Market, inspect link и exact raw float на момент чтения HTML; перед покупкой проверьте актуальный статус лота."
                    .to_owned()
            } else {
                "fixture означает, что это детерминированный тестовый лот, а не проверяемое объявление Steam Market.".to_owned()
            },
            "delta применяется как допустимое отклонение результата от цели; формула и все промежуточные операции — float32.".to_owned(),
        ]
        .into(),
    })
}

#[allow(clippy::too_many_arguments)]
fn ideal_reverse_plan(
    catalog: &Catalog,
    request: &PlanRequest,
    target: &Skin,
    candidate_input_skins: &[&Skin],
    owned: &[ResolvedInput<'_>],
    target_adjusted: f32,
    acceptable_low: f32,
    acceptable_high: f32,
    target_center: f32,
) -> Result<PlanResponse, PlannerError> {
    let required = 10 - owned.len();
    let input_skin = candidate_input_skins
        .iter()
        .copied()
        .find(|skin| !request.stattrak || skin.stattrak_supported)
        .ok_or_else(|| {
            PlannerError::Validation("no StatTrak-compatible inputs exist for this path".to_owned())
        })?;
    let owned_sum = owned.iter().fold(0.0_f32, |sum, input| {
        f32_add(
            sum,
            adjusted_float(input.float_value, input.skin).expect("validated owned item"),
        )
    });
    let ideal_adjusted = f32_div(
        f32_sub(f32_mul(target_adjusted, 10.0), owned_sum),
        required as f32,
    );
    let bounded_adjusted = ideal_adjusted.clamp(0.0, 1.0);
    let ideal_float = output_float(bounded_adjusted, input_skin);
    let mut resolved_inputs = request.owned_inputs.clone();
    resolved_inputs.extend((0..required).map(|_| InputRequest {
        skin_id: input_skin.id.clone(),
        float_value: ideal_float,
        inspect_link: None,
    }));
    let analysis = analyze_contract(
        catalog,
        AnalyzeRequest {
            contract_size: 10,
            stattrak: request.stattrak,
            inputs: resolved_inputs,
        },
    )?;
    let target_outcome = analysis
        .outcomes
        .iter()
        .find(|outcome| outcome.skin_id == target.id)
        .ok_or_else(|| {
            PlannerError::Catalog("target missing from its own contract outcomes".to_owned())
        })?;
    let predicted = target_outcome.predicted_float.value;
    let within_target = predicted >= acceptable_low && predicted <= acceptable_high;
    let status = if within_target {
        match request.target {
            TargetConstraint::Exact { value } if predicted.to_bits() == value.to_bits() => "exact",
            _ => "within_tolerance",
        }
    } else {
        "closest"
    };
    let owned_listings = owned
        .iter()
        .enumerate()
        .map(|(index, input)| PlannedListing {
            slot: index + 1,
            listing_id: None,
            source: "owned".to_owned(),
            skin_id: input.skin.id.clone(),
            skin_name: input.skin.name.clone(),
            float_value: Float32Value::new(input.float_value),
            adjusted_float: Float32Value::new(
                adjusted_float(input.float_value, input.skin).expect("validated"),
            ),
            price_cents: None,
            market_url: None,
            inspect_link: None,
            owned: true,
        });
    let generated_listings = (0..required).map(|index| PlannedListing {
        slot: owned.len() + index + 1,
        listing_id: None,
        source: "ideal_math".to_owned(),
        skin_id: input_skin.id.clone(),
        skin_name: input_skin.name.clone(),
        float_value: Float32Value::new(ideal_float),
        adjusted_float: Float32Value::new(bounded_adjusted),
        price_cents: None,
        market_url: None,
        inspect_link: None,
        owned: false,
    });

    Ok(PlanResponse {
        status: status.to_owned(),
        message: if within_target {
            "Построен идеальный математический контракт; реальные лоты для его покупки ещё не подключены.".to_owned()
        } else {
            "С текущими owned-предметами цель недостижима: показан ближайший математический контракт.".to_owned()
        },
        target_skin: target.clone(),
        target_adjusted: Float32Value::new(target_adjusted),
        acceptable_output_range: [
            Float32Value::new(acceptable_low),
            Float32Value::new(acceptable_high),
        ],
        contract_size: 10,
        target_probability: target_outcome.probability_percent.clone(),
        candidate_input_skins: candidate_input_skins.iter().map(|skin| (*skin).clone()).collect(),
        selected_inputs: owned_listings.chain(generated_listings).collect(),
        total_price_cents: 0,
        pricing_available: false,
        predicted_target_float: Float32Value::new(predicted),
        target_distance: Float32Value::new(f32_abs(f32_sub(predicted, target_center))),
        target_wear: target_outcome.wear.clone(),
        all_outcomes: analysis.outcomes,
        optimizer: "analytical float32 construction; no purchasable listing provider is configured".to_owned(),
        warnings: vec![
            "Слоты ideal_math задают требуемые exact float, а не существующие рыночные предметы.".to_owned(),
            "Подключите provider, который возвращает конкретный listing, inspect payload и exact float, чтобы оптимизировать цену и выдать ссылки на покупку.".to_owned(),
            "delta применяется к результату; все промежуточные операции выполняются как последовательные float32.".to_owned(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    #[test]
    fn normalizes_knight_and_calculates_dragon_lore_with_float32() {
        let catalog = Catalog::load_embedded().unwrap();
        let knight = catalog.skin("m4a1s-knight").unwrap();
        let dragon_lore = catalog.skin("awp-dragon-lore").unwrap();
        let adjusted = adjusted_float(0.01_f32, knight).unwrap();
        let average = average_adjusted(std::iter::repeat_n(adjusted, 10), 10);
        let output = output_float(average, dragon_lore);

        // 0.01f32 / 0.1f32 is one ULP below the decimal-looking 0.1f32.
        assert_eq!(adjusted.to_bits(), 0x3dcc_cccc);
        assert_eq!(output.to_bits(), 0.07_f32.to_bits());
        assert_eq!(wear_info(output).name, "Minimal Wear");
        assert!(wear_info(output).near_boundary);
    }

    #[test]
    fn partial_preview_exposes_possible_outcomes_and_an_honest_float_range() {
        let catalog = Catalog::load_embedded().unwrap();
        let preview = preview_partial_contract(
            &catalog,
            PartialAnalyzeRequest {
                stattrak: false,
                inputs: vec![PartialInputRequest {
                    skin_id: "m4a1s-knight".to_owned(),
                    float_value: None,
                }],
            },
        )
        .unwrap();

        assert_eq!(preview.selected_slots, 1);
        assert_eq!(preview.floats_specified, 0);
        assert_eq!(preview.remaining_float_slots, 10);
        let dragon_lore = preview
            .outcomes
            .iter()
            .find(|outcome| outcome.skin_id == "awp-dragon-lore")
            .unwrap();
        assert_eq!(dragon_lore.predicted_float_min.value, 0.0);
        assert_eq!(dragon_lore.predicted_float_max.value, 0.7);
        assert_eq!(dragon_lore.probability_min, 0.1);
        assert_eq!(dragon_lore.probability_max, 1.0);
    }

    #[test]
    fn floatjitsu_regression_uses_sequential_f32_sum() {
        let cap_skin = Skin {
            id: "fixture".into(),
            name: "fixture".into(),
            collection_id: "fixture".into(),
            collection_name: "fixture".into(),
            rarity: Rarity::Classified,
            min_float: 0.0,
            max_float: 0.8,
            stattrak_supported: false,
            def_index: None,
            paint_index: None,
        };
        let out_skin = Skin {
            max_float: 0.8,
            min_float: 0.06,
            ..cap_skin.clone()
        };
        let raw = f32::from_bits(0x3d62_d947);
        let adjusted = adjusted_float(raw, &cap_skin).unwrap();
        let average = average_adjusted(std::iter::repeat_n(adjusted, 10), 10);
        let output = output_float(average, &out_skin);

        assert_eq!(adjusted.to_bits(), 0x3d8d_c7cc);
        assert_eq!(average.to_bits(), 0x3d8d_c7cd);
        assert_eq!(output.to_bits(), 0x3de3_cc2c);
    }

    #[test]
    fn calculates_one_hundred_percent_dragon_lore_path() {
        let catalog = Catalog::load_embedded().unwrap();
        let response = analyze_contract(
            &catalog,
            AnalyzeRequest {
                contract_size: 10,
                stattrak: false,
                inputs: (0..10)
                    .map(|_| InputRequest {
                        skin_id: "m4a1s-knight".into(),
                        float_value: 0.01,
                        inspect_link: None,
                    })
                    .collect(),
            },
        )
        .unwrap();

        assert_eq!(response.outcomes.len(), 1);
        assert_eq!(response.outcomes[0].skin_id, "awp-dragon-lore");
        assert_eq!(response.outcomes[0].probability, 1.0);
    }

    #[test]
    fn rejects_mixed_rarity() {
        let catalog = Catalog::load_embedded().unwrap();
        let error = analyze_contract(
            &catalog,
            AnalyzeRequest {
                contract_size: 10,
                stattrak: false,
                inputs: (0..10)
                    .map(|slot| InputRequest {
                        skin_id: if slot == 0 {
                            "awp-dragon-lore"
                        } else {
                            "m4a1s-knight"
                        }
                        .into(),
                        float_value: 0.01,
                        inspect_link: None,
                    })
                    .collect(),
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("outside") || error.to_string().contains("same rarity"));
    }

    #[test]
    fn reverse_plan_keeps_owned_slots_and_fills_the_rest() {
        let catalog = Catalog::load_embedded().unwrap();
        let plan = plan_reverse(
            &catalog,
            PlanRequest {
                target_skin_id: "awp-dragon-lore".into(),
                target: TargetConstraint::Exact { value: 0.15 },
                delta: 0.0001,
                stattrak: false,
                owned_inputs: (0..5)
                    .map(|_| InputRequest {
                        skin_id: "m4a1s-knight".into(),
                        float_value: 0.021_428_57,
                        inspect_link: None,
                    })
                    .collect(),
                priority: PlanningPriority::Cheapest,
                listing_limit: None,
            },
        )
        .unwrap();

        assert_eq!(plan.selected_inputs.len(), 10);
        assert_eq!(
            plan.selected_inputs
                .iter()
                .filter(|input| input.owned)
                .count(),
            5
        );
        assert_eq!(plan.status, "within_tolerance");
        assert_eq!(plan.target_probability, "100.00%");
    }

    #[test]
    fn full_catalog_without_listings_returns_an_ideal_math_contract() {
        let snapshot = br#"[
          {"id":"classified","name":"Input","min_float":0.0,"max_float":1.0,"rarity":{"id":"rarity_legendary_weapon"},"stattrak":true,"collections":[{"id":"collection-test","name":"Test"}]},
          {"id":"covert","name":"Target","min_float":0.06,"max_float":0.8,"rarity":{"id":"rarity_ancient_weapon"},"stattrak":true,"collections":[{"id":"collection-test","name":"Test"}]}
        ]"#;
        let catalog = Catalog::from_upstream_snapshot(snapshot, "test", "now".to_owned()).unwrap();
        let plan = plan_reverse(
            &catalog,
            PlanRequest {
                target_skin_id: "collection-test/covert".into(),
                target: TargetConstraint::Exact { value: 0.15 },
                delta: 0.0001,
                stattrak: false,
                owned_inputs: Vec::new(),
                priority: PlanningPriority::Cheapest,
                listing_limit: None,
            },
        )
        .unwrap();

        assert!(
            plan.selected_inputs
                .iter()
                .all(|input| input.source == "ideal_math")
        );
        assert!(!plan.pricing_available);
        assert_eq!(plan.status, "exact");
    }

    #[test]
    fn large_listing_pool_uses_combination_search_without_reusing_listings() {
        let catalog = Catalog::load_embedded().unwrap();
        let listings = (0..200)
            .map(|index| FixtureListing {
                id: format!("live-{index}"),
                skin_id: "m4a1s-knight".to_owned(),
                float_value: 0.021_25 + index as f32 * 0.000_01,
                price_cents: 10_000 + index,
                market_url: format!("https://example.test/item/{index}"),
                source: "steam_community_market".to_owned(),
                inspect_link: Some(format!("steam://inspect/{index}")),
            })
            .collect();
        let catalog = catalog.with_listings(listings);
        let plan = plan_reverse(
            &catalog,
            PlanRequest {
                target_skin_id: "awp-dragon-lore".to_owned(),
                target: TargetConstraint::Exact { value: 0.15 },
                delta: 0.0001,
                stattrak: false,
                owned_inputs: Vec::new(),
                priority: PlanningPriority::Cheapest,
                listing_limit: Some(200),
            },
        )
        .unwrap();
        let listing_ids = plan
            .selected_inputs
            .iter()
            .filter_map(|input| input.listing_id.as_ref())
            .collect::<std::collections::HashSet<_>>();

        assert!(plan.optimizer.starts_with("bounded beam search"));
        assert_eq!(plan.selected_inputs.len(), 10);
        assert_eq!(listing_ids.len(), 10);
        assert!(plan.pricing_available);
    }
}
