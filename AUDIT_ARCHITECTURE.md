# 🔍 Аудит архитектуры и структуры проекта RustRag

**Дата:** 2026-06-10  
**Проект:** /home/odolen/RustRag  
**Тип:** Rust workspace (5 crate'ов) — RAG для кода на Rust

---

## Итоговая оценка: **7.2 / 10** ⭐⭐⭐⭐⭐⭐☆☆☆

---

## 1. Архитектура и проектирование — 9/10 ✅ Отлично

### Структура workspace
- Чистая Cargo workspace с `resolver = "2"` — современный стандарт
- Единая `[workspace.package]` для версий, edition и rust-version (`1.85`)
- `[workspace.dependencies]` для общих зависимостей (anyhow, thiserror, serde, tokio)

### Граф зависимостей (ацикличный, чёткая иерархия):
```
core ← llm ← cli
            ↖── tui ← cli
server ← core, llm
```

| Crate | Зависимости | Назначение |
|-------|-------------|------------|
| `rust-rag-core` | Нет (ядро) | Индексация, эмбеддинги, векторное хранилище, BM25, call graph |
| `rust-rag-cli` | core + llm + tui | CLI binary с подкомандами |
| `rust-rag-server` | core + llm | HTTP API (axum) + MCP stdio server |
| `rust-rag-llm` | core | LLM client abstraction (ChatBackend trait) |
| `rust-rag-tui` | core + llm | Интерактивный терминальный UI (ratatui) |

### Потенциальные проблемы
- Зависимость `ra_ap_syntax = "=0.0.178"` зафиксирована по патчу — может вызывать конфликты при обновлении
- Жёсткая привязка к `fastembed = "=4.0.0"` тоже с потенциальными конфликтами

---

## 2. Качество кодовой базы

### core — 8/10 ✅ Хорошо
| Модуль | Оценка | Комментарии |
|--------|--------|-------------|
| `config.rs` (67 строк) | 9/10 | Чистый, хорошо структурированный. `Config::find()` с восходящим поиском — отличный паттерн. |
| `state.rs` (139 строк) | 8/10 | Incremental state с SHA-256 хешированием. Атомарное сохранение через temp+rename. |
| `embedding.rs` (371 строка) | 8/10 | Многоуровневый поиск модели (env → config → HF cache → Download/) — отличный fallback chain. LazyLock с `expect()` может паниковать при инициализации. |
| `indexer.rs` (381 строка) | 7/10 | AST-индексация через tree-sitter. Рекурсия по AST без защиты от стекового переполнения на глубоко вложенных файлах. |
| `vector_store.rs` (551 строка) | 8/10 | Solid BM25 реализация. `panic!("Unknown SymbolKind in index")` — unwrap/panic в десериализации плохая практика для production. |
| `retrieval.rs` (84 строки) | 6/10 | `_graph` параметр с подчеркиванием — unused parameter hint, MVP placeholder. |
| `callgraph.rs` (126 строк) | 7/10 | Рекурсивный обход descendants AST для каждого чанка — может быть медленно на больших файлах. |

### cli — 7/10 ⚠️ Требует улучшения
- **main.rs** (171 строка): Чистый clap derive API, `#[tokio::main] async fn main()`
- **lib.rs** (681 строка): КРИТИЧЕСКАЯ ПРОБЛЕМА — дублирование кода! `ask()`, `ask_stream()`, `ask_json()`, `ask_stream_json()` повторяют retrieval pipeline 4 раза. Также `show_info()`/`show_info_json()` и `search_symbol()`/`search_symbol_json()`.
- **cmd/*.rs** (5 строк каждый): Все файлы содержат ровно 4-5 строк — пустые обёртки-диспетчеры, добавляющие косвенность без ценности.

### llm — 8/10 ✅ Хорошо
- **lib.rs** (40 строк): Чистый trait `ChatBackend` с default method implementations. Template method pattern работает хорошо.
- **ollama_client.rs** (256 строк): SSE streaming парсер корректно обрабатывает OpenAI/Ollama/llama.cpp форматы. Default model name слишком специфичный: `"Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf"` — должен быть нейтральный (например `llama3`).

### server — 7/10 ⚠️ Требует улучшения
- **lib.rs** (286 строк): Axum router с CORS. 3 duplicate system prompt строки в разных handlers. `query_stream_handler` (~90 строк) слишком большой. Generic `StatusCode::INTERNAL_SERVER_ERROR` без детализации.
- **mcp.rs** (427 строк): Полная реализация MCP stdio protocol с JSON-RPC 2.0, batch requests, tools/list, tools/call. unused field `protocol_version`.
- **bin.rs** (74 строки): Чистый entry point.

### tui — 6/10 ⚠️ Требует серьёзного улучшения
- **app.rs** (508 строк): КРИТИЧЕСКАЯ ПРОБЛЕМА — монолитный App struct нарушает single responsibility. `draw()` содержит ~200 строк только за рендеринг. Hardcoded цвета (`Color::White`, `Color::Blue`) вместо темизированной палитры. Нет поддержки dark/light mode. Используется `std::thread::spawn` + mpsc вместо tokio — устаревший паттерн.

---

## 3. Качество кода

### Стиль кодирования — 8/10 ✅
- `cargo fmt --all` проходит без ошибок
- Использование `anyhow::Result` повсеместно — хороший выбор для CLI
- Pattern matching читаемый и последовательный

### Обработка ошибок — 7/10 ⚠️
- `anyhow::Result` повсеместно
- ЕСТЬ panic!() в production-путях:
  - `vector_store.rs:336` — `panic!("Unknown SymbolKind in index")`
  - `embedding.rs:148` — `LazyLock::new(|| init_embedder().expect(...))`

### async/await — 8/10 ✅
- `#[tokio::main] async fn main()` в CLI и server binary
- Shared static `LazyLock<Runtime>` для sync contexts — правильно решена проблема nested runtimes

### Тестирование — 9/10 ✅ Отлично
**40 тестов проходят успешно!** Покрытие edge cases:
- ✅ Индексация workspace, Vector store roundtrip + multi-doc + deletion
- ✅ Cosine similarity (identical, orthogonal, opposite, empty, mismatched lengths)
- ✅ Hybrid search (alpha=0 pure BM25, alpha=1.0 pure vector, blending)
- ✅ Search filters (symbol kind, file extension, various kinds)
- ✅ Chunk overlap (extends boundaries, single chunk noop, zero is noop, multi-file isolation)
- ✅ Incremental state (detects changes, skips unchanged, removes deleted, detects new files)
- ✅ BM25 edge cases (empty documents, dissimilar query ranking)
- ✅ End-to-end RAG pipeline

### Документация — 7/10 ⚠️
- README.md — отличный, подробный (~244 строки), с архитектурой, CLI reference, API docs
- CHANGELOG.md — структурирован по версиям
- НЕТ `///` doc comments на публичных функциях в большинстве модулей

---

## 4. Потенциальные проблемы

### Code Smells / Антипаттерны

| Проблема | Где | Серьёзность |
|----------|-----|-------------|
| Дублирование retrieval pipeline 4 раза | `cli/lib.rs` — ask, ask_stream, ask_json, ask_stream_json | Средняя |
| Пустые cmd/*.rs файлы (5 строк каждый) | Все 7 файлов в `cmd/` | Низкая |
| Монолитный App struct (~508 строк) | `tui/app.rs` | Средняя — нарушает SRP |
| Hardcoded system prompt 4+ раза | cli, server, tui — один и тот же промпт | Низкая |

### Безопасность (предварительная оценка) — 7/10 ⚠️
- ✅ MCP server имеет JSON Schema валидацию входных данных
- ✅ HTTP endpoint не принимает произвольные файлы на запись
- ⚠️ `RUSRAG_WORKSPACE` env var используется для определения workspace path — потенциальный путь к LFR (Local File Read) если передан внешний path
- ⚠️ Model download с HuggingFace без проверки TLS/SSL integrity

### Производительность (предварительная оценка) — 7/10 ⚠️
- ✅ Batch embedding через fastembed (один ONNX inference вместо N)
- ✅ Incremental indexing с SHA-256 hash comparison
- ⚠️ BM25 inverted index строится заново при каждом поиске — должен кешироваться
- ⚠️ `parse_call_exprs` рекурсивно обходит descendants для каждого чанка — O(n * m)

### Утечки ресурсов / Missing Drop — 8/10 ✅
- ✅ File handles закрываются через RAII (BufWriter flush + drop)
- Shared Tokio runtimes (`LazyLock`) никогда не останавливаются — OK для CLI
- Atomic file operations предотвращают corruption

### TODO/FIXME/HACK/XXX комментарии — 10/10 ✅ Отлично
- **0 найденных** TODO/FIXME/HACK/XXX комментариев — проект хорошо ухожен. Все известные проблемы задокументированы в TODOS.md и закрыты.

---

## 5. Конфигурация и инфраструктура

### CI/CD — 3/10 ❌ КРИТИЧЕСКИ НИЗКО
- ❌ `.github/workflows/` директория пустая — нет GitHub Actions workflow файлов
- Нет автоматического тестирования при PR/commit
- Нет линтинга/clippy в CI
- Нет автобилда релизов

### Конфигурация — 8/10 ✅
- `.rustrag.toml` — понятный TOML с `[embedding]` и `[llm]` секциями
- Environment variable overrides (RUSRAG_MODEL_PATH, LLAMA_ENDPOINT, LLAMA_MODEL) — хороший fallback chain

### CHANGELOG.md — 8/10 ✅
- Структурирован по версиям с секциями Added/Fixed
- Unreleased section присутствует

### TODOS.md — 7/10 ✅
- Содержит историю предыдущих code review стадий (3-10)
- Все пункты закрыты ✅

---

## Ключевые рекомендации по улучшению (приоритет):

### 🔴 CRITICAL
1. **Создать CI/CD workflow** с `cargo test`, `cargo clippy`, `cargo fmt --check` — критически важно для качества кода

### 🟡 HIGH PRIORITY
2. **Рефакторировать CLI lib.rs** — вынести общий retrieval pipeline в одну функцию с configurable output mode (enum)
3. **Удалить пустые cmd/*.rs файлы** — либо наполнить смыслом, либо убрать
4. **Прокешировать BM25 inverted index** на VectorStore уровне для улучшения производительности

### 🟢 MEDIUM PRIORITY
5. **Заменить `panic!()` на Error** в vector_store десериализации SymbolKind
6. **Вынести system prompt в константу** (3+ места дублирования)
7. **Добавить theme system в TUI** — убрать hardcoded цвета, добавить dark/light палитру
8. **Заменить специфичный default model name** на нейтральный (например `llama3`)

### 🔵 LOW PRIORITY
9. Добавить `///` doc comments на публичные API в core и llm модулях
10. Рассмотреть защиту от рекурсивного обхода AST без лимита глубины в indexer.rs
11. Заменить `std::thread::spawn` + mpsc в TUI на tokio mpsc

---

## Сводная таблица оценок по категориям

| Категория | Оценка | Комментарий |
|-----------|--------|-------------|
| Архитектура и проектирование | **9/10** | Отличная workspace структура, чёткие границы между крэйтами, ацикличные зависимости |
| Качество core | **8/10** | Solid BM25 + vector hybrid search, good incremental indexing, minor panic on deserialization |
| Качество CLI | **7/10** | Дублирование кода (4x retrieval pipeline), пустые cmd/*.rs файлы |
| Качество LLM | **8/10** | Clean trait abstraction, good SSE parsing, shared runtime pattern correct |
| Качество Server | **7/10** | Solid MCP implementation, но large handlers и duplicate system prompts |
| Качество TUI | **6/10** | Monolithic App struct (508 строк), hardcoded цвета, нет theme system, legacy mpsc threading |
| Обработка ошибок | **7/10** | anyhow::Result повсеместно, но есть panic!() в production-путях |
| Тестирование | **9/10** | 40 тестов с хорошим покрытием edge cases |
| Документация | **7/10** | Отличный README, но мало doc comments на публичных API |
| CI/CD | **3/10** | ❌ Пустые workflow файлы — критический пробел |

---

*Аудит выполнен автоматически агентом-исследователем*  
*Для полного security audit рекомендуется запустить отдельный аудит безопасности*
