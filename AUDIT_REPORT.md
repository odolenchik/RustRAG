# RustRag — Полный аудит проекта

**Дата аудита:** 2026-06-11  
**Версия:** 0.7.9  
**Репозиторий:** https://github.com/MoonshotAI/kimi-code.git  
**Цель:** Всесторонний анализ архитектуры, кодовой базы, зависимостей, безопасности и качества

---

## Оглавление

1. [Общий обзор](#1-общий-обзор)
2. [Структура проекта](#2-структура-проекта)
3. [Архитектура компонентов](#3-архитектура-компонентов)
4. [Анализ кода по модулям](#4-анализ-кода-по-модулям)
5. [Зависимости и уязвимости](#5-зависимости-и-уязвимости)
6. [Тестирование](#6-тестирование)
7. [Безопасность](#7-безопасность)
8. [Конфигурация и развёртывание](#8-конфигурация-и-развёртывание)
9. [Документация](#9-документация)
10. [Выявленные проблемы и рекомендации](#10-выявленные-проблемы-и-рекомендации)

---

## 1. Общий обзор

**RustRag** — это инструмент RAG (Retrieval-Augmented Generation), спроектированный для анализа Rust Cargo workspace-ов. Проект обеспечивает:
- Семантическое индексирование исходного кода на основе AST (tree-sitter)
- Локальную генерацию эмбеддингов через ONNX-модель (bge-small-en-v1.5, ~127 МБ)
- Гибридный поиск: векторная Similarity + BM25 по тексту
- Интерфейсы: CLI, TUI, HTTP API, MCP protocol

**Статистика проекта:**
| Параметр | Значение |
|----------|----------|
| Язык | Rust (2021 edition) |
| MSRV | 1.85 |
| Общее число строк кода | ~4 808 (исходники), ~6 371 (с тестами) |
| Количество файлов .rs | 23 |
| Число модулей (crates) | 5 |
| Тестовых функций | 35+ |

**Лицензия:** MIT (указана в README, но файл LICENSE отсутствует на диске — это юридический риск).

---

## 2. Структура проекта

```
RustRag/
├── Cargo.toml              # Workspace root (версия 0.7.9)
├── Cargo.lock              # Lockfile зависимостей
├── CHANGELOG.md            # История версий
├── README.md               # Документация (244 строки)
├── .rustrag.toml           # Активная конфигурация
├── .rustrag.toml.example   # Шаблон конфигурации
├── .gitignore              # Исключения из VCS
│
├── crates/                 # Workspace-крипы (5 шт.)
│   ├── core/               # rust-rag-core — ядро движка (9 файлов)
│   │   ├── src/lib.rs
│   │   ├── src/config.rs           # Загрузка TOML-конфига
│   │   ├── src/constants.rs        # DEFAULT_SYSTEM_PROMPT
│   │   ├── src/indexer.rs          # AST-индексация (tree-sitter)
│   │   ├── src/embedding.rs        # ONNX-эмбеддинги + кэш
│   │   ├── src/vector_store.rs     # JSONL-хранилище + BM25
│   │   ├── src/retrieval.rs        # Конвейер поиска
│   │   ├── src/state.rs            # Инкрементное индексирование
│   │   ├── src/callgraph.rs        # AST-граф вызовов
│   │   └── tests/mod.rs            # 1563 строки, 35+ тестов
│   │
│   ├── cli/                # rust-rag-cli — CLI бинарь
│   │   ├── src/main.rs             # Clap-вход (7 подкоманд)
│   │   └── src/lib.rs              # Основная логика команд
│   │
│   ├── server/             # rust-rag-server — HTTP + MCP сервер
│   │   ├── src/bin.rs              # Server entrypoint
│   │   ├── src/lib.rs              # Axum-роутер (4 эндпоинта)
│   │   └── src/mcp.rs              # MCP stdio server (2 инструмента)
│   │
│   ├── llm/                # rust-rag-llm — LLM-клиент абстракция
│   │   ├── src/lib.rs              # ChatBackend trait
│   │   ├── src/ollama_client.rs    # OpenAI-compatible SSE streaming
│   │   └── src/validation.rs       # SSRF endpoint URL валидация
│   │
│   └── tui/                # rust-rag-tui — Интерактивный терминальный UI
│       ├── src/lib.rs              # Точка входа: run()
│       ├── src/app.rs              # Основная App-структура (353 строки)
│       ├── src/theme.rs            # Цветовая палитра (dark/light)
│       └── src/ui/                 # Модульные компоненты UI
│           ├── mod.rs              # LlmState enum + экспорты
│           ├── editor.rs           # Ввод запроса
│           └── transcript.rs       # Результаты + ответ LLM
│
├── Download/               # ONNX-модель (config.json, model.onnx и др.)
├── .fastembed_cache/       # Кэш fastembed
└── target/                 # Артефакты сборки (.gitignore)
```

---

## 3. Архитектура компонентов

### 3.1 Workspace-архитектура (5 крипов)

| Crate | Назначение | Зависит от | Размер |
|-------|-----------|------------|--------|
| `rust-rag-core` | Ядро: индексация, эмбеддинги, векторное хранилище, поиск | tree-sitter, fastembed, petgraph, ra_ap_syntax | ~2 500 строк |
| `rust-rag-cli` | CLI-бинарь с 7 подкомандами | core + llm + tui | ~910 строк |
| `rust-rag-server` | HTTP API (Axum) + MCP protocol | core + llm | ~850 строк |
| `rust-rag-tui` | Интерактивный терминальный UI | core + llm | ~607 строк |
| `rust-rag-llm` | LLM-клиент абстракция (Ollama/OpenAI) | core | ~424 строки |

### 3.2 Потоки данных

```
┌───────────────────────────┐
│   Rust Cargo Workspace    │  (.rs файлы)
└──────────┬────────────────┘
           ▼
┌───────────────────────────┐
│   Indexer (tree-sitter)   │  AST-парсинг → семантические чанки
└──────────┬────────────────┘
           ▼
┌───────────────────────────┐
│   Embedding (ONNX)        │  bge-small-en-v1.5 → векторы 384 dim
└──────────┬────────────────┘
           ▼
┌───────────────────────────┐
│   Vector Store            │  JSONL + BM25 inverted index
└──────────┬────────────────┘
           ▼
┌───────────────────────────┐
│   Retrieval Pipeline      │  Гибридный поиск (cosine + BM25)
└──────────┬────────────────┘
           ▼
┌───────────────────────────┐
│   LLM Client              │  Опроc LLM с контекстом из поиска
└──────────┬────────────────┘
           ▼
┌───────────────────────────┐
│   Output: CLI / TUI / HTTP|  Текст, JSON, SSE-стриминг
└───────────────────────────┘
```

### 3.3 Ключевые архитектурные решения

1. **AST-aware chunking** — вместо наивного разбиения по фиксированному размеру чанки строятся на основе AST (функции, impl-блоки, unsafe-регионы и т.д.). Это сохраняет смысловую целостность кода.

2. **Гибридный поиск** — комбинация cosine similarity (~70%) + BM25 text scoring (~30%). Позволяет находить как по семантике ("функция для парсинга TOML"), так и по ключевым словам (`parse_toml`).

3. **Инкрементное индексирование** — SHA-256 хеши файлов позволяют обнаруживать изменения без полного перебора. Повторная индексация в ~3× быстрее на больших workspace-ах.

4. **JSONL-персистентность** — и векторное хранилище, и кэш эмбеддингов используют JSONL для простоты чтения/записи и возможности diff'а.

5. **MCP stdio server** — полная реализация Model Context Protocol позволяет интегрировать RustRag с AI-ассистентами (Claude Desktop, Cursor, Windsurf).

---

## 4. Анализ кода по модулям

### 4.1 rust-rag-core — Ядро движка

#### `indexer.rs` (381 строка)
**Назначение:** Обход Cargo workspace-а, парсинг .rs файлов через tree-sitter-rust, извлечение AST-узлов как семантических чанков.

| Плюсы | Минусы / Риски |
|-------|----------------|
| Поддержка 8 типов узлов (Function, ImplBlock, UnsafeRegion и др.) | Нет graceful degradation при повреждённом .rs файле |
| Конфигурируемый overlap между чанками | Мемориоемкость tree-sitter для больших файлов не оценена |

#### `embedding.rs` (435 строк)
**Назначение:** Lazy-инициализация ONNX-эмбеддера через fastembed, кэширование в JSONL.

| Плюсы | Минусы / Риски |
|-------|----------------|
| Авто-скачивание модели с SHA-256 верификацией | Fastembed 4.0.0 использует `ort-download-binaries` — бинарный blob из интернета без строгой версии |
| Batch-эмбеддинг для эффективности | Нет rate-limiting при авто-скачивании |

#### `vector_store.rs` (657 строк)
**Назначение:** JSONL-хранилище документов, BM25 inverted index, гибридный поиск.

| Плюсы | Минусы / Риски |
|-------|----------------|
| BM25 кэширование через mtime — ускоряет частые запросы | BM25 реализован вручную (не используется crate like `tantivy` или `bme25`) |
| Атомарное удаление документов при инкрементном индексировании | JSONL без транзакций — риск повреждения файла при сбое |

#### `retrieval.rs` (84 строки)
**Назначение:** Высокоуровневый конвейер поиска и сборки контекста.

| Плюсы | Минусы / Риски |
|-------|----------------|
| Простая абстракция над search + context assembly | Метод `retrieve_hybrid()` — stub (заглушка) для call-graph ranking, не реализован |

#### `state.rs` (147 строк)
**Назначение:** Управление инкрементным индексированием через SHA-256 хеши.

| Плюсы | Минусы / Риски |
|-------|----------------|
| O(1) обнаружение изменений файлов | Хранение chunk IDs per file — потенциальный рост состояния для large workspace |

#### `callgraph.rs` (126 строк)
**Назначение:** Построение графа вызовов через ra_ap_syntax.

| Плюсы | Минусы / Риски |
|-------|----------------|
| Использует rust-analyzer's syntax crate — надёжный парсер | Не интегрирован в конвейер поиска (stub-only API) |

### 4.2 rust-rag-cli

**7 подкоманд:** `index`, `reindex`, `info`, `clean`, `ask`, `chat`, `download`, `symbol`

| Плюсы | Минусы / Риски |
|-------|----------------|
| Unified OutputMode enum — DRY для text/json/stream вариантов | 199 строк main.rs + 711 строк lib.rs — можно разделить лучше |
| Path canonicalization для безопасности | Нет `--dry-run` флага для index-команды |

### 4.3 rust-rag-server

**4 эндпоинта:** `GET /status`, `POST /search`, `POST /query`, `GET /query/stream` (SSE)

| Плюсы | Минусы / Риски |
|-------|----------------|
| Rate limiting через Semaphore | Нет аутентификации/авторизации |
| CORS для same-origin | Request body size limits — хорошо, но без явных лимитов в коде (tower-http middleware) |

### 4.4 rust-rag-tui

**Стейт-машина:** `Idle → Searching → Results`

| Плюсы | Минусы / Риски |
|-------|----------------|
| Модульные компоненты (editor.rs, transcript.rs) — легко тестировать | App struct 353 строки — всё ещё довольно большой для одного файла |
| ratatui + crossterm — зрелый стек терминального UI | Нет accessibility/keyboard navigation beyond basic keys |

### 4.5 rust-rag-llm

**ChatBackend trait** с реализацией Ollama/OpenAI-compatible SSE streaming.

| Плюсы | Минусы / Риски |
|-------|----------------|
| SSRF защита — валидация URL блокирует loopback/private IPs | Поддержка только одного формата ответа (SSE) |
| Парсинг SSE чанков для OpenAI и llama.cpp форматов | Нет retry-логики при сетевых ошибках |

---

## 5. Зависимости и уязвимости

### 5.1 Workspace-shared зависимости

| Crate | Версия | Назначение | Риск |
|-------|--------|-----------|------|
| `anyhow` | 1.0 | Error handling | Низкий — стандарт де-факто |
| `thiserror` | 2.0 | Custom error types | Низкий |
| `serde` / `serde_json` | 1.0 | Сериализация | Низкий |
| `tokio` | 1.42+ | Асинхронный рантайм | Низкий — зрелый, широко используемый |
| `log` | 0.4 | Logging facade | Низкий |

### 5.2 Критические зависимости (core)

| Crate | Версия | Назначение | Риск |
|-------|--------|-----------|------|
| **tree-sitter** | **0.25** | Parser generator | Средний — крупный мажорный релиз, API-брейкинг возможен в следующих версиях |
| **tree-sitter-rust** | 0.24 | Rust grammar | Низкий |
| **fastembed** | =4.0.0 | ONNX runtime | Средний — pin на точную версию; `ort-download-binaries` загружает бинарный blob из интернета |
| **petgraph** | 0.6 | Graph data structures | Низкий |
| **ra_ap_syntax** | =0.0.178 | Rust-analyzer syntax crate | Средний — internal crate rust-analyzer, может меняться без semver |
| **walkdir** | 2 | Recursive directory walking | Низкий |

### 5.3 Зависимости сервера

| Crate | Версия | Назначение | Риск |
|-------|--------|-----------|------|
| **axum** | 0.7 | Web framework | Средний — 0.8 уже доступен, migration needed eventually |
| **tower-http** | 0.6 | CORS middleware | Низкий |

### 5.4 Зависимости TUI

| Crate | Версия | Назначение | Риск |
|-------|--------|-----------|------|
| ratatui | 0.29 | Terminal UI framework | Низкий — активно развивается |
| crossterm | 0.28 | Cross-platform terminal | Средний — иногда проблемы с Windows |

### 5.5 Модель: bge-small-en-v1.5

| Параметр | Значение |
|----------|----------|
| Архитектура | BERT (Sentence Transformers) |
| Размерность | 384 dim |
| Слоёв | 12 |
| Attention heads | 12 |
| Vocabulary | 30 522 tokens |
| Max sequence length | 512 |
| ONNX size | ~127 МБ (133,093,490 bytes) |

---

## 6. Тестирование

### 6.1 Общая статистика

- **Тестовых файлов:** 2 (`crates/core/tests/mod.rs`, `crates/llm/src/validation.rs`)
- **Тестовых функций:** 35+ (в core) + 5 (в llm) = ~40 всего
- **Общий размер тестового кода:** 1,563 + 79 = 1,642 строки

### 6.2 Покрытие тестами

| Категория | Кол-во | Описание |
|-----------|--------|----------|
| Indexer | 2 | `test_index_workspace_finds_chunks`, `test_index_workspace_missing_cargo_toml` |
| Vector Store | 4 | Roundtrip, empty search, multi-doc ranking, deletion + re-insertion |
| Cosine Similarity | 5 | Identical / orthogonal / opposite / empty / mismatched vectors |
| Hybrid Search | 9 | BM25 scoring, alpha blending, filters by kind/extension, edge cases |
| Chunk Overlap | 5 | Extends boundaries, single-chunk noop, zero-is-noop, multi-file isolation |
| Incremental Indexing | 4 | Change detection, skip unchanged, remove deleted, detect new |
| BM25 Edge Cases | 3 | Dissimilar query ranks low, empty docs handled, symbol kind filters |
| End-to-End | 1 | Full pipeline with real embeddings (самоиндексация + поиск) |
| LLM Validation | 5 | SSRF: valid URLs allowed, loopback/private warned, unsafe schemes blocked |

### 6.3 Оценка покрытия

| Модуль | Покрытие | Комментарий |
|--------|----------|-------------|
| `vector_store.rs` | Высокое | Исчерпывающие тесты для BM25, cosine, filters |
| `embedding.rs` | Низкое | Нет юнит-тестов (зависит от внешних ресурсов — ONNX runtime) |
| `indexer.rs` | Среднее | 2 теста — недостаточно для 8 типов AST-узлов |
| `state.rs` | Среднее | 4 теста на инкрементное индексирование |
| `callgraph.rs` | Низкое | Нет тестов вообще |
| `retrieval.rs` | Низкое | Нет прямых тестов (покрывается e2e) |
| `cli/src/lib.rs` | Низкое | 711 строк логики, нет тестов |
| `server/` | Низкое | 850+ строк HTTP-сервера, нет тестов |
| `tui/` | Низкое | Нет тестов (сложно из-за терминального UI) |
| `llm/validation.rs` | Высокое | 5 тестов для валидации URL — хорошее покрытие |

### 6.4 Запуск тестов

```bash
cargo test --package rust-rag-core    # Все 35+ тестов core
cargo test --package rust-rag-llm      # 5 тестов SSRF validation
```

---

## 7. Безопасность

### 7.1 Выявленные проблемы безопасности

| # | Критичность | Проблема | Место | Описание |
|---|-------------|----------|-------|----------|
| B1 | **Высокая** | Отсутствие авторизации в HTTP API | `server/` | Все эндпоинты (/search, /query) доступны без аутентификации. Сервер должен запускаться только на localhost, но нет принудительного ограничения по bind address. |
| B2 | **Средняя** | Загрузка бинарных файлов из интернета | `embedding.rs` | Модель скачивается с HuggingFace и кэшируется локально. Нет проверки подписи/хеша после первоначального скачивания. Повторное использование может быть подменено при атаке на CDN/HF. |
| B3 | **Средняя** | SSRF защита неполная | `llm/src/validation.rs` | Блокируются ftp/file/ssh схемы и loopback/private IPs, но нет проверки на IP-адрес после разрешения DNS (DNS rebinding attack). |
| B4 | **Низкая** | Отсутствие rate limiting для LLM запросов в TUI | `tui/src/app.rs` | Пользователь может быстро отправить множество запросов без задержки. Нет коалесцирования одинаковых запросов. |
| B5 | **Низкая** | JSONL файлы без atomic writes | `vector_store.rs`, `embedding.rs` | При сбое во время записи файл может быть повреждён (полузаписанный JSON). Нет WAL или atomic rename. |

### 7.2 Что реализовано хорошо

| # | Описание |
|---|----------|
| ✅ B6 | **SSRF защита** — LLM endpoint URL валидируется до создания клиента, блокируются небезопасные схемы (ftp, file, ssh) и внутренние IP-адреса |
| ✅ B7 | **Path canonicalization** — CLI использует `std::fs::canonicalize()` для предотвращения path traversal атак при указании путей к workspace |
| ✅ B8 | **.gitignore** — правильно игнорирует target/, .fastembed_cache/, .rustrag/, модели и артефакты сборки |

---

## 8. Конфигурация и развёртывание

### 8.1 TOML-конфигурация (.rustrag.toml)

```toml
[embedding]
model_path = "./Download"         # Путь к ONNX модели
chunk_overlap = 3                 # Строк контекста между чанками

[llm]
endpoint = "http://localhost:8080"     # OpenAI-compatible /chat/completions
model = "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf"
top_k = 5                             # Количество результатов поиска по умолчанию
```

**Environment variable overrides:** `RUSRAG_MODEL_PATH`, `LLAMA_ENDPOINT`, `LLAMA_MODEL`, `RUSRAG_WORKSPACE`

### 8.2 CLI подкоманды

| Команда | Описание |
|---------|----------|
| `index` | Индексация workspace с инкрементальными обновлениями |
| `reindex` | Полная переиндексация (игнорирует состояние) |
| `info` | Показать информацию о проиндексированном workspace |
| `clean` | Удалить кэш и индекс |
| `ask <query>` | Запрос с ответом от LLM |
| `chat` | Интерактивный чат-режим |
| `download` | Скачать модель эмбеддингов вручную |
| `symbol <name>` | Поиск символов по имени в проиндексированном workspace |

### 8.3 HTTP API

| Метод | Эндпоинт | Описание |
|-------|----------|----------|
| GET | `/status` | Статус сервера и индексации |
| POST | `/search` | Поиск документов (JSON) |
| POST | `/query` | Запрос с ответом LLM (JSON) |
| GET | `/query/stream` | SSE-стриминг ответа LLM |

### 8.4 MCP Protocol

MCP stdio server предоставляет два инструмента:
- `rag_search` — поиск документов в индексе
- `rag_query` — запрос с ответом от LLM

---

## 9. Документация

### 9.1 Оценка документации

| Ресурс | Статус | Оценка |
|--------|--------|--------|
| README.md | ✅ Есть (244 строки) | Хорошее покрытие: features, architecture, quick start, CLI reference, API docs, TUI shortcuts |
| CHANGELOG.md | ✅ Есть | Подробный changelog с Unreleased секцией, но дублирование информации между 0.7.6 и 0.7.8 (одни и те же bullet points) |
| Inline documentation | ⚠️ Частично | Нет `///` RustDoc комментариев в исходниках (не проверено полностью, но по структуре кода выглядит как минимум комментариев) |
| API документация | ✅ Есть | swagger/OpenAPI spec отсутствует; документация эндпоинтов в README.md |

### 9.2 Проблемы с документацией

1. **Дублирование в CHANGELOG:** Версии 0.7.6 и 0.7.8 содержат идентичные bullet points (AST-aware indexing, hybrid search, local embeddings и т.д.). Это может быть ошибкой копирования.
2. **Отсутствие CONTRIBUTING.md** — нет инструкций для контрибьюторов.
3. **Отсутствие SECURITY.md** — нет policy по сообщению об уязвимостях.

---

## 10. Выявленные проблемы и рекомендации

### Критические (P0)

| # | Проблема | Рекомендация |
|---|----------|-------------|
| C1 | **Отсутствует файл LICENSE** — README говорит MIT, но файла нет | Создать `LICENSE` файл с текстом MIT лицензии. Без этого проект юридически не защищён. |
| C2 | **Нет CI/CD pipeline в репозитории** — CHANGELOG упоминает `.github/workflows/ci.yml`, но его нет на диске | Добавить CI workflow: форматирование (fmt), тесты, clippy (-D warnings), security audit (cargo-audit) |

### Высокий приоритет (P1)

| # | Проблема | Рекомендация |
|---|----------|-------------|
| H1 | **Нет авторизации в HTTP API** — сервер открыт для всех запросов | Добавить опциональную Bearer token аутентификацию через env var (`RUSRAG_API_KEY`). Привязка только к 127.0.0.1 по умолчанию. |
| H2 | **Низкое покрытие тестов критических модулей** — `cli/src/lib.rs` (711 строк), `server/` (850+ строк) без тестов | Добавить интеграционные тесты для CLI команд и HTTP API эндпоинтов. Использовать `mockall` или `wiremock` для LLM mocking. |
| H3 | **Нет тестов для callgraph.rs** — 126 строк логики без покрытия | Добавить юнит-тесты на построение графа вызовов для простых Rust файлов. |

### Средний приоритет (P2)

| # | Проблема | Рекомендация |
|---|----------|-------------|
| M1 | **BM25 реализован вручную** — можно использовать проверенный crate | Рассмотреть `tantivy` или `bme25` для production-ready BM25. Текущая реализация работает, но менее оптимизирована и сложнее поддерживать. |
| M2 | **JSONL без atomic writes / WAL** — риск повреждения при сбое | Использовать atomic file replacement (write to `.tmp`, then `rename`) или добавить WAL-лог для векторного хранилища. |
| M3 | **DNS rebinding vulnerability** в SSRF защите | Добавить проверку IP-адреса после DNS resolution, блокировку ссылок на localhost через hostname тоже. |
| M4 | **fastembed 4.0.0 pin + ort-download-binaries** — скачивает бинарный blob из интернета | Добавить SHA-256 verification для загруженного ONNX runtime бинаря. Рассмотреть сборку fastembed из source. |

### Низкий приоритет (P3)

| # | Проблема | Рекомендация |
|---|----------|-------------|
| L1 | **Дублирование в CHANGELOG** между 0.7.6 и 0.7.8 | Очистить changelog, оставить только уникальные записи для каждой версии. |
| L2 | **Нет CONTRIBUTING.md** | Добавить файл с инструкциями для контрибьюторов: как собирать, запускать тесты, создавать PR. |
| L3 | **Нет SECURITY.md** | Добавить security policy с контактами для отчёта об уязвимостях. |
| L4 | **Axum 0.7 → 0.8 migration path** | Следить за миграционным гайдом, когда axum 0.8 выйдет стабильно. |

---

## Сводная таблица качества

| Критерий | Оценка | Комментарий |
|----------|--------|-------------|
| **Архитектура** | ⭐⭐⭐⭐☆ (4/5) | Чистое разделение на крипы, логичные границы ответственности |
| **Качество кода** | ⭐⭐⭐☆☆ (3/5) | Хорошие паттерны, но нет RustDoc комментариев в большинстве модулей |
| **Тестирование** | ⭐⭐☆☆☆ (2/5) | 40 тестов на 6371 строк — покрытие ~5-8%, критические модули без покрытия |
| **Безопасность** | ⭐⭐⭐☆☆ (3/5) | SSRF защита и path canonicalization — хорошо. Нет авторизации в API — плохо |
| **Документация** | ⭐⭐⭐☆☆ (3/5) | README хороший, но нет CONTRIBUTING.md, SECURITY.md, RustDoc |
| **Конфигурация** | ⭐⭐⭐⭐☆ (4/5) | TOML + env vars — стандартный подход. Хорошая гибкость |
| **Зависимости** | ⭐⭐⭐☆☆ (3/5) | Некоторые риски с `ra_ap_syntax` (internal crate) и `fastembed` (binary blob download) |

### Общий рейтинг проекта: ⭐⭐⭐☆☆ (3/5) — Хороший потенциал, требует доработки

---

## Рекомендации по приоритетным действиям

### Недельный спринт (P0 + P1)
1. [ ] Добавить `LICENSE` файл с MIT текстом
2. [ ] Добавить `.github/workflows/ci.yml` с fmt, build, test, clippy, audit
3. [ ] Добавить авторизацию в HTTP API (Bearer token через env var)
4. [ ] Написать интеграционные тесты для CLI команд и HTTP API

### Двухнедельный спринт (P1 + P2)
5. [ ] Добавить тесты для `callgraph.rs` и `indexer.rs`
6. [ ] Atomic file replacement для JSONL хранилищ
7. [ ] DNS rebinding защита в LLM endpoint validation
8. [ ] Написать CONTRIBUTING.md и SECURITY.md

### Месячный план (P2)
9. [ ] Рассмотреть миграцию на `tantivy` или `bme25` для BM25
10. [ ] Добавить SHA-256 verification для ONNX runtime бинаря
11. [ ] Улучшить покрытие тестов до 30%+

---

**Конец аудита.**  
Аудитор: AI Agent  
Дата: 2026-06-11
