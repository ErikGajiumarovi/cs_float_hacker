use serde::{Deserialize, Serialize};

use crate::domain::{FixtureListing, Rarity, Skin};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceMetadata {
    pub name: String,
    pub url: String,
    pub license: String,
    pub retrieved_at: String,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawCatalog {
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
    pub fixture_listing_count: usize,
    pub limitations: Vec<&'static str>,
}

impl Catalog {
    pub fn load_embedded() -> Result<Self, serde_json::Error> {
        let raw: RawCatalog = serde_json::from_str(include_str!("../data/catalog.v1.json"))?;
        Ok(Self {
            schema_version: raw.schema_version,
            source: raw.source,
            skins: raw.skins,
            listings: raw.fixture_listings,
        })
    }

    pub fn public_response(&self) -> CatalogResponse {
        CatalogResponse {
            schema_version: self.schema_version.clone(),
            source: self.source.clone(),
            skins: self.skins.clone(),
            fixture_listing_count: self.listings.len(),
            limitations: vec![
                "Каталог — проверенный небольшой snapshot для MVP, а не полный live-каталог CS2.",
                "Лоты помечены fixture: это не активные Steam Market listings и не должны использоваться для покупки.",
                "Пул 5 Covert → knife/gloves требует отдельного источника unusual/loot mappings и в этом snapshot не материализован.",
            ],
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
        limit: usize,
    ) -> Vec<&FixtureListing> {
        let mut listings: Vec<_> = self
            .listings
            .iter()
            .filter(|listing| skin_ids.iter().any(|id| id == &listing.skin_id))
            .collect();
        listings.sort_by_key(|listing| (listing.price_cents, listing.id.as_str()));
        listings.truncate(limit);
        listings
    }
}
