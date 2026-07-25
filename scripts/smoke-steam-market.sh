#!/usr/bin/env bash
set -euo pipefail

# Verifies the live boundary with Steam's undocumented SSR listing format.
# Start the API with MARKET_PROVIDER=steam before running this script.
readonly API_URL="${API_URL:-http://localhost:8080}"

market_status="$(curl --fail --silent --show-error "${API_URL}/api/market-status")"
jq --exit-status '
  .enabled == true and .provider == "steam_community_market"
' <<<"${market_status}" >/dev/null || {
  echo "Steam Market provider is not enabled at ${API_URL}" >&2
  exit 1
}

catalog="$(curl --fail --silent --show-error "${API_URL}/api/catalog")"
target_skin_id="$(jq --raw-output '
  .skins as $skins
  | [
      $skins[]
      | select(.rarity == "covert")
      | . as $target
      | select($skins | any(
          .collection_id == $target.collection_id and .rarity == "classified"
        ))
      | .id
    ]
  | first // empty
' <<<"${catalog}")"

if [[ -z "${target_skin_id}" ]]; then
  echo "Active catalog has no normal Classified-to-Covert path to smoke-test" >&2
  exit 1
fi

request="$(jq --null-input --arg target_skin_id "${target_skin_id}" '{
  target_skin_id: $target_skin_id,
  target: {mode: "maximum", value: 0.99},
  delta: 0.01,
  stattrak: false,
  owned_inputs: [],
  priority: "cheapest",
  listing_limit: 200
}')"
plan="$(curl --fail --silent --show-error \
  --header 'content-type: application/json' \
  --data "${request}" \
  "${API_URL}/api/plan")"

jq --exit-status '
  .pricing_available == true
  and (.selected_inputs | length == 10)
  and (.selected_inputs | all(
    .[];
    .source == "steam_community_market"
    and (.listing_id | type == "string")
    and (.price_cents | type == "number")
    and (.market_url | type == "string")
    and (.inspect_link | type == "string")
  ))
' <<<"${plan}" >/dev/null || {
  echo "Steam smoke-test failed: provider did not return ten verifiable listings" >&2
  jq '{status, message, pricing_available, warnings}' <<<"${plan}" >&2
  exit 1
}

jq --raw-output '
  "Steam smoke-test passed for \(.target_skin.name): \(.selected_inputs | length) listings, $\(.total_price_cents / 100)"
' <<<"${plan}"
