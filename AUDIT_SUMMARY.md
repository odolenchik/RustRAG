# 📊 Итоговый аудит проекта RustRag — Сводный отчёт

**Дата:** 2026-06-10  
**Проект:** /home/odolen/RustRag (v0.7.9)  
**Тип:** RAG (Retrieval Augmented Generation) для кода на Rust  
**Аудиторы:** 3 специализированных агента (Architecture, Security, Performance)

---

## 📑 Структура аудита

| Файл | Содержание | Объём |
|------|-----------|-------|
| `AUDIT_ARCHITECTURE.md` | Архитектура, структура кода, качество модулей, CI/CD | 204 строки |
| `AUDIT_SECURITY.md` | Уязвимости, безопасность, input validation, network security | 446 строк |
| `AUDIT_PERFORMANCE.md` | Производительность, memory leaks, algorithmic complexity, I/O optimization | 597 строк |
| **Этот файл** | Сводка всех находок и приоритизированный план исправлений | — |

---

## 🎯 Итоговые оценки

| Категория | Оценка | Статус |
|-----------|--------|--------|
| **Архитектура и проектирование** | 9/10 ✅ | Отличная workspace структура, чёткие границы между крэйтами |
| **Качество core** | 8/10 ✅ | Solid BM25 + vector hybrid search, good incremental indexing |
| **Качество CLI** | 7/10 ⚠️ | Дублирование кода (4x retrieval pipeline), пустые cmd/*.rs файлы |
| **Качество LLM** | 8/10 ✅ | Clean trait abstraction, good SSE parsing |
| **Качество Server** | 7/10 ⚠️ | Solid MCP implementation, но large handlers и duplicate prompts |
| **Качество TUI** | 6/10 ⚠️ | Monolithic App struct (508 строк), hardcoded цвета, legacy threading |
| **Безопасность** | MEDIUM RISK ⚠️ | 1 High, 6 Medium, 8 Low уязвимостей |
| **Производительность** | FAIR (3/5) ⚠️ | Критические bottleneck'и при работе с большими кодовыми базами (>5k файлов) |
| **Тестирование** | 9/10 ✅ | 40 тестов с отличным покрытием edge cases |
| **CI/CD** | 3/10 ❌ | Пустой .github/workflows/ — нет автоматизации |

### 🏆 Общий балл: **7.0 / 10** (⭐⭐⭐⭐⭐⭐☆☆☆)

---

## 🔥 Топ-10 Критических Проблем (приоритет по severity)

### 🔴 CRITICAL — Необходимо исправить немедленно

| # | Проблема | Категория | Где | Влияние |
|---|----------|-----------|-----|---------|
| 1 | **BM25 inverted index rebuilds per query** | Performance | `core/vector_store.rs:228` | Поиск занимает 2-15 секунд для 5k файлов, 30+ сек для больших |
| 2 | **Full JSONL re-parse с serde_json::Value** | Performance | `core/vector_store.rs:133-174` | Пиковое потребление памяти 2.5-5 GB при поиске |
| 3 | **SSRF via unvalidated LLM endpoint URLs** | Security | `llm/ollama_client.rs:117` | Атакующий может отправлять запросы к внутренним сервисам |

### 🟠 HIGH — Высокий приоритет

| # | Проблема | Категория | Где | Влияние |
|---|----------|-----------|-----|---------|
| 4 | **O(n²) Vec::contains в callgraph parsing** | Performance | `core/callgraph.rs:103` | Линейно растущая задержка при увеличении количества чанков |
| 5 | **Unnecessary Chunk clones + redundant byte_offsets** | Performance | `core/indexer.rs:74-221` | Избыточное копирование памяти при индексации |
| 6 | **No request rate limiting / input size limits** | Security | `server/lib.rs:58-67` | DoS через resource exhaustion |
| 7 | **Path traversal в workspace paths** | Security | `cli/main.rs:118`, `cli/lib.rs:66` | Случайное удаление файлов вне intended scope |
| 8 | **No authentication on any API endpoint** | Security | Все server handlers | Любой с сетевым доступом может использовать RAG систему |

### 🟡 MEDIUM — Средний приоритет

| # | Проблема | Категория | Где | Влияние |
|---|----------|-----------|-----|---------|
| 9 | **Дублирование retrieval pipeline 4 раза** | Architecture | `cli/lib.rs` (681 строка) | DRY violation, сложно поддерживать |
| 10 | **No TLS/HTTPS support on server** | Security | `server/bin.rs:52`, `llm/ollama_client.rs:161` | Plaintext HTTP для всех запросов |
| 11 | **Monolithic TUI App struct (508 строк)** | Architecture | `tui/app.rs` | Нарушает SRP, сложно расширять |
| 12 | **Empty cmd/*.rs файлы (5 строк каждый)** | Architecture | Все 7 файлов в `cmd/` | Добавляют косвенность без ценности |
| 13 | **Embedding cache full rewrite on incremental index** | Performance | `core/embedding.rs:284-297` | Избыточная запись на диск |
| 14 | **No CI/CD workflow** | Architecture | `.github/workflows/` пустой | Нет автоматического тестирования и линтинга |

---

## 📋 Детальные находки по категориям

### Архитектура — Найдено 15 проблем

**Положительные моменты:**
- ✅ Чистая Cargo workspace с `resolver = "2"`
- ✅ Ацикличный граф зависимостей: `core ← llm ← cli`, `server ← core, llm`
- ✅ `[workspace.dependencies]` для общих зависимостей
- ✅ 40 passing tests с отличным coverage edge cases

**Треуют исправления:**
- ❌ Дублирование retrieval pipeline 4 раза в `cli/lib.rs` (~270 строк дубликата)
- ❌ Пустые обёртки в `cmd/*.rs` (5 файлов по 4-5 строк)
- ❌ Монолитный TUI App struct нарушает single responsibility principle
- ❌ Hardcoded system prompt в 7+ местах вместо константы
- ❌ Специфичный default model name: `"Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf"`

### Безопасность — Найдено 15 уязвимостей (1 High, 6 Medium, 8 Low)

**Критические:**
- 🔴 **SSRF**: Нет валидации URL для LLM endpoints — можно отправить запрос на `http://169.254.169.254/` или внутренние сервисы
- 🟡 **CORS**: `CorsLayer::permissive()` позволяет любой origin
- 🟡 **No TLS**: Сервер работает только по HTTP, нет поддержки HTTPS
- 🟡 **No rate limiting**: Нет ограничений на размер запроса и частоту вызовов
- 🟡 **Path traversal**: Пользовательские пути не канонизируются
- 🟡 **No auth**: Ни один endpoint не требует аутентификации
- 🟡 **Unverified model downloads**: ONNX модели скачиваются без SHA-256 checksum

**Положительные моменты:**
- ✅ Нет shell injection векторов (все операции через Rust stdlib)
- ✅ Atomic file operations (temp + rename pattern)
- ✅ TLS для outbound connections (`rustls-tls` в core crate)
- ✅ No hardcoded secrets или credentials
- ✅ JSON Schema validation в MCP tools

### Производительность — Найдено 10 bottleneck'ов

**Критические:**
- 🔴 **BM25 inverted index rebuilds per query**: O(D × T) где D = документы, T = tokens. Каждый поиск перестраивает индекс заново!
- 🔴 **JSONL parse с serde_json::Value**: 2.5-5 GB peak memory для 50k документов
- 🟠 **O(n²) Vec::contains в callgraph.rs**: Линейно растущая задержка

**Высокий приоритет:**
- 🟠 **Unnecessary Chunk clones**: Избыточное копирование при overlap алгоритме (~150 строк)
- 🟡 **New reqwest::Client per LLM request**: Нет connection pooling
- 🟡 **std::thread::spawn per TUI query**: ~2MB на поток, un-cancellable

**Низкий приоритет:**
- 🟢 **Embedding cache full rewrite**: Полная перезапись при incremental index
- 🟢 **Symbol search parse entire JSONL**: Для простого substring match

---

## 🛠 Приоритизированный план исправлений

### Phase 1: Immediate (1-2 недели) — Критические проблемы

#### 🔴 P0: Исправить performance bottleneck'и
```rust
// 1. Персистентный BM25 inverted index (кэшируется на диск и в памяти)
// core/vector_store.rs
pub struct VectorStore {
    pub path: PathBuf,
    doc_cache: RwLock<Option<DocCacheEntry>>,
    bm25_index: RwLock<Option<PersistedInvertedIndex>>, // NEW
}

// 2. Custom Document struct вместо serde_json::Value (уменьшает память на 60%)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub id: String,
    pub file_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub module_name: String,
    pub symbol_kind: String,
    pub text_content: String,
    pub embedding: Vec<f32>,
    pub byte_offsets: (usize, usize),
}

// 3. SSRF protection для LLM endpoints
fn validate_url(url: &str) -> Result<()> {
    let parsed = url.parse::<url::Url>()?;
    match parsed.scheme() {
        "http" | "https" => {},
        _ => return Err(anyhow!("Only http and https schemes allowed")),
    };
    // Block private IP ranges
    let host = parsed.host_str().ok_or_else(|| anyhow!("Invalid host"))?;
    if is_private_ip(host) {
        return Err(anyhow!("Private IP addresses are not allowed"));
    }
    Ok(())
}
```

#### 🔴 P0: Добавить CI/CD workflow
```yaml
# .github/workflows/ci.yml
name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --all
      - run: cargo test --all
      - run: cargo clippy --all -- -D warnings
      - run: cargo fmt --all --check
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install cargo-audit
      - run: cargo audit
```

---

### Phase 2: Short-term (1 месяц) — Высокий приоритет

#### 🟠 P1: Рефакторить CLI lib.rs
```rust
// Вместо 4 дублирующихся функций → одна с enum output mode
enum OutputMode {
    Text,
    Stream,
    Json,
    StreamJson,
}

async fn execute_retrieval(
    config: &Config,
    query: &str,
    output: OutputMode,
) -> Result<()> {
    let store = VectorStore::open(&config.get_store_path())?;
    let results = store.search_by_text(query, 5)?;
    
    match output {
        OutputMode::Text => print_results_text(&results),
        OutputMode::Stream => stream_results(results).await,
        OutputMode::Json => println!("{}", serde_json::to_string_pretty(&results)?),
        OutputMode::StreamJson => stream_json_results(results).await,
    }
}
```

#### 🟠 P1: Добавить authentication и rate limiting на server
```rust
// server/lib.rs — добавить middleware
use axum::{middleware::from_fn_with_state, Router};
use tower_http::limit::RequestBodyLimitLayer;
use std::sync::{Arc, Mutex};

struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<std::time::Instant>>>>,
}

impl RateLimiter {
    fn is_allowed(&self, ip: &str) -> bool {
        // 100 requests per minute per IP
        let mut requests = self.requests.lock().unwrap();
        let now = std::time::Instant::now();
        requests.entry(ip.to_string())
            .or_default()
            .retain(|t| now.duration_since(*t).as_secs() < 60);
        let count = requests.get_mut(ip).map(|v| v.len()).unwrap_or(0);
        count < 100
    }
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/status", get(status_handler))
        .route("/search", post(search_handler))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1MB limit
        .layer(from_fn_with_state(state.clone(), rate_limit_middleware))
}
```

#### 🟠 P1: Исправить security vulnerabilities
```rust
// MEDIUM-4: Path traversal protection
let workspace_root = std::fs::canonicalize(&args.path)?;
if !workspace_root.starts_with(expected_parent) {
    return Err(anyhow!("Workspace path must be within expected directory"));
}

// MEDIUM-6: SHA-256 checksum verification для model downloads
fn download_model_with_checksum(target: &Path, url: &str, expected_sha256: &str) -> Result<()> {
    let bytes = client.get(url).send()?.bytes()?;
    let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
    if hash != expected_sha256 {
        return Err(anyhow!("Checksum mismatch: expected {}, got {}", expected_sha256, hash));
    }
    std::fs::write(target, &bytes)?;
    Ok(())
}
```

---

### Phase 3: Medium-term (2-3 месяца) — Средний приоритет

#### 🟡 P2: Рефакторить TUI для разделения ответственности
```rust
// Разбить App struct (~508 строк) на отдельные компоненты:
struct AppComponent {
    editor: EditorComponent,
    transcript: TranscriptComponent,
    controls: ControlsComponent,
}

impl AppComponent {
    fn draw(&self, frame: &mut Frame) {
        self.editor.draw(frame);
        self.transcript.draw(frame);
        self.controls.draw(frame);
    }
}

// Добавить theme system
pub struct Theme {
    pub colors: ColorPalette,
    pub styles: StyleMap,
}

impl AppComponent {
    fn draw_with_theme(&self, frame: &mut Frame, theme: &Theme) {
        // Использовать theme.colors вместо hardcoded Color::White/Blue
    }
}
```

#### 🟡 P2: Удалить пустые cmd/*.rs файлы
```rust
// Вариант 1: Объединить все команды в одну struct
mod cmd {
    pub mod index {
        pub async fn run(args: &IndexArgs) -> Result<()> {
            // Реальная логика, а не просто диспетчер
        }
    }
    
    pub mod ask {
        pub async fn run(args: &AskArgs) -> Result<()> {
            // ...
        }
    }
}

// Вариант 2: Убрать cmd/ модуль полностью, вызывать напрямую из lib.rs
```

#### 🟡 P2: Добавить documentation и doc comments
```rust
/// Searches the vector store using hybrid BM25 + vector similarity.
/// 
/// # Arguments
/// * `query` - The search query string
/// * `top_k` - Maximum number of results to return (1-100)
/// 
/// # Examples
/// ```
/// let store = VectorStore::open(&path)?;
/// let results = store.search_by_text("function parser", 5)?;
/// assert!(results.len() <= 5);
/// ```
pub fn search_by_text(&self, query: &str, top_k: usize) -> Result<Vec<SearchResult>> {
    // ...
}
```

---

### Phase 4: Long-term (3+ месяцев) — Низкий приоритет, но важное

#### 🟢 P3: Оптимизация concurrency и parallelism
```rust
// Использовать tokio mpsc вместо std::thread в TUI
use tokio::sync::mpsc;

async fn handle_query(
    tx: mpsc::Sender<Chunk>,
    query: String,
) -> Result<()> {
    let store = VectorStore::open(&config.get_store_path())?;
    let results = store.search_by_text(&query, 5)?;
    
    for chunk in results {
        tx.send(chunk).await?;
    }
    Ok(())
}

// Параллельная обработка чанков при индексации
use tokio::task::JoinSet;

async fn index_workspace_parallel(workspace: &Path) -> Result<Vec<Chunk>> {
    let mut set = JoinSet::new();
    for entry in walkdir::WalkDir::new(workspace) {
        let path = entry?.path().to_path_buf();
        set.spawn_blocking(move || {
            parse_file(&path)
        });
    }
    
    let mut all_chunks = Vec::new();
    while let Some(res) = set.join_next() {
        all_chunks.extend(res??);
    }
    Ok(all_chunks)
}
```

#### 🟢 P3: Добавить integration tests и benchmarks
```rust
// tests/performance_benchmarks.rs
#[cfg(test)]
mod performance_tests {
    use std::time::Instant;
    
    #[test]
    fn test_search_latency_under_1s() {
        let store = create_test_store(5000); // 5k files
        let start = Instant::now();
        let results = store.search_by_text("parser", 10).unwrap();
        let elapsed = start.elapsed();
        
        assert!(elapsed.as_secs_f64() < 1.0, 
                "Search took {}s, expected < 1s", elapsed.as_secs_f64());
        assert!(results.len() <= 10);
    }
    
    #[test]
    fn test_index_memory_under_500mb() {
        let workspace = create_large_workspace(10000); // 10k files
        let start_mem = get_process_memory_mb();
        
        index_workspace(&workspace).unwrap();
        
        let end_mem = get_process_memory_mb();
        let memory_used = end_mem - start_mem;
        
        assert!(memory_used < 500, 
                "Indexing used {}MB, expected < 500MB", memory_used);
    }
}
```

---

## 📊 Сводная таблица всех находок

| Категория | Найдено проблем | Критических | Высокий приоритет | Средний | Низкий |
|-----------|-----------------|-------------|-------------------|---------|--------|
| Архитектура | 15 | 0 | 4 | 6 | 5 |
| Безопасность | 15 | 1 | 3 | 2 | 9 |
| Производительность | 10 | 2 | 3 | 4 | 1 |
| **ИТОГО** | **40** | **3** | **10** | **12** | **15** |

---

## 🎯 Ключевые метрики проекта

| Метрика | Текущее значение | Целевое значение | Статус |
|---------|------------------|------------------|--------|
| Lines of code (Rust) | ~2,300 | < 3,000 | ✅ Хорошо |
| Passing tests | 40 | > 50 | ⚠️ Требуется больше |
| Code coverage | Неизвестно | > 80% | ❌ Не измерено |
| CI/CD workflow | 0 (пусто) | ≥ 1 (test + lint) | ❌ Критично |
| Security vulnerabilities | 15 (1 High, 6 Med, 8 Low) | 0 High, < 3 Medium | ⚠️ Требует работы |
| Search latency (5k files) | 2-15 секунд | < 1 секунда | ❌ Требуется оптимизация |
| Peak memory (indexing 5k files) | Неизвестно | < 500 MB | ⚠️ Нужно измерить |

---

## 💡 Общие рекомендации

### Что делать СЕЙЧАС (эта неделя):
1. ✅ Создать `.github/workflows/ci.yml` с `cargo test`, `cargo clippy`, `cargo fmt --check`
2. 🔴 Добавить URL validation для LLM endpoints (SSRF fix)
3. 🔴 Прокешировать BM25 inverted index на диск и в память

### Что делать В ЭТОМ МЕСЯЦЕ:
4. 🟠 Рефакторить `cli/lib.rs` — вынести общий retrieval pipeline в одну функцию
5. 🟠 Добавить authentication и rate limiting на server API
6. 🟠 Исправить path traversal и добавить SHA-256 checksum verification

### Что делать В СЛЕДУЮЩЕМ МЕСЯЦЕ:
7. 🟡 Разбить монолитный TUI App struct на компоненты
8. 🟡 Удалить пустые `cmd/*.rs` файлы или наполнить смыслом
9. 🟡 Добавить `///` doc comments на публичные API

### Что делать В БУДУЩЕМ:
10. 🟢 Оптимизировать concurrency (tokio mpsc вместо std::thread)
11. 🟢 Добавить integration tests и benchmarks
12. 🟢 Добавить theme system в TUI (dark/light mode)
13. 🟢 Рассмотреть ANN library (FAISS, HNSWlib) для масштабирования vector search

---

## ✅ Положительные моменты проекта

Проект демонстрирует множество сильных сторон:

- ✅ **Отличная workspace структура** — чёткие границы между крэйтами, ацикличные зависимости
- ✅ **Solid testing foundation** — 40 passing tests с good edge case coverage
- ✅ **Atomic file operations** — temp + rename pattern предотвращает corruption
- ✅ **Hybrid search** — BM25 + vector similarity работает корректно
- ✅ **Incremental indexing** — SHA-256 hash comparison для эффективных обновлений
- ✅ **Clean LLM abstraction** — trait `ChatBackend` с good default method implementations
- ✅ **MCP protocol implementation** — полная поддержка JSON-RPC 2.0, batch requests
- ✅ **No shell injection vectors** — все операции через Rust stdlib
- ✅ **TLS для outbound connections** — rustls-tls enabled
- ✅ **Zero TODO/FIXME/HACK/XXX comments** — проект хорошо ухожен

---

## 📝 Заключение

RustRag — это функциональный RAG инструмент с чистой архитектурой и хорошей тестовой базой. Основные проблемы лежат в трёх областях:

1. **Производительность**: BM25 индекс перестраивается при каждом поиске, что критично для больших кодовых баз (>5k файлов). Кэширование на диск и в память решит 90% проблем.
   
2. **Безопасность**: Server component имеет несколько уязвимостей (SSRF, CORS, no auth), которые становятся критичными при любой сетевой экспозиции. Для локального использования на `127.0.0.1` риски минимальны.

3. **Архитектура**: CLI и TUI модули требуют рефакторинга для разделения ответственности (DRY, SRP principles). Пустые обёртки в `cmd/*.rs` добавляют косвенность без ценности.

**Рекомендуемый next step:** Начать с Phase 1 — создать CI/CD workflow, исправить SSRF и закэшировать BM25 индекс. Эти три изменения поднимут качество проекта с "FAIR" до "GOOD".

---

*Аудит выполнен автоматически тремя специализированными агентами-исследователями*  
*Дата завершения: 2026-06-10*  
*Все детали доступны в соответствующих AUDIT_*.md файлах*
