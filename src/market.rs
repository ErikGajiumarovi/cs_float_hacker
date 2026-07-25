use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{sync::Mutex, time::sleep};

use crate::{
    catalog::Catalog,
    domain::{FixtureListing, PlanRequest, PlannerError, Skin},
};

const DEFAULT_STEAM_MARKET_BASE: &str = "https://steamcommunity.com/market/listings/730/";
const DEFAULT_CACHE_TTL_SECONDS: u64 = 60;
const DEFAULT_MIN_REQUEST_INTERVAL_MS: u64 = 1_000;
const MAX_CANDIDATES_PER_SKIN: usize = 200;
const PAGE_SIZE: usize = 20;
const MAX_PAGES_PER_SKIN: usize = MAX_CANDIDATES_PER_SKIN / PAGE_SIZE;
const MAX_SKINS_PER_LIVE_PLAN: usize = 5;
const STEAM_USD_CURRENCY: u8 = 1;

const WEAR_VARIANTS: [(&str, f32, f32); 5] = [
    ("Factory New", 0.0, 0.07),
    ("Minimal Wear", 0.07, 0.15),
    ("Field-Tested", 0.15, 0.38),
    ("Well-Worn", 0.38, 0.45),
    ("Battle-Scarred", 0.45, 1.0),
];

#[derive(Debug, Clone, Serialize)]
pub struct MarketStatus {
    pub provider: String,
    pub enabled: bool,
    pub cache_ttl_seconds: u64,
    pub minimum_request_interval_ms: u64,
    pub cached_searches: usize,
    pub max_candidates_per_skin: usize,
}

#[derive(Clone)]
pub enum MarketProvider {
    Disabled,
    SteamCommunityMarket(Arc<SteamCommunityMarketProvider>),
}

impl MarketProvider {
    pub fn from_environment() -> Result<Self, String> {
        match std::env::var("MARKET_PROVIDER")
            .unwrap_or_else(|_| "steam".to_owned())
            .to_lowercase()
            .as_str()
        {
            "" | "disabled" | "none" => Ok(Self::Disabled),
            "steam" | "steam_market" | "steam_community_market" => Ok(Self::SteamCommunityMarket(
                Arc::new(SteamCommunityMarketProvider::from_environment()?),
            )),
            provider => Err(format!(
                "unsupported MARKET_PROVIDER={provider}; supported values: disabled, steam"
            )),
        }
    }

    pub async fn listings_for_plan(
        &self,
        catalog: &Catalog,
        request: &PlanRequest,
    ) -> Result<Option<Vec<FixtureListing>>, PlannerError> {
        match self {
            Self::Disabled => Ok(None),
            Self::SteamCommunityMarket(provider) => {
                provider.fetch_for_plan(catalog, request).await.map(Some)
            }
        }
    }

