# Floatcraft — CS2 reverse trade-up planner MVP

Работающий вертикальный срез спецификации из `notes.md`: пользователь выбирает
Covert-цель и float, добавляет уже имеющиеся точные предметы, а backend на Rust
подбирает недостающие предметы из детерминированного fixture pool, рассчитывает
все исходы и проверяет итоговый контракт на `float32`.

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
- Последовательная IEEE-754 `f32` математика на каждом промежуточном шаге,
  decimal display и `u32` bit pattern.
- Normal 10-item contract: валидация count/rarity/collection/StatTrak,
  probabilities по коллекциям, все outcomes, wear и предупреждение у границ.
- Reverse path к Covert target: exact / maximum / range, `δ`, budget,
  price-vs-float priority, и фиксированные owned slots.
- Точный перебор сочетаний для текущего небольшого fixture pool, а не подбор
  одного ближайшего лота на каждый слот по отдельности.
- Реальные caps/collection edges для Knight → Dragon Lore (100%) и Chroma
  (два возможных Covert outcome), плюс React/TypeScript UI.
- Regression tests, включая результат последовательного `f32`, проверенный
  вручную по FloatJitsu.
- Постоянный background sync: при старте и затем раз в 24 часа backend берёт
  актуальные pinned commits ByMykel + SteamTracking, хранит полный snapshot в
  Docker volume `./runtime/` и публикует результат сверки caps.

## Фоновая синхронизация и реальные регрессии

После `docker compose up` приложение автоматически скачивает полный каталог
ByMykel (`skins.json` и `collections.json`), фиксирует SHA каждого источника и
сохраняет их в `runtime/catalog/<commit>/`. Затем оно извлекает `paint_kits`
из Valve game data и формирует отчёт `verification.json`: совпадения и
расхождения cap’ов видны, а не скрываются.

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

### Набор фактических контрактов

Игра и публичные inspect APIs не публикуют историю самого контракта — они
раскрывают данные **одного** предмета, но не связывают 10 входов с реально
полученным output. Поэтому приложение не заполняет regression suite
придуманными записями. Вместо этого есть persistent collector:

```bash
curl -X POST http://localhost:8080/api/regressions/contracts \
  -H 'content-type: application/json' \
  --data @actual-contract.json

curl http://localhost:8080/api/regressions/status | jq
```

`actual-contract.json` содержит 10 `{ "skin_id", "float_value" }`,
`actual_output_skin_id`, `actual_output_float` и, желательно, `evidence_url`
с inspect link/публичным доказательством. Запись сохраняется в
`runtime/actual-contracts.json` вместе с predicted и actual float32 bits.
После 20 фактических записей endpoint отметит suite как готовый. В текущем
MVP collector валидирует paths из embedded-каталога; full snapshot уже
сохраняется и проверяется, а его перевод в multi-collection runtime catalog —
следующий безопасный schema migration.

Проверки:

```bash
cargo test
cd frontend && npm run build
```

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

API вернёт 10 слотов, из которых 5 будут `source: "owned"`, цену только
недостающих fixture listings, target probability, output float и all outcomes.

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
  Steam, а не на конкретный asset. Поэтому этот срез не обещает покупки.
- Steam публично не предоставляет стабильный anonymous API, который отдаёт
  конкретные listing + inspect + exact float. Для реального FR-4 нужен
  аутентифицированный/browser-backed provider, cache/rate limits и UI для
  свежести предложений.
- Современные CS2 inspect links можно локально декодировать через
  [csfloat/cs-inspect-serializer](https://github.com/csfloat/cs-inspect-serializer)
  и сохранить `paintwear` как f32 bits; в этой версии UI принимает exact float
  вручную, чтобы не подменять недоступный inspector.
- 5 Covert → knife/gloves признан на уровне валидации, но не материализован:
  для него нужен отдельный versioned unusual/loot outcome map и реальные
  регрессионные крафты. Никаких выдуманных knife outcomes не показывается.
- EV, комиссии, ликвидность, Souvenir и подтверждённое влияние порядка слотов
  оставлены на следующий этап.
