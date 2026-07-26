# Floatcraft

Нативное desktop-приложение для расчёта CS2 trade-up контрактов: React UI в
Tauri WebView и Rust-ядро с последовательной IEEE-754 `f32` математикой.
Приложение не запускает localhost-сервер, не требует Docker и не отправляет
пользовательские данные на собственный backend.

## Установка и запуск для разработки

Нужны Rust 1.96+, Node 22+ и системные prerequisites Tauri для вашей ОС.

```bash
cd frontend
npm ci
npm run tauri:dev
```

Сборка production-пакета для текущей платформы:

```bash
cd frontend
npm run tauri:build
```

## Что работает локально

- Reverse planner и forward validation с точной `float32` математикой.
- Встроенный fallback-каталог: UI и расчёты открываются без сети.
- SQLite в системной папке данных приложения: настройки, evidence-backed
  regression contracts, активный catalog snapshot, sync state и журнал
  синхронизаций; старые JSON импортируются однократно.
- Версионированный полный каталог: обновляется в фоне, атомарно переключается
  только после успешной проверки, а последняя валидная версия сохраняется.
- UI проверяет обновления через подписанные GitHub Releases. Скачивание и
  перезапуск всегда подтверждает пользователь.

## Сеть и Steam Market

По умолчанию live Steam Market включён в настройках, но перед первым запросом
приложение показывает предупреждение и требует подтверждения. Provider не
использует Steam key, cookie или авторизацию; он читает публичную SSR-разметку,
кэширует результаты, ограничивает частоту запросов и работает только во время
построения плана. Один план ограничен 20 HTTP-запросами и 30 секундами; при
таймауте или ошибке он всё равно возвращает local `ideal_math` с явной причиной.

Steam не предоставляет для этого стабильный документированный API. Если
разметка изменилась, сеть недоступна или запросы ограничены, планировщик честно
возвращается к `ideal_math`, без выдуманных цен и ссылок. Market можно отключить
в интерфейсе в любой момент.

## Данные и восстановление

SQLite использует WAL, foreign keys, транзакционные миграции и проверку
целостности при старте. Перед миграцией создаётся резервная копия. Полные
каталоги хранятся как versioned snapshot в системной папке данных приложения,
а встроенный каталог остаётся безопасным fallback. GitHub commit resolution
использует ETag, поэтому неизменившиеся upstream snapshots не скачиваются снова.

## Проверки

```bash
cargo test --locked
cd frontend && npm test && npm run build
cargo check --manifest-path frontend/src-tauri/Cargo.toml
```

GitHub Actions выполняет эти проверки на PR. Теги формата `vX.Y.Z` запускают
release pipeline, но только если `X.Y.Z` совпадает с версиями manifest и tag
указывает на коммит из `main`. После обязательной проверки и approval
environment `release` macOS ARM и Windows x64 runners публикуют единый draft,
который становится stable GitHub Release лишь после успешной сборки обеих
платформ.

## Ограничения public beta

Updater-артефакты криптографически подписаны отдельным ключом, чей private key
хранится только в GitHub Actions environment secret. Однако public beta не
notarized Apple и не имеет Windows code-signing certificate: Gatekeeper или
SmartScreen могут потребовать ручного подтверждения установки. Для production
релиза понадобятся Apple Developer notarization и Windows signing certificate.

Подробности источников каталога — в [data/SOURCES.md](data/SOURCES.md).
