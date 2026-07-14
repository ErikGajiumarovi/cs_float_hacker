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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureListing {
    pub id: String,
    pub skin_id: String,
    pub float_value: f32,
    pub price_cents: u64,
    pub market_url: String,
    pub source: String,
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
            "Вычисления выполняются на backend как последовательные IEEE-754 float32; bits сохранены в ответе.".to_owned(),
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
    pub budget_cents: Option<u64>,
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
    let candidate_budget_key = candidate.total_price_cents;
    let current_budget_key = current.total_price_cents;
    match priority {
        PlanningPriority::Cheapest => {
            if candidate.within_target && current.within_target {
                (candidate_budget_key, candidate.distance) < (current_budget_key, current.distance)
            } else {
                (candidate.distance, candidate_budget_key) < (current.distance, current_budget_key)
            }
        }
        PlanningPriority::Closest => {
            (candidate.distance, candidate_budget_key) < (current.distance, current_budget_key)
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
    budget_cents: Option<u64>,
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
        if budget_cents.is_some_and(|budget| total_price_cents > budget) {
            return;
        }
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
            budget_cents,
            priority,
            index + 1,
            selected,
            best,
        );
        selected.pop();
    }
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
    if required > candidates.len() {
        return Err(PlannerError::Validation(format!(
            "only {} usable fixture listings are available for {} missing slots",
            candidates.len(),
            required
        )));
    }
    if candidates.len() > 28 {
        return Err(PlannerError::Validation(
            "the exact MVP optimizer is deliberately capped at 28 fixture candidates; a live 200-listing optimizer is the next adapter".to_owned(),
        ));
    }

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
        request.budget_cents,
        &request.priority,
        0,
        &mut selected,
        &mut best,
    );
    let Some(best) = best else {
        return Err(PlannerError::Validation(
            "no candidate combination is within the supplied budget".to_owned(),
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
    let message = match status {
        "exact" => "Найден bit-exact float32 результат для выбранной цели.".to_owned(),
        "within_tolerance" => {
            "Найден самый дешёвый набор fixture-лотов в допустимом диапазоне.".to_owned()
        }
        _ => "Набора в допустимом диапазоне нет; показан ближайший из доступных fixture-лотов."
            .to_owned(),
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
        predicted_target_float: Float32Value::new(best.predicted),
        target_distance: Float32Value::new(best.distance),
        target_wear: target_outcome.wear.clone(),
        all_outcomes: analysis.outcomes,
        optimizer: "exact combination enumeration over the current fixture candidate pool".to_owned(),
        warnings: vec![
            "fixture означает, что это детерминированный тестовый лот, а не проверяемое объявление Steam Market.".to_owned(),
            "В production вместо fixture provider нужен auth/browser-backed market adapter и local inspect-link decoder.".to_owned(),
            "delta применяется как допустимое отклонение результата от цели; формула и все промежуточные операции — float32.".to_owned(),
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
                budget_cents: None,
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
}
