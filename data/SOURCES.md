# Каталог float caps и outcome pools

В MVP используется маленький проверенный subset в `catalog.v1.json`, чтобы
математическое ядро и UI были воспроизводимы без сетевой зависимости.

## Основной импортный источник

- [ByMykel/CSGO-API](https://github.com/ByMykel/CSGO-API), лицензия MIT.
- Зафиксированный upstream commit:
  `0a2030006075f2e184806bc956223f1e8a42d436`.
- Входные файлы:
  [`skins.json`](https://raw.githubusercontent.com/ByMykel/CSGO-API/0a2030006075f2e184806bc956223f1e8a42d436/public/api/en/skins.json)
  и
  [`collections.json`](https://raw.githubusercontent.com/ByMykel/CSGO-API/0a2030006075f2e184806bc956223f1e8a42d436/public/api/en/collections.json).

`skins.json` даёт `min_float`, `max_float`, StatTrak-совместимость, rarity,
weapon/paint index и membership в коллекциях. `collections.json` задаёт pool
предметов в коллекции и rarity, поэтому normal trade-up outcome materialize
как `collection × next_rarity`.

Обновление snapshot:

```bash
./scripts/fetch-catalog-snapshots.sh
```

Скрипт только скачивает pinned JSON, не перезаписывая проверенный subset.
Полный importer должен использовать составной ключ `(weapon def_index,
paint_index)`, а не один `paint_index`: один paint kit допустим у нескольких
видов оружия.

## Что такое «absolute float»

Отдельной таблицы skin → absolute float не существует: значение детерминированно
вычисляется по caps конкретного skin.

```text
adjusted = f32((raw - skin_min) / (skin_max - skin_min))
raw      = f32(skin_min + adjusted * (skin_max - skin_min))
```

В базе хранятся caps; `adjusted` — производное универсальное значение 0–1.
В runtime вместе с десятичным отображением сохраняются IEEE-754 `f32` bits.

## Независимая сверка

[SteamTracking/GameTracking-CS2](https://github.com/SteamTracking/GameTracking-CS2)
содержит актуальные Valve game data: в `paint_kits` используются
`wear_remap_min` и `wear_remap_max`. Его следует применять для QA/diff
нового snapshot, но не вендорить: это не OSS-лицензированный каталог.

Для inspect links ориентир — MIT-проект
[csfloat/cs-inspect-serializer](https://github.com/csfloat/cs-inspect-serializer):
современный payload содержит `paintwear` как `u32` bits f32. Это планируемый
следующий адаптер для owned items; текущий MVP честно принимает exact float
вручную.