    pub async fn status(&self) -> MarketStatus {
        match self {
            Self::Disabled => MarketStatus {
                provider: "disabled".to_owned(),
                enabled: false,
                cache_ttl_seconds: 0,
                minimum_request_interval_ms: 0,
                cached_searches: 0,
                max_candidates_per_skin: MAX_CANDIDATES_PER_SKIN,
            },
            Self::SteamCommunityMarket(provider) => provider.status().await,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SearchKey {
    skin_id: String,
    stattrak: bool,
}

#[derive(Debug, Clone)]
struct CachedSearch {
    fetched_at: Instant,
    listings: Vec<FixtureListing>,
}

type PageFetchFuture = Pin<Box<dyn Future<Output = Result<SteamMarketPage, PlannerError>> + Send>>;

/// The only live-network boundary of the Market provider.  Keeping it behind
/// this small interface lets parser, pagination, cache and optimizer tests use
/// fixed HTML fixtures without opening a TCP port or relying on Steam.
trait MarketPageFetcher: Send + Sync {
    fn fetch_page(&self, url: Url) -> PageFetchFuture;
}

struct HttpMarketPageFetcher {
    client: Client,
}

impl MarketPageFetcher for HttpMarketPageFetcher {
    fn fetch_page(&self, url: Url) -> PageFetchFuture {
        let client = self.client.clone();
        Box::pin(async move {
            let response = client
                .get(url)
                .header("accept", "text/html,application/xhtml+xml")
                .header("user-agent", "Floatcraft/0.1 Steam Community Market reader")
                .send()
                .await
                .map_err(|error| {
                    PlannerError::Catalog(format!("Steam Market listing request failed: {error}"))
                })?;
            let status = response.status();
            if !status.is_success() {
                return Err(PlannerError::Catalog(format!(
                    "Steam Market listing request returned HTTP {status}"
                )));
            }
            let html = response.text().await.map_err(|error| {
                PlannerError::Catalog(format!(
                    "Steam Market listing response could not be read: {error}"
                ))
            })?;
            parse_steam_market_page(&html).map_err(|error| {
                PlannerError::Catalog(format!("Steam Market listing HTML is invalid: {error}"))
            })
        })
    }
}

/// Reads the public server-rendered Steam Community Market page.  The modern
/// Market page embeds the current listings in `window.SSR.renderContext`; it
/// includes the asset's raw float and the property used by its inspect link.
/// No Steam account, session cookie or third-party marketplace is used.
pub struct SteamCommunityMarketProvider {
    page_fetcher: Arc<dyn MarketPageFetcher>,
    market_listing_base: Url,
    cache_ttl: Duration,
    minimum_request_interval: Duration,
    cache: Mutex<HashMap<SearchKey, CachedSearch>>,
    last_request_at: Mutex<Option<Instant>>,
}

impl SteamCommunityMarketProvider {
    fn from_environment() -> Result<Self, String> {
        let market_listing_base: Url = std::env::var("STEAM_MARKET_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_STEAM_MARKET_BASE.to_owned())
            .parse()
            .map_err(|error| format!("STEAM_MARKET_BASE_URL must be a valid URL: {error}"))?;
        if !market_listing_base.path().ends_with('/') {
            return Err("STEAM_MARKET_BASE_URL must end with /".to_owned());
        }
        let cache_ttl = Duration::from_secs(env_u64(
            "MARKET_CACHE_TTL_SECS",
            DEFAULT_CACHE_TTL_SECONDS,
            1,
            3_600,
        ));
        let minimum_request_interval = Duration::from_millis(env_u64(
            "STEAM_MARKET_MIN_REQUEST_INTERVAL_MS",
            DEFAULT_MIN_REQUEST_INTERVAL_MS,
            100,
            60_000,
        ));
        let client = Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|error| format!("cannot create Steam Market client: {error}"))?;
        Ok(Self {
            page_fetcher: Arc::new(HttpMarketPageFetcher { client }),
            market_listing_base,
            cache_ttl,
            minimum_request_interval,
            cache: Mutex::new(HashMap::new()),
            last_request_at: Mutex::new(None),
        })
    }

    #[cfg(test)]
    fn for_test(market_listing_base: Url, page_fetcher: Arc<dyn MarketPageFetcher>) -> Self {
        Self {
            page_fetcher,
            market_listing_base,
            cache_ttl: Duration::from_secs(60),
            minimum_request_interval: Duration::ZERO,
            cache: Mutex::new(HashMap::new()),
            last_request_at: Mutex::new(None),
        }
    }

    async fn status(&self) -> MarketStatus {
        MarketStatus {
            provider: "steam_community_market".to_owned(),
            enabled: true,
            cache_ttl_seconds: self.cache_ttl.as_secs(),
            minimum_request_interval_ms: self.minimum_request_interval.as_millis() as u64,
            cached_searches: self.cache.lock().await.len(),
            max_candidates_per_skin: MAX_CANDIDATES_PER_SKIN,
        }
    }

    async fn fetch_for_plan(
        &self,
        catalog: &Catalog,
        request: &PlanRequest,
    ) -> Result<Vec<FixtureListing>, PlannerError> {
        let target = catalog.skin(&request.target_skin_id).ok_or_else(|| {
            PlannerError::Validation(format!(
                "unknown target_skin_id: {}",
                request.target_skin_id
            ))
        })?;
        let candidate_skins = catalog.input_skins_for_target(target);
        if candidate_skins.len() > MAX_SKINS_PER_LIVE_PLAN {
            return Err(PlannerError::Catalog(format!(
                "Steam Market request budget exceeded: this plan needs {} input-skin searches, while the desktop safety limit is {MAX_SKINS_PER_LIVE_PLAN}. Disable live Market to calculate ideal_math, or narrow the target/owned inputs.",
                candidate_skins.len()
            )));
        }
        let mut listings = Vec::new();
        for skin in candidate_skins {
            listings.extend(self.fetch_for_skin(skin, request.stattrak).await?);
        }
        listings.sort_by_key(|listing| (listing.price_cents, listing.id.clone()));
        listings.dedup_by(|left, right| left.id == right.id);
        Ok(listings)
    }

    async fn fetch_for_skin(
        &self,
        skin: &Skin,
        stattrak: bool,
    ) -> Result<Vec<FixtureListing>, PlannerError> {
        let key = SearchKey {
            skin_id: skin.id.clone(),
            stattrak,
        };
        if let Some(cached) = self.cache.lock().await.get(&key).cloned() {
            if cached.fetched_at.elapsed() < self.cache_ttl {
                return Ok(cached.listings);
            }
        }
        if stattrak && !skin.stattrak_supported {
            return Ok(Vec::new());
        }

        let wears = eligible_wears(skin);
        if wears.is_empty() {
            return Ok(Vec::new());
        }
        // Steam returns 20 entries per SSR response.  Split the fixed ten-page
        // budget across the valid exteriors, so wide-cap skins retain coverage
        // instead of collecting all candidates from just one exterior.
        let pages_per_wear = MAX_PAGES_PER_SKIN.div_ceil(wears.len());
        let mut listings = Vec::new();
        let mut seen = HashSet::new();
        for wear in wears {
            if listings.len() >= MAX_CANDIDATES_PER_SKIN {
                break;
            }
            let market_hash_name = market_hash_name(skin, wear, stattrak);
            let mut start = 0;
            for _ in 0..pages_per_wear {
                if listings.len() >= MAX_CANDIDATES_PER_SKIN {
                    break;
                }
                let page = self.fetch_page(&market_hash_name, start).await?;
                let received = page.listings.len();
                for listing in page.listings {
                    if listing.description.market_hash_name != market_hash_name {
                        continue;
                    }
                    let Some(raw_float) = listing.raw_float() else {
                        continue;
                    };
                    if !raw_float.is_finite()
                        || raw_float < skin.min_float
                        || raw_float > skin.max_float
                    {
                        continue;
                    }
                    let Some(inspect_link) = listing.inspect_link() else {
                        continue;
                    };
                    if seen.insert(listing.listing_id.clone()) {
                        listings.push(FixtureListing {
                            id: listing.listing_id.clone(),
                            skin_id: skin.id.clone(),
                            float_value: raw_float,
                            price_cents: listing.buyer_price_cents(),
                            market_url: self.market_url(&market_hash_name, &listing.listing_id)?,
                            source: "steam_community_market".to_owned(),
                            inspect_link: Some(inspect_link),
                        });
                    }
                    if listings.len() == MAX_CANDIDATES_PER_SKIN {
                        break;
                    }
                }
                if !page.more || received == 0 {
                    break;
                }
                start += received;
            }
        }
        listings.sort_by_key(|listing| (listing.price_cents, listing.id.clone()));
        self.cache.lock().await.insert(
            key,
            CachedSearch {
                fetched_at: Instant::now(),
                listings: listings.clone(),
            },
        );
        Ok(listings)
    }

    fn listing_page_url(&self, market_hash_name: &str) -> Result<Url, PlannerError> {
        let mut url = self.market_listing_base.clone();
        url.path_segments_mut()
            .map_err(|_| {
                PlannerError::Catalog(
                    "Steam Market base URL cannot accept path segments".to_owned(),
                )
            })?
            .pop_if_empty()
            .push(market_hash_name);
        Ok(url)
    }

    fn market_url(&self, market_hash_name: &str, listing_id: &str) -> Result<String, PlannerError> {
        let mut url = self.listing_page_url(market_hash_name)?;
        // The Market page is the purchase surface.  `listing` lets the
        // browser-side Market UI retain the selected concrete listing when
        // Steam supports it; the normal page still works if it ignores it.
        url.query_pairs_mut()
            .append_pair("buy", "1")
            .append_pair("listing", listing_id);
        Ok(url.to_string())
    }

    async fn fetch_page(
        &self,
        market_hash_name: &str,
        start: usize,
    ) -> Result<SteamMarketPage, PlannerError> {
        self.wait_for_rate_limit().await;
        let mut url = self.listing_page_url(market_hash_name)?;
        url.query_pairs_mut()
            .append_pair("start", &start.to_string())
            .append_pair("currency", &STEAM_USD_CURRENCY.to_string())
            .append_pair("language", "english");
        self.page_fetcher.fetch_page(url).await
    }

    async fn wait_for_rate_limit(&self) {
        let mut last_request_at = self.last_request_at.lock().await;
        if let Some(last) = *last_request_at {
            let elapsed = last.elapsed();
            if elapsed < self.minimum_request_interval {
                sleep(self.minimum_request_interval - elapsed).await;
            }
        }
        *last_request_at = Some(Instant::now());
    }
}

fn eligible_wears(skin: &Skin) -> Vec<&'static str> {
    WEAR_VARIANTS
        .iter()
        .filter(|(_, lower, upper)| skin.min_float < *upper && skin.max_float > *lower)
        .map(|(name, _, _)| *name)
        .collect()
}

fn market_hash_name(skin: &Skin, wear: &str, stattrak: bool) -> String {
    let prefix = if stattrak { "StatTrak™ " } else { "" };
    format!("{prefix}{} ({wear})", skin.name)
}

fn env_u64(name: &str, default: u64, minimum: u64, maximum: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
        .clamp(minimum, maximum)
}

#[derive(Debug, Deserialize)]
struct SteamMarketPage {
    #[serde(default)]
    more: bool,
    #[serde(default)]
    listings: Vec<SteamListing>,
}

#[derive(Debug, Deserialize)]
struct SteamListing {
    #[serde(rename = "listingid")]
    listing_id: String,
    #[serde(rename = "unPrice", default)]
    price_cents: u64,
    #[serde(rename = "unFee", default)]
    fee_cents: u64,
    description: SteamDescription,
    asset: SteamAsset,
}

impl SteamListing {
    fn buyer_price_cents(&self) -> u64 {
        self.price_cents.saturating_add(self.fee_cents)
    }

    fn raw_float(&self) -> Option<f32> {
        self.asset
            .asset_properties
            .iter()
            .find(|property| property.property_id == 2)
            .and_then(|property| property.float_value)
            .map(|value| value as f32)
    }

    fn inspect_link(&self) -> Option<String> {
        let action = self
            .description
            .market_actions
            .iter()
            .find(|action| action.link.contains("csgo_econ_action_preview"))?;
        let value = self
            .asset
            .asset_properties
            .iter()
            .find(|property| property.property_id == 6)
            .and_then(|property| property.string_value.as_deref())?;
        Some(action.link.replace("%propid:6%", value))
    }
}

#[derive(Debug, Deserialize)]
struct SteamDescription {
    market_hash_name: String,
    #[serde(default)]
    market_actions: Vec<SteamMarketAction>,
}

#[derive(Debug, Deserialize)]
struct SteamMarketAction {
    link: String,
}

#[derive(Debug, Deserialize)]
struct SteamAsset {
    #[serde(default)]
    asset_properties: Vec<SteamAssetProperty>,
}

#[derive(Debug, Deserialize)]
struct SteamAssetProperty {
    #[serde(rename = "propertyid")]
    property_id: u32,
    #[serde(default)]
    float_value: Option<f64>,
    #[serde(default)]
    string_value: Option<String>,
}

fn parse_steam_market_page(html: &str) -> Result<SteamMarketPage, String> {
    const MARKER: &str = "window.SSR.renderContext=JSON.parse(";
    let start = html
        .find(MARKER)
        .ok_or_else(|| "SSR render context is absent".to_owned())?
        + MARKER.len();
    let literal = extract_json_string_literal(&html[start..])?;
    let mut context: Value = serde_json::from_str(literal)
        .map_err(|error| format!("cannot decode SSR string literal: {error}"))?;
    // The current Market HTML applies JSON.parse to a JSON string which itself
    // contains serialized context.  Accept either that form or a one-level
    // form, but cap recursion so malformed HTML cannot consume unbounded work.
    for _ in 0..2 {
        let Some(serialized) = context.as_str() else {
            break;
        };
        context = serde_json::from_str(serialized)
            .map_err(|error| format!("cannot decode nested SSR context: {error}"))?;
    }
    let query_data = context
        .get("queryData")
        .and_then(Value::as_str)
        .ok_or_else(|| "SSR context has no queryData".to_owned())?;
    let queries: Value = serde_json::from_str(query_data)
        .map_err(|error| format!("cannot decode Market queryData: {error}"))?;
    let page = queries
        .get("queries")
        .and_then(Value::as_array)
        .and_then(|queries| {
            queries.iter().find_map(|query| {
                query
                    .pointer("/state/data/pages/0")
                    .filter(|page| page.get("listings").is_some())
            })
        })
        .ok_or_else(|| "Market listings page is absent from SSR queryData".to_owned())?;
    serde_json::from_value(page.clone())
        .map_err(|error| format!("cannot decode Market listings page: {error}"))
}

fn extract_json_string_literal(input: &str) -> Result<&str, String> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'\"') {
        return Err("SSR JSON.parse does not start with a string literal".to_owned());
    }
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(1) {
        if escaped {
            escaped = false;
        } else if *byte == b'\\' {
            escaped = true;
        } else if *byte == b'\"' {
            return Ok(&input[..=index]);
        }
    }
    Err("SSR JSON string literal is not terminated".to_owned())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;
    use crate::domain::{PlanningPriority, TargetConstraint};

