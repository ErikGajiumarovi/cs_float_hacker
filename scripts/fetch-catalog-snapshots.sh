#!/usr/bin/env bash
set -euo pipefail

# Downloads the pinned upstream sources used to prepare/review catalog.v1.json.
# It does not overwrite the hand-reviewed MVP subset. A future importer should
# join the files by (weapon_id, paint_index), preserve collection membership,
# and emit a new versioned application snapshot.

readonly BYMYKEL_SHA="0a2030006075f2e184806bc956223f1e8a42d436"
readonly BASE="https://raw.githubusercontent.com/ByMykel/CSGO-API/${BYMYKEL_SHA}/public/api/en"
readonly OUTPUT_DIR="${1:-data/upstream/${BYMYKEL_SHA}}"

mkdir -p "$OUTPUT_DIR"
curl --fail --location --retry 3 --silent --show-error \
  "${BASE}/skins.json" -o "${OUTPUT_DIR}/skins.json"
curl --fail --location --retry 3 --silent --show-error \
  "${BASE}/collections.json" -o "${OUTPUT_DIR}/collections.json"

printf 'Saved pinned ByMykel snapshot to %s\n' "$OUTPUT_DIR"
