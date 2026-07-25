# Floatcraft — CS2 reverse trade-up planner MVP

Работающий вертикальный срез спецификации из `notes.md`: пользователь выбирает
Covert-цель и float, добавляет уже имеющиеся точные предметы, а backend на Rust
использует полный versioned catalog, строит математический reverse-контракт,
рассчитывает все исходы и проверяет итоговый контракт на `float32`.

## Быстрый запуск

Нужны Rust 1.96+ и Node 22+.

```bash
# терминал 1
cargo run

# терминал 2
cd frontend
npm install
npm run dev
```

Откройте <http://localhost:5173>. API доступен на
<http://localhost:8080/api/health>.

Или через Docker:

```bash
docker compose up --build
```

## Уже реализовано

- Rust/Axum API: `GET /api/catalog`, `POST /api/analyze`, `POST /api/plan`.
- Market API: `GET /api/market-status`; Steam Community Market provider читает
  public SSR HTML, получает active listings с ценой покупателя, exact float и
  inspect link, кэширует запросы и ограничивает их частоту.
- Последовательная IEEE-754 `f32` математика на каждом промежуточном шаге,
  decimal display и `u32` bit pattern.
- Full-catalog importer: после sync API атомарно переключается с embedded
  fallback на versioned multi-collection runtime catalog.
- Normal 10-item contract: валидация count/rarity/collection/StatTrak,
  probabilities по коллекциям, все outcomes, wear и предупреждение у границ.
- Reverse path к Covert target: exact / maximum / range, `δ`, budget,
  price-vs-float priority и фиксированные owned slots. Без provider лотов
  выдаётся честный `ideal_math` контракт без выдуманной цены или ссылки.
- Точный перебор сочетаний для доступного fixture-кандидатного пула, а не
  подбор одного ближайшего лота на каждый слот по отдельности.
- Реальные caps/collection edges для Knight → Dragon Lore (100%) и Chroma
  (два возможных Covert outcome), плюс React/TypeScript UI.
- Evidence-backed collector в UI: 10 входов → preview фактического output на
  уровне `float32` bits → явное сохранение записи и progress к 20 регрессиям.
- Детерминированный offline test suite для `f32`-математики, importer, Steam
  SSR parser/pagination/cache, regression storage и collector UI. Live Steam
  оставлен отдельным smoke-test, а не частью обычного тестового запуска.
- Постоянный background sync: при старте и затем раз в 24 часа backend берёт
  актуальные pinned commits ByMykel + SteamTracking, хранит полный snapshot в
  Docker volume `./runtime/`, materialize-ит `planner-catalog.json` и
  переключает работающий API на проверенную версию.

## Фоновая синхронизация и реальные регрессии

После `docker compose up` приложение автоматически скачивает полный каталог
ByMykel (`skins.json` и `collections.json`), фиксирует SHA каждого источника и
сохраняет их в `runtime/catalog/<commit>/`. Затем оно извлекает `paint_kits`
из Valve game data, формирует отчёт `verification.json` и строит активный
runtime-каталог. Для skin из нескольких коллекций создаётся отдельный stable
id `collection/skin`, потому что membership является частью trade-up path.

Статус доступен без UI:

```bash
curl http://localhost:8080/api/sync-status | jq
```

Ручной запуск sync для разработки:

```bash
curl -X POST http://localhost:8080/api/sync-now | jq
```

Частота по умолчанию — 24 часа. Её можно изменить без пересборки:

```bash
CATALOG_SYNC_INTERVAL_SECS=3600 docker compose up
```

## Реальные лоты Steam Community Market

По умолчанию provider включён и получает до 200 active Steam Market listings
**на каждый допустимый input skin**:

```bash
docker compose up --build
```

Для полностью офлайн-математического режима задайте
`MARKET_PROVIDER=disabled`; тогда план вернёт `ideal_math` без выдуманной цены.

Steam key, session cookie и CSFloat API не используются. Provider запрашивает
публичную HTML-страницу Market в USD, извлекает embedded SSR data только для
лотов с совпадающими названием/wear, raw float и inspect payload. Цена — сумма
`unPrice + unFee`, то есть сумма, которую видит покупатель, а не доход
продавца. Десять SSR-страниц распределяются между допустимыми wear, поэтому
лимит — до 200 кандидатов на **один** skin; затем optimiser рассматривает пул
каждого допустимого skin, а не только 200 самых дешёвых лотов всего контракта.

`MARKET_CACHE_TTL_SECS` (по умолчанию 60) и
`STEAM_MARKET_MIN_REQUEST_INTERVAL_MS` (по умолчанию 1000) управляют
cache/rate limit. `STEAM_MARKET_BASE_URL` нужен только для тестового/зеркального
источника; по умолчанию это `https://steamcommunity.com/market/listings/730/`.
Steam не документирует SSR-разметку как стабильный API, поэтому при изменении
страницы provider останавливает расчёт с ошибкой upstream, а не подставляет
сомнительные float или цену.

### Набор фактических контрактов

Игра и публичные inspect APIs не публикуют историю самого контракта — они
раскрывают данные **одного** предмета, но не связывают 10 входов с реально
полученным output. Поэтому приложение не заполняет regression suite
придуманными записями. В UI есть форма «Добавить фактический контракт»: она
сначала вызывает `POST /api/analyze`, показывает predicted и actual `float32`
bits и только затем разрешает сохранить результат. API остаётся доступен для
автоматизированного сбора:

```bash
curl -X POST http://localhost:8080/api/regressions/contracts \
  -H 'content-type: application/json' \
  --data @actual-contract.json

curl http://localhost:8080/api/regressions/status | jq
```