    struct FixturePageFetcher {
        pages: Arc<HashMap<usize, String>>,
        calls: Arc<AtomicUsize>,
    }

    impl FixturePageFetcher {
        fn new(pages: HashMap<usize, String>) -> Arc<Self> {
            Arc::new(Self {
                pages: Arc::new(pages),
                calls: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    impl MarketPageFetcher for FixturePageFetcher {
        fn fetch_page(&self, url: Url) -> PageFetchFuture {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let start = url
                .query_pairs()
                .find_map(|(key, value)| (key == "start").then(|| value.parse::<usize>().ok()))
                .flatten()
                .unwrap_or(0);
            let html = self.pages.get(&start).cloned();
            Box::pin(async move {
                let html = html.ok_or_else(|| {
                    PlannerError::Catalog(format!("fixture has no Market page at start={start}"))
                })?;
                parse_steam_market_page(&html).map_err(PlannerError::Catalog)
            })
        }
    }

    #[tokio::test]
    async fn steam_provider_parses_html_price_float_and_inspect_link_and_uses_cache() {
        let skin = test_skin(0.45, 1.0);
        let item = market_hash_name(&skin, "Battle-Scarred", false);
        let fetcher = FixturePageFetcher::new(HashMap::from([(
            0,
            steam_html(serde_json::json!({
              "more": false,
              "listings": [
                listing("good", &item, 0.55, 100, 15, "ABCDEF"),
                listing("wrong-float", &item, 0.1, 1, 0, "NOPE"),
                listing("wrong-name", "Other (Battle-Scarred)", 0.55, 1, 0, "OTHER")
              ]
            })),
        )]));
        let provider = SteamCommunityMarketProvider::for_test(
            "https://fixture.test/market/listings/730/".parse().unwrap(),
            fetcher.clone(),
        );

        let first = provider.fetch_for_skin(&skin, false).await.unwrap();
        let second = provider.fetch_for_skin(&skin, false).await.unwrap();

        assert_eq!(fetcher.calls.load(Ordering::Relaxed), 1);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, "good");
        assert_eq!(first[0].float_value.to_bits(), 0.55_f32.to_bits());
        assert_eq!(first[0].price_cents, 115);
        assert_eq!(
            first[0].inspect_link.as_deref(),
            Some("steam://run/730//+csgo_econ_action_preview%20ABCDEF")
        );
        assert!(first[0].market_url.contains("buy=1"));
        assert_eq!(second[0].id, "good");
    }

    #[test]
    fn parser_rejects_an_ssr_page_without_listing_context() {
        let error = parse_steam_market_page("<html><body>no SSR data</body></html>").unwrap_err();
        assert_eq!(error, "SSR render context is absent");
    }

    #[tokio::test]
    async fn steam_provider_collects_up_to_two_hundred_exact_candidates() {
        let item = market_hash_name(&test_skin(0.45, 1.0), "Battle-Scarred", false);
        let pages = (0..MAX_CANDIDATES_PER_SKIN)
            .step_by(PAGE_SIZE)
            .map(|start| {
                let entries = (start..start + PAGE_SIZE)
                    .map(|index| {
                        listing(
                            &format!("listing-{index}"),
                            &item,
                            0.5 + (index as f64 / 10_000.0),
                            index as u64,
                            0,
                            "PAYLOAD",
                        )
                    })
                    .collect::<Vec<_>>();
                (
                    start,
                    steam_html(serde_json::json!({
                      "more": start + PAGE_SIZE < MAX_CANDIDATES_PER_SKIN,
                      "listings": entries,
                    })),
                )
            })
            .collect();
        let fetcher = FixturePageFetcher::new(pages);
        let provider = SteamCommunityMarketProvider::for_test(
            "https://fixture.test/market/listings/730/".parse().unwrap(),
            fetcher.clone(),
        );

        let listings = provider
            .fetch_for_skin(&test_skin(0.45, 1.0), false)
            .await
            .unwrap();

        assert_eq!(fetcher.calls.load(Ordering::Relaxed), MAX_PAGES_PER_SKIN);
        assert_eq!(listings.len(), MAX_CANDIDATES_PER_SKIN);
        assert_eq!(listings.first().unwrap().id, "listing-0");
        assert_eq!(listings.last().unwrap().id, "listing-199");
    }

    #[tokio::test]
    async fn plan_fetches_only_eligible_input_skins_from_steam() {
        let catalog = Catalog::from_upstream_snapshot(
            br#"[
              {"id":"input","name":"Input","min_float":0.45,"max_float":1.0,"rarity":{"id":"rarity_legendary_weapon"},"stattrak":true,"collections":[{"id":"collection-test","name":"Test"}]},
              {"id":"target","name":"Target","min_float":0.0,"max_float":1.0,"rarity":{"id":"rarity_ancient_weapon"},"stattrak":true,"collections":[{"id":"collection-test","name":"Test"}]}
            ]"#,
            "test",
            "now".to_owned(),
        )
        .unwrap();
        let input = catalog.skin("collection-test/input").unwrap();
        let item = market_hash_name(input, "Battle-Scarred", false);
        let fetcher = FixturePageFetcher::new(HashMap::from([(
            0,
            steam_html(serde_json::json!({
              "more": false,
              "listings": [listing("good", &item, 0.55, 100, 15, "ABCDEF")]
            })),
        )]));
        let provider = SteamCommunityMarketProvider::for_test(
            "https://fixture.test/market/listings/730/".parse().unwrap(),
            fetcher,
        );
        let request = PlanRequest {
            target_skin_id: "collection-test/target".to_owned(),
            target: TargetConstraint::Exact { value: 0.5 },
            delta: 0.0001,
            stattrak: false,
            owned_inputs: Vec::new(),
            budget_cents: None,
            priority: PlanningPriority::Cheapest,
            listing_limit: Some(200),
        };

        let listings = provider.fetch_for_plan(&catalog, &request).await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].skin_id, "collection-test/input");
    }

