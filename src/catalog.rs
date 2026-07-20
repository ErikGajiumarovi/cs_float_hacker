use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::domain::{FixtureListing, Rarity, Skin};

const RUNTIME_CATALOG_FILE: &str = "planner-catalog.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceMetadata {
    pub name: String,
    pub url: String,
    pub license: String,
    pub retrieved_at: String,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistedCatalog {
    schema_version: String,
    source: SourceMetadata,
    skins: Vec<Skin>,
    #[serde(default)]
    listings: Vec<FixtureListing>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawFixtureCatalog {
    schema_version: String,
    source: SourceMetadata,
    skins: Vec<Skin>,
    fixture_listings: Vec<FixtureListing>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub schema_version: String,
    pub source: SourceMetadata,
    skins: Vec<Skin>,
    listings: Vec<FixtureListing>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogResponse {
    pub schema_version: String,
    pub source: SourceMetadata,
    pub skins: Vec<Skin>,
    pub listing_count: usize,
    pub limitations: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UpstreamSkin {
    id: String,
    name: String,
    min_float: Option<f32>,
    max_float: Option<f32>,
    rarity: UpstreamRarity,
    #[serde(default)]
    stattrak: bool,
    paint_index: Option<String>,
    weapon: Option<UpstreamWeapon>,
    #[serde(default)]
    collections: Vec<UpstreamCollection>,
}

#[derive(Debug, Deserialize)]
struct UpstreamWeapon {
    weapon_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct UpstreamRarity {
    id: String,
}

#[derive(Debug, Deserialize)]
struct UpstreamCollection {
    id: String,
    name: String,
}

impl Catalog {
    pub fn load_embedded() -> Result<Self, serde_json::Error> {
        let raw: RawFixtureCatalog = serde_json::from_str(include_str!("../data/catalog.v1.json"))?;
        Ok(Self {
            schema_version: raw.schema_version,
            source: raw.source,
            skins: raw.skins,
            listings: raw.fixture_listings,
        })
    }

    /// Uses the last successfully imported, versioned catalog when available.
    /// A corrupt or interrupted runtime file never prevents the service from
    /// starting: the reviewed embedded fixture remains a safe fallback.
    pub fn load_runtime_or_embedded(data_dir: &Path) -> Self {
        let path = data_dir.join(RUNTIME_CATALOG_FILE);
        std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PersistedCatalog>(&bytes).ok())
            .and_then(|raw| Self::from_persisted(raw).ok())
            .unwrap_or_else(|| Self::load_embedded().expect("embedded catalog must be valid JSON"))
    }

    pub fn from_upstream_snapshot(
        skins_json: &[u8],
        commit: &str,
        retrieved_at: String,
    ) -> Result<Self, String> {
        let upstream: Vec<UpstreamSkin> = serde_json::from_slice(skins_json)
            .map_err(|error| format!("cannot parse upstream skins for planner import: {error}"))?;
        let mut skins = Vec::new();

        // A skin can belong to several collections. The collection membership is
        // part of a trade-up path, so materialize one stable path id per pair.
        for skin in upstream {
            let Some(rarity) = rarity_from_upstream(&skin.rarity.id) else {
                continue;
            };
            let (Some(min_float), Some(max_float)) = (skin.min_float, skin.max_float) else {
                continue;
            };
            if !min_float.is_finite() || !max_float.is_finite() || min_float >= max_float {
                continue;
            }
            for collection in skin.collections {
                skins.push(Skin {
                    id: format!("{}/{}", collection.id, skin.id),
                    name: skin.name.clone(),
                    collection_id: collection.id,
                    collection_name: collection.name,
                    rarity,
                    min_float,
                    max_float,
                    stattrak_supported: skin.stattrak,
                    def_index: skin.weapon.as_ref().and_then(|weapon| weapon.weapon_id),
                    paint_index: skin
                        .paint_index
                        .as_deref()
                        .and_then(|paint_index| paint_index.parse().ok()),
                });
            }
        }
        skins.sort_by(|left, right| {
            (&left.collection_name, &left.name, &left.id).cmp(&(
                &right.collection_name,
                &right.name,
                &right.id,
            ))
        });
        if skins.is_empty() {
            return Err("upstream snapshot produced no tradable collection skins".to_owned());
        }

        Self::from_persisted(PersistedCatalog {
            schema_version: format!("bymykel-{commit}"),
            source: SourceMetadata {
                name: "ByMykel/CSGO-API".to_owned(),
                url: format!("https://github.com/ByMykel/CSGO-API/tree/{commit}"),
                license: "MIT".to_owned(),
                retrieved_at,
                note: "Full versioned runtime catalog imported from the pinned ByMykel snapshot. A skin is materialized once for every collection trade-up path it belongs to.".to_owned(),
            },
            skins,
            listings: Vec::new(),
        })
    }

    pub fn store_runtime(&self, data_dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(&PersistedCatalog {
            schema_version: self.schema_version.clone(),
            source: self.source.clone(),
            skins: self.skins.clone(),
            listings: self.listings.clone(),
        })
        .map_err(|error| error.to_string())?;
        let temporary = data_dir.join(format!("{RUNTIME_CATALOG_FILE}.new"));
        std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
        std::fs::rename(temporary, data_dir.join(RUNTIME_CATALOG_FILE))
            .map_err(|error| error.to_string())
    }

    fn from_persisted(raw: PersistedCatalog) -> Result<Self, String> {
        if raw.skins.is_empty() {
            return Err("catalog has no skins".to_owned());
        }
        let mut ids = BTreeMap::new();
        for skin in &raw.skins {
            if !skin.min_float.is_finite()
                || !skin.max_float.is_finite()
                || skin.min_float >= skin.max_float
            {
                return Err(format!("{} has invalid float caps", skin.id));
            }
            if ids.insert(&skin.id, ()).is_some() {
                return Err(format!("catalog has duplicate skin id {}", skin.id));
            }
        }
        Ok(Self {
            schema_version: raw.schema_version,
            source: raw.source,
            skins: raw.skins,
            listings: raw.listings,
        })
    }

    pub fn public_response(&self) -> CatalogResponse {
        let mut limitations = vec![
            "Лоты отображаются только когда подключён проверяемый provider с exact float; Steam Community Market provider читает публичную SSR-разметку и может быть временно ограничен Steam.".to_owned(),
            "Пул 5 Covert → knife/gloves требует отдельного versioned unusual/loot mapping и пока не материализован.".to_owned(),
        ];
        if !self.listings.is_empty()
            && self
                .listings
                .iter()
                .all(|listing| listing.source == "fixture")
        {
            limitations.insert(0, "Лоты помечены fixture: это детерминированные тестовые данные, не активные объявления Steam Market.".to_owned());
        }
        CatalogResponse {
            schema_version: self.schema_version.clone(),
            source: self.source.clone(),
            skins: self.skins.clone(),
            listing_count: self.listings.len(),
            limitations,
        }
    }

    pub fn skin(&self, id: &str) -> Option<&Skin> {
        self.skins.iter().find(|skin| skin.id == id)
    }

    pub fn outcomes_for_collection(&self, collection_id: &str, rarity: Rarity) -> Vec<&Skin> {
        self.skins
            .iter()
            .filter(|skin| skin.collection_id == collection_id && skin.rarity == rarity)
            .collect()
    }

    pub fn input_skins_for_target(&self, target: &Skin) -> Vec<&Skin> {
        let Some(input_rarity) = target.rarity.previous() else {
            return Vec::new();
        };
        self.skins
            .iter()
            .filter(|skin| {
                skin.collection_id == target.collection_id && skin.rarity == input_rarity
            })
            .collect()
    }

    pub fn fixture_listings_for_skins(
        &self,
        skin_ids: &[String],
        limit_per_skin: usize,
    ) -> Vec<&FixtureListing> {
        let mut listings = Vec::new();
        for skin_id in skin_ids {
            let mut skin_listings = self
                .listings
                .iter()
                .filter(|listing| listing.skin_id == *skin_id)
                .collect::<Vec<_>>();
            skin_listings.sort_by_key(|listing| (listing.price_cents, listing.id.as_str()));
            skin_listings.truncate(limit_per_skin);
            listings.extend(skin_listings);
        }
        listings.sort_by_key(|listing| (listing.price_cents, listing.id.as_str()));
        listings
    }

    pub fn with_listings(&self, listings: Vec<FixtureListing>) -> Self {
        Self {
            schema_version: self.schema_version.clone(),
            source: self.source.clone(),
            skins: self.skins.clone(),
            listings,
        }
    }
}

fn rarity_from_upstream(id: &str) -> Option<Rarity> {
    match id {
        "rarity_common_weapon" => Some(Rarity::Consumer),
        "rarity_uncommon_weapon" => Some(Rarity::Industrial),
        "rarity_rare_weapon" => Some(Rarity::MilSpec),
        "rarity_mythical_weapon" => Some(Rarity::Restricted),
        "rarity_legendary_weapon" => Some(Rarity::Classified),
        "rarity_ancient_weapon" => Some(Rarity::Covert),
        "rarity_ancient" | "rarity_contraband_weapon" => Some(Rarity::Extraordinary),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importer_preserves_each_collection_path() {
        let json = br#"[
          {"id":"skin-a","name":"A","min_float":0.0,"max_float":1.0,"rarity":{"id":"rarity_legendary_weapon"},"stattrak":true,"collections":[{"id":"collection-one","name":"One"},{"id":"collection-two","name":"Two"}]},
          {"id":"skin-b","name":"B","min_float":0.1,"max_float":0.7,"rarity":{"id":"rarity_ancient_weapon"},"collections":[{"id":"collection-one","name":"One"}]}
        ]"#;
        let catalog =
            Catalog::from_upstream_snapshot(json, "abc", "2026-07-20".to_owned()).unwrap();
        assert_eq!(catalog.public_response().skins.len(), 3);
        assert!(catalog.skin("collection-one/skin-a").is_some());
        assert_eq!(
            catalog
                .input_skins_for_target(catalog.skin("collection-one/skin-b").unwrap())
                .len(),
            1
        );
    }

    #[test]
    fn listing_limit_is_applied_to_each_input_skin() {
        let json = br#"[
          {"id":"input-a","name":"A","min_float":0.0,"max_float":1.0,"rarity":{"id":"rarity_legendary_weapon"},"collections":[{"id":"collection-one","name":"One"}]},
          {"id":"input-b","name":"B","min_float":0.0,"max_float":1.0,"rarity":{"id":"rarity_legendary_weapon"},"collections":[{"id":"collection-two","name":"Two"}]}
        ]"#;
        let mut catalog =
            Catalog::from_upstream_snapshot(json, "abc", "2026-07-20".to_owned()).unwrap();
        catalog.listings = ["collection-one/input-a", "collection-two/input-b"]
            .into_iter()
            .flat_map(|skin_id| {
                (0..3).map(move |index| FixtureListing {
                    id: format!("{skin_id}-{index}"),
                    skin_id: skin_id.to_owned(),
                    float_value: 0.1,
                    price_cents: index,
                    market_url: "https://steamcommunity.com/market/".to_owned(),
                    source: "steam_community_market".to_owned(),
                    inspect_link: None,
                })
            })
            .collect();

        let ids = vec![
            "collection-one/input-a".to_owned(),
            "collection-two/input-b".to_owned(),
        ];
        let selected = catalog.fixture_listings_for_skins(&ids, 2);

        assert_eq!(selected.len(), 4);
        assert_eq!(
            selected
                .iter()
                .filter(|listing| listing.skin_id == "collection-one/input-a")
                .count(),
            2
        );
        assert_eq!(
            selected
                .iter()
                .filter(|listing| listing.skin_id == "collection-two/input-b")
                .count(),
            2
        );
    }
}