`actual-contract.json` содержит 10 `{ "skin_id", "float_value" }`,
`actual_output_skin_id`, `actual_output_float` и обязательный `evidence_url`:
публичную ссылку на capture/video/профиль или другой сохранённый материал,
который показывает состав контракта и его результат. Inspect link одного
предмета полезен как дополнение, но сам по себе не доказывает все десять
входов. Запись сохраняется в `runtime/actual-contracts.json` вместе с
predicted и actual float32 bits, а также версией и URL каталога, по которым
сделан расчёт. ID записи детерминированно строится из всех input float bits,
output и evidence URL, поэтому повторная отправка того же доказательства
отклоняется, а не создаёт дубликат. Suite готов только после 20
evidence-backed записей с точным float32 совпадением. Статус отдельно отдаёт
`evidence_backed_records`, `evidence_backed_exact_matches`, `mismatches` и
`records_missing_evidence`. Collector валидирует paths активного versioned
runtime-каталога и не создаёт мнимые «фактические» крафты сам.

Smoke-test live Steam provider после запуска API:

```bash
./scripts/smoke-steam-market.sh
```

Скрипт выбирает поддерживаемую Covert-цель из активного каталога, запрашивает
план и завершается с ошибкой, если Steam не вернул десять конкретных лотов с
ценой, Market URL и inspect link. Это проверка интеграции с текущей SSR-разметкой,
а не доказательство результата совершённого контракта.

Проверки:

```bash
cargo test
cd frontend && npm test && npm run build
```

Rust-suite использует фиксированные catalog/Valve/Steam SSR fixtures и не
открывает локальный TCP-порт. UI-suite подменяет API детерминированными
ответами и проверяет путь collector: 10 inputs → float32 preview → сохранение
→ обновлённый progress. Live Steam smoke-test остаётся отдельной внешней
проверкой, потому что SSR-разметка Valve не является стабильным API.

На текущем состоянии `cargo test` содержит 18 offline Rust-тестов, а
`npm test` — UI workflow-тест collector. Количество тестов может расти, но
обычный test path не должен обращаться к Steam, GitHub или реальному TCP.

## Пример API

```bash
curl http://localhost:8080/api/plan \
  -H 'content-type: application/json' \
  --data '{
    "target_skin_id": "awp-dragon-lore",
    "target": { "mode": "exact", "value": 0.15 },
    "delta": 0.0001,
    "priority": "cheapest",
    "owned_inputs": []
  }'
```

Для partial craft добавьте, например, пять `m4a1s-knight` в `owned_inputs`:

```json
{ "skin_id": "m4a1s-knight", "float_value": 0.02142857 }
```

API вернёт 10 слотов, из которых 5 будут `source: "owned"`, target probability,
output float и all outcomes. Если доступен provider лотов, добавятся цена и
purchase URL; без него остальные слоты честно помечены `source: "ideal_math"`.

## Математика и данные

`raw` float между разными skins усреднять нельзя. Сначала каждый raw float
нормализуется caps конкретного skin:

```text
adjusted_i = f32((float_i - min_i) / (max_i - min_i))
avg         = f32(sequential_sum(adjusted_i) / n)
out         = f32(out_min + avg * (out_max - out_min))
```

Таким образом «absolute / normalized float» — не отдельная таблица, а
производное от versioned catalog `min/max`. Подробные источники, pinned commit,
ограничения ключей и способ обновления описаны в [data/SOURCES.md](data/SOURCES.md).

Каталог MVP основан на MIT [ByMykel/CSGO-API](https://github.com/ByMykel/CSGO-API).
Caps стоит периодически сверять с
[SteamTracking/GameTracking-CS2](https://github.com/SteamTracking/GameTracking-CS2)
(`wear_remap_min/max`). [FloatJitsu](https://floatjitsu.com/) используется
только как внешний manual oracle для QA: у него нет стабильного публичного
data/API контракта.

## Честные границы MVP

- `fixture` listings — **не** активные Steam Market лоты; URL ведёт на поиск
  Steam, а не на конкретный asset. В полном каталоге без listing provider
  planner показывает `ideal_math`, поэтому не обещает покупки и не рисует
  фиктивную цену.
- Steam Community Market provider использует текущую публичную SSR-разметку,
  а не документированный API. Valve может изменить её, ввести rate limit или
  показать другой набор лотов без предупреждения; кэш и ограничение частоты
  снижают нагрузку, но не отменяют эту операционную зависимость.
- Normal Steam flow проверен live в Docker. Для анонимной SSR group-страницы
  Valve пока не подтверждён отдельный фильтр `StatTrak™`: provider принимает
  только лоты с точным StatTrak market hash и поэтому безопасно отбрасывает
  normal-лоты, но production-поддержка StatTrak требует отдельного regression
  smoke-test после каждого изменения Steam Market.
- Для Market-лотов raw float берётся непосредственно из asset property `2`, а
  inspect link собирается из `market_actions` и asset property `6`. Для
  собственных предметов UI по-прежнему принимает exact float вручную: adapter,
  который валидирует произвольный pasted inspect link, ещё не реализован.
- 5 Covert → knife/gloves признан на уровне валидации, но не материализован:
  для него нужен отдельный versioned unusual/loot outcome map и реальные
  регрессионные крафты. Никаких выдуманных knife outcomes не показывается.
- EV, комиссии, ликвидность, Souvenir и подтверждённое влияние порядка слотов
  оставлены на следующий этап.