    fn test_skin(min_float: f32, max_float: f32) -> Skin {
        Skin {
            id: "collection-test/input".to_owned(),
            name: "Input".to_owned(),
            collection_id: "collection-test".to_owned(),
            collection_name: "Test".to_owned(),
            rarity: crate::domain::Rarity::Classified,
            min_float,
            max_float,
            stattrak_supported: true,
            def_index: None,
            paint_index: None,
        }
    }

    fn listing(
        id: &str,
        market_hash_name: &str,
        float_value: f64,
        price: u64,
        fee: u64,
        inspect_payload: &str,
    ) -> Value {
        serde_json::json!({
          "listingid": id,
          "unPrice": price,
          "unFee": fee,
          "description": {
            "market_hash_name": market_hash_name,
            "market_actions": [{"link":"steam://run/730//+csgo_econ_action_preview%20%propid:6%"}]
          },
          "asset": {"asset_properties": [
            {"propertyid": 2, "float_value": float_value},
            {"propertyid": 6, "string_value": inspect_payload}
          ]}
        })
    }

    fn steam_html(page: Value) -> String {
        let query_data = serde_json::json!({
            "queries": [{"state": {"data": {"pages": [page]}}}]
        })
        .to_string();
        let context = serde_json::json!({"queryData": query_data}).to_string();
        let nested = serde_json::to_string(&context).unwrap();
        format!("<script>window.SSR.renderContext=JSON.parse({nested});</script>")
    }
}
