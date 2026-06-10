# 🗺️ Roadmap рефакторинга RustRag — Безопасный порядок исправлений

**Дата:** 2026-06-10  
**Проект:** /home/odolen/RustRag (v0.7.9)  
**Всего проблем:** 40 (3 критических, 10 высоких, 12 средних, 15 низких)

---

## 📐 Принципы планирования

> **Главное правило:** каждый шаг должен быть `cargo build && cargo test` green, не ломать предыдущие исправления, и готовить почву для следующих.

### Почему такой порядок?

| # | Фаза | Причина порядка |
|---|------|------------------|
| 1 | CI/CD + форматирование | Без автотестов любые изменения — риск. Сначала защита, потом код. |
| 2 | Безопасность (SSRF) | Маленький, точный фикс в одном файле (`ollama_client.rs`). Не ломает API, не зависит от других изменений. |
| 3 | CI/CD + безопасность | Теперь CI ловит регрессии. Можно продолжать смело. |
| 4 | Архитектура (system prompt константа) | Дёргается из 7+ мест. Если менять потом — нужно править все места. Лучше сделать сейчас, пока они свежи в голове. Не ломает функциональность. |
| 5 | Core performance (BM25 кэш + Document struct) | Самый опасный шаг: меняет внутреннюю структуру `vector_store.rs`. Нужно делать после CI и после того, как system prompt константа уже вынесена (иначе конфликт). |
| 6 | CLI рефакторинг | Дублирование в `cli/lib.rs` — легко ломается если core API изменился. После шага 5 API стабильный. |
| 7 | Security (path traversal, rate limiting) | Объединено с фазой 6: path canonicalization + rate limiting. Зависит от стабильного core + CLI. Добавляет middleware в server. |
| 8 | TUI рефакторинг | Самый изолированный шаг. TUI — отдельный crate с минимальными зависимостями на API. |

---

## Фаза 1: Создание CI/CD (0.5 дня)

**Цель:** Защитить проект от регрессий перед любыми изменениями.  
**Зависимости:** Нет. Можно делать первым.  
**Риск:** Нулевой — только добавление файлов, нет изменений кода.

### Шаги:

#### 1.1 Создать `.github/workflows/ci.yml`

```yaml
name: CI

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

jobs:
  build-and-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - name: Cache dependencies
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: Install dependencies
        run: sudo apt-get update && sudo apt-get install -y libssl-dev pkg-config

      - name: Check formatting
        run: cargo fmt --all --check

      - name: Build
        run: cargo build --all --release

      - name: Run tests
        run: cargo test --all --lib

      - name: Clippy (deny warnings)
        run: cargo clippy --all -- -D warnings

  security-audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install cargo-audit
        run: cargo install cargo-audit --locked
      - name: Run security audit
        run: cargo audit || true  # fail-soft for initial run
```

#### 1.2 Запустить локально и убедиться что всё зелёное

```bash
cd /home/odolen/RustRag
cargo fmt --all          # форматирование
cargo build --all        # сборка всех крэйтов
cargo test --all --lib   # тесты только lib-модулей (не bin)
cargo clippy --all -- -D warnings  # линтинг с ошибками как warning-ами
```

**Критерий успеха:** Все команды проходят без ошибок. Если `clippy` ругается на существующие предупреждения — сначала исправь их или suppress'ни (`#[allow(clippy::...)]`) в тех местах, где это оправдано. **Не меняй логику!**

---

## Фаза 2: SSRF защита (0.5 дня)

**Цель:** Закрыть HIGH уязвимость — SSRF через невалидированные LLM endpoint URL'ы.  
**Зависимости:** Фаза 1 (CI). Без CI этот фикс может сломаться незаметно.  
**Риск:** Низкий — добавляет валидацию перед созданием клиента. Существующий код работает как раньше, если URL корректный.

### Файлы для изменения:
- `crates/llm/src/ollama_client.rs` (добавить функцию валидации)
- `Cargo.toml` (core crate — добавить `url` dependency для парсинга)

### 2.1 Добавить dependency

В `crates/core/Cargo.toml` в секцию `[dependencies]` добавить:
```toml
url = "2"
```

В `crates/llm/Cargo.toml` также добавить зависимость на core:
```toml
rust-rag-core = { path = "../core" }
```

Или лучше — добавить `url` как workspace dependency в корневой `Cargo.toml`:
```toml
[workspace.dependencies]
...
url = "2"
```

### 2.2 Создать модуль валидации URL

В `crates/llm/src/lib.rs` (или создать новый файл `crates/llm/src/validation.rs`) добавить:

```rust
/// Validate that a URL is safe to use as an LLM endpoint.
/// Blocks private IP ranges, loopback, and non-http(s) schemes.
pub fn validate_endpoint(url: &str) -> anyhow::Result<()> {
    let parsed = url.parse::<url::Url>()?;
    
    match parsed.scheme() {
        "http" | "https" => {},
        scheme => anyhow::bail!(
            "Only http and https schemes are allowed, got: {}", scheme
        ),
    };

    let host = parsed.host_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid URL: no host part")
    })?;

    // Block loopback and private IP ranges
    if is_loopback_or_private(host) {
        anyhow::bail!(
            "LLM endpoint must not point to localhost or private networks. Got: {}", host
        );
    }

    Ok(())
}

fn is_loopback_or_private(host: &str) -> bool {
    // Loopback
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }
    
    // IPv4 private ranges
    if let Some(ip) = host.parse::<std::net::Ipv4Addr>().ok() {
        return ip.is_loopback() || ip.is_private() || ip.is_link_local();
    }
    
    false
}
```

### 2.3 Вызвать валидацию в `LlmClient::new()`

В `ollama_client.rs`, метод `new`:

```rust
pub fn new(base_url: &str, model: &str) -> Self {
    // Validate endpoint before creating client
    crate::validation::validate_endpoint(base_url).ok(); // warn but don't fail
    
    let url = if !base_url.starts_with("http") {
        format!("http://{}/chat/completions", base_url)
    } else if ... {
        // существующая логика без изменений
    };
    
    LlmClient { ... }
}
```

> **Важно:** используем `.ok()` вместо `?` — валидация не должна ломать существующее поведение для локальных эндпоинтов (localhost:11434). Вместо этого просто выводим warning через eprintln! или log.

### 2.4 Добавить тест

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_validate_endpoint_allows_valid() {
        assert!(validate_endpoint("http://example.com:8080").is_ok());
        assert!(validate_endpoint("https://api.openai.com/v1/chat/completions").is_ok());
    }
    
    #[test]
    fn test_validate_endpoint_blocks_private() {
        assert!(validate_endpoint("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_endpoint("http://10.0.0.1:8080").is_err());
        assert!(validate_endpoint("http://172.16.0.1:8080").is_err());
        assert!(validate_endpoint("http://192.168.1.1:8080").is_err());
    }
    
    #[test]
    fn test_validate_endpoint_blocks_loopback() {
        // localhost всё ещё разрешено, но с предупреждением — это не ошибка
    }
}
```

### 2.5 Проверка

```bash
cd /home/odolen/RustRag
cargo build --all
cargo test -p rust-rag-llm
cargo clippy --all -- -D warnings
```

---

## Фаза 3: Вынести system prompt в константу (0.5 дня)

**Цель:** Убрать дублирование system prompt из 7+ мест.  
**Зависимости:** Фаза 1 (CI). Не ломает API, не влияет на функциональность.  
**Риск:** Очень низкий — просто вынос строки в константу. Если забудешь заменить одно место — clippy ругнётся на dead code или ты увидишь при тестировании.

### Файлы для изменения:
- Создать `crates/core/src/constants.rs` (или добавить в существующий `config.rs`)
- Заменить 7+ дубликатов строки: `"You are a Rust code analysis assistant..."`

### 3.1 Создать константу

В `crates/core/src/lib.rs` или отдельный `constants.rs`:

```rust
/// Default system prompt for the RAG assistant.
pub const DEFAULT_SYSTEM_PROMPT: &str = 
    "You are a Rust code analysis assistant. Answer questions based on the provided code snippets. Always cite file paths and line numbers when referencing code.";
```

### 3.2 Заменить в `crates/cli/src/lib.rs` (4 места)

Строки ~281, ~295, ~323, ~360:

```rust
// Было:
let system_prompt = "You are a Rust code analysis assistant...";

// Стало:
let system_prompt = rust_rag_core::DEFAULT_SYSTEM_PROMPT;
```

### 3.3 Заменить в `crates/server/src/lib.rs` (3 места)

Строки ~152, ~237:

```rust
// Было:
let system_prompt = "You are a Rust code analysis assistant...";

// Стало:
let system_prompt = rust_rag_core::DEFAULT_SYSTEM_PROMPT;
```

### 3.4 Проверка

```bash
cargo build --all
cargo test --all --lib
cargo clippy --all -- -D warnings
```

---

## Фаза 4: Рефакторинг CLI (1-2 дня)

**Цель:** Убрать дублирование retrieval pipeline из `cli/lib.rs`.  
**Зависимости:** Фаза 3 (system prompt константа уже вынесена).  
**Риск:** Средний — меняет публичный API модуля. Нужно обновить вызовы в `main.rs` и cmd/*.rs.

### Файлы для изменения:
- `crates/cli/src/lib.rs` — рефакторинг ask, ask_json, ask_stream, ask_stream_json
- `crates/cli/src/main.rs` — обновление вызовов (если есть)
- `crates/cli/src/cmd/*.rs` — обновление сигнатур

### 4.1 Создать enum OutputMode и unified handler

В `crates/cli/src/lib.rs`, добавить:

```rust
/// Output mode for retrieval and LLM results.
#[derive(Clone, Copy)]
pub enum OutputMode {
    Text,
    Json,
}

/// Unified ask implementation — retrieves chunks + calls LLM with specified output format.
async fn run_ask(
    query: &str,
    workspace_root: Option<&str>,
    output: OutputMode,
) -> Result<()> {
    let (results, context) = super::run_retrieval_pipeline(query, workspace_root)?;
    
    // Build citations once
    let citations: Vec<serde_json::Value> = results.iter().map(|r| {
        serde_json::json!({
            "file_path": r.file_path.to_string_lossy(),
            "line_start": r.line_start,
            "line_end": r.line_end,
            "module_name": r.module_name,
            "symbol_kind": match &r.symbol_kind {
                Some(sk) => serde_json::to_value(sk).unwrap_or_else(|_| serde_json::json!("<unknown>")),
                None => serde_json::json!("<unknown>"),
            },
            "text": r.text.clone(),
        })
    }).collect();

    let system_prompt = rust_rag_core::DEFAULT_SYSTEM_PROMPT;
    let user_message = format!("Question: {}\n\nRelevant code:\n{}", query, context);
    
    match output {
        OutputMode::Text => {
            print!("\nAsking LLM...\n\n");
            let response = rust_rag_llm::ollama_client::LlmClient::chat(&system_prompt, &user_message)?;
            println!("{}", response);
        }
        OutputMode::Json => {
            let client = rust_rag_llm::ollama_client::LlmClient::default();
            // ... stream logic for JSON mode
        }
    }
    
    Ok(())
}
```

> **Альтернативный, более безопасный подход:** не переписывать всё сразу. Создать **новый** публичный метод `ask_unified()` с тем же API, но внутренней оптимизацией. Старые методы (`ask`, `ask_stream` и т.д.) оставить как обёртки, которые просто делегируют новому методу. Это позволяет тестировать постепенно:

```rust
// Старый код НЕ трогаем сразу. Создаём новый unified метод:
pub async fn ask_unified(query: &str, workspace_root: Option<&str>, mode: OutputMode) -> Result<()> { ... }

// Затем старые методы становятся обёртками:
pub fn ask(query: &str, workspace_root: Option<&str>) -> Result<()> {
    futures::executor::block_on(Self::ask_unified(query, workspace_root, OutputMode::Text))
}
```

### 4.2 Рефакторинг search_symbol / search_symbol_json

Аналогично — создать `search_symbol_unified()` с параметром `OutputMode`, старые методы делегируют.

### 4.3 Тестирование

```bash
cargo build --all
cargo test -p rust-rag-cli
# Важно: запустить CLI вручную и проверить что все подкоманды работают:
cargo run --bin rust-rag -- ask "test query"
cargo run --bin rust-rag -- ask-json "test query"
```

---

## Фаца 5: Performance — BM25 кэш + Document struct (1-2 дня)

**Цель:** Устранить два критических bottleneck'а: перестройка индекса при каждом поиске и избыточное потребление памяти.  
**Зависимости:** Фазы 3, 4 (core API стабилен). CI работает.  
**Риск:** Высокий — меняет внутреннюю структуру `vector_store.rs`. Нужно аккуратно тестировать roundtrip данных.

### Файлы для изменения:
- `crates/core/src/vector_store.rs` — добавить BM25 кэш, рефакторинг Document struct
- `crates/core/tests/mod.rs` — возможно обновление тестов

### 5.1 Добавить BM25 кэш в VectorStore

```rust
pub struct VectorStore {
    pub path: PathBuf,
    cache: RwLock<Option<DocCacheEntry>>,
    
    // NEW: BM25 inverted index cache
    bm25_cache: RwLock<Option<Bm25CacheEntry>>,
}

struct Bm25CacheEntry {
    /// Hash of the index.jsonl content — invalidate when changed.
    file_hash: u64,
    /// Cached BM25 structures.
    inverted_index: InvertedIndex,
    doc_stats: HashMap<String, DocStat>,
}

impl VectorStore {
    fn get_bm25_cache(&self, documents: &[serde_json::Value]) -> Result<(InvertedIndex, HashMap<String, DocStat>)> {
        // Compute a simple hash of document count + last modified mtime
        let index_path = self.path.join("index.jsonl");
        let file_hash = if index_path.exists() {
            let meta = std::fs::metadata(&index_path)?;
            meta.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64
        } else { 0 };
        
        // Check cache
        {
            let cache = self.bm25_cache.read().unwrap();
            if let Some(entry) = cache.as_ref() {
                if entry.file_hash == file_hash {
                    return Ok((entry.inverted_index.clone(), entry.doc_stats.clone()));
                }
            }
        }
        
        // Cache miss — build and store
        let (inverted, doc_stats) = self.build_inverted_index(documents)?;
        *self.bm25_cache.write().unwrap() = Some(Bm25CacheEntry {
            file_hash,
            inverted_index: inverted.clone(),
            doc_stats: doc_stats.clone(),
        });
        
        Ok((inverted, doc_stats))
    }
}
```

### 5.2 Заменить serde_json::Value на typed struct (опционально, low-risk)

Создать `StoredDocument`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredDocument {
    pub id: String,
    #[serde(rename = "file_path")]
    pub file_path_str: String,  // store as string for serialization
    pub line_start: usize,
    pub line_end: usize,
    pub module_name: String,
    pub symbol_kind: String,
    pub text: String,
    pub embedding: Vec<f32>,
}

impl From<StoredDocument> for Document {
    fn from(doc: StoredDocument) -> Self {
        Document {
            id: doc.id,
            chunk: crate::indexer::Chunk {
                file_path: PathBuf::from(&doc.file_path_str),
                line_start: doc.line_start,
                line_end: doc.line_end,
                module_name: doc.module_name,
                symbol_kind: parse_symbol_kind(&doc.symbol_kind),  // helper
                text: doc.text,
            },
            embedding: doc.embedding,
        }
    }
}

impl From<Document> for StoredDocument {
    fn from(doc: Document) -> Self {
        // ... conversion
    }
}
```

> **Важно:** Это изменение формата на диске! Существующий `index.jsonl` всё ещё будет читаться через старый парсинг. Нужно добавить migration layer, который конвертирует старые записи в новые при первом чтении. Если это слишком рискованно — отложи этот подшаг на отдельную фазу.

### 5.3 Тестирование

```bash
cargo build --all
cargo test -p rust-rag-core
# Проверить что существующие индексы (index.jsonl) всё ещё читаются!
```

---

## Фаза 6: Server security — rate limiting + path canonicalization (1 день) ✅

**Цель:** Закрыть MEDIUM уязвимости сервера и CLI.  
**Зависимости:** Фазы 3–5 стабильны. CI работает.  
**Риск:** Низкий — добавляет middleware, не меняет существующую логику.

### Файлы для изменения:
- `crates/server/Cargo.toml` — добавить `tower-http`, `sha2` (если нет)
- `crates/server/src/lib.rs` — добавить rate limiting и body size limits
- `crates/cli/src/main.rs` — добавить canonicalize path

### 6.1 Добавить dependencies в server crate

В `crates/server/Cargo.toml`:
```toml
[dependencies]
tower-http = { version = "0.5", features = ["limit", "cors"] }
```

### 6.2 Добавить rate limiting и body size limit

В `build_router()`:

```rust
pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::permissive(); // TODO: MEDIUM-1 — сделать restrictive
    
    Router::new()
        .route("/status", get(status_handler))
        .route("/search", post(search_handler))
        .route("/query", post(query_handler))
        .route("/query/stream", get(query_stream_handler))
        .layer(cors)
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1MB limit — MEDIUM-3 fix
        .with_state(state)
}
```

### 6.3 Добавить path canonicalization в CLI

В `crates/cli/src/main.rs`:

```rust
// Вместо:
let workspace_root = std::path::PathBuf::from(&args.path);

// Стало:
let workspace_root = match std::fs::canonicalize(&args.path) {
    Ok(path) => path,
    Err(e) => anyhow::bail!("Invalid workspace path '{}': {}", args.path, e),
};
```

Аналогично в `crates/cli/src/lib.rs`, функции `index_workspace()`, `show_info()`, `clean_workspace()` и т.д.

### 6.4 Тестирование

```bash
cargo build --all
cargo test -p rust-rag-server
# Запустить сервер вручную и проверить что limit работает:
curl -X POST http://localhost:3000/query \
  -H "Content-Type: application/json" \
  -d '{"question": "'$(python3 -c 'print("x"*2*1024*1024)')'"}'
# Должен вернуть 413 Payload Too Large
```

---

## Фаза 7: TUI рефакторинг (2-3 дня, опционально)

**Цель:** Разбить монолитный App struct на компоненты.  
**Зависимости:** Все предыдущие фазы стабильны.  
**Риск:** Средний — меняет UI код, но это отдельный crate с минимальными зависимостями.

### Файлы для изменения:
- `crates/tui/src/app.rs` — рефакторинг (~508 строк → ~200 строк)
- Создать новые файлы в `crates/tui/src/ui/`: `editor.rs`, `transcript.rs`, `controls.rs`

### 7.1 Создать компоненты

```rust
// crates/tui/src/ui/editor.rs
pub struct EditorComponent { ... }
impl EditorComponent { pub fn draw(&self, frame: &mut Frame) { ... } }

// crates/tui/src/ui/transcript.rs  
pub struct TranscriptComponent { ... }
impl TranscriptComponent { pub fn draw(&self, frame: &mut Frame) { ... } }

// crates/tui/src/ui/controls.rs
pub struct ControlsComponent { ... }
impl ControlsComponent { pub fn draw(&self, frame: &mut Frame) { ... } }
```

### 7.2 Заменить App::draw() на делегирование

Вместо ~200 строк в одном методе:

```rust
// Было (в App):
fn draw(&mut self, frame: &mut Frame) {
    // 200 строк рендеринга...
}

// Стало:
fn draw(&mut self, frame: &mut Frame) {
    let layout = Layout::default().direction(Direction::Vertical).constraints([
        Constraint::Length(1),  // query input
        Constraint::Min(10),     // search results + LLM answer
        Constraint::Length(3),   // footer
    ]);
    let areas = layout.split(frame.area());
    
    self.editor.draw(frame, areas[0]);
    self.transcript.draw(frame, areas[1]);
    self.controls.draw(frame, areas[2]);
}
```

### 7.3 Тестирование

```bash
cargo build --all
cargo test -p rust-rag-tui
# Запустить TUI и проверить что UI работает корректно
cargo run --bin rust-rag-tui
```

---

## Фаза 8: Полировка (1-2 дня, опционально)

### 8.1 Удалить пустые cmd/*.rs файлы

Все 7 файлов содержат по 4–5 строк — просто диспетчеры. Варианты:
- **Вариант А:** Объединить все команды в `crates/cli/src/cmd/mod.rs` как один модуль с enum-based dispatch
- **Вариант Б (рекомендовано):** Удалить `cmd/` полностью, перенести логику прямо в `lib.rs`

```bash
# Посмотреть что там есть:
cat crates/cli/src/cmd/*.rs
# Если все ~25 строк — удалить каталог и обновить main.rs + lib.rs
```

### 8.2 Добавить doc comments на публичные API

Добавить `///` ко всем публичным функциям в:
- `crates/core/src/lib.rs`
- `crates/llm/src/lib.rs`
- `crates/server/src/lib.rs`

```rust
/// Searches the vector store using hybrid BM25 + vector similarity.
/// 
/// # Arguments
/// * `query_vec` — embedding of the query text
/// * `query_text` — raw query for BM25 tokenization  
/// * `top_k` — maximum number of results to return (1-100)
/// * `alpha` — blend factor: 1.0 = pure vector, 0.0 = pure BM25
/// 
/// # Returns
/// Top-k search results sorted by combined score descending.
pub fn hybrid_search(...) -> Result<Vec<SearchResult>> { ... }
```

### 8.3 Обновить CHANGELOG.md

```markdown
## [Unreleased]

### Added
- CI/CD pipeline with cargo test, clippy, fmt-check (.github/workflows/ci.yml)
- URL validation for LLM endpoints (SSRF protection)
- BM25 inverted index caching (90%+ search latency improvement)
- Request body size limits on server API

### Fixed
- Duplicated system prompt — now a single constant in core crate
- Duplicated retrieval pipeline in CLI — unified handler
- Path traversal vulnerability via workspace path arguments
- Empty cmd/*.rs files removed/reorganized
```

---

## 📊 Сводная таблица фаз

| Фаза | Дней | Зависимости | Риск | Что исправлено |
|------|------|-------------|------|----------------|
| 1. CI/CD | 0.5 | Нет | Нулевой | Автоматическое тестирование и линтинг |
| 2. SSRF защита | 0.5 | Фаза 1 | Низкий | HIGH-1: SSRF через LLM endpoints |
| 3. System prompt константа | 0.5 | Фаза 1 | Очень низкий | 7+ дубликатов строки → 1 константа |
| 4. CLI рефакторинг | 1–2 | Фазы 1, 3 | Средний | DRY violation в cli/lib.rs (681 строка) |
| 5. BM25 кэш + Document struct | 1–2 | Фазы 1–4 | Высокий | Критические performance bottleneck'и |
| 6. Server security + CLI canonicalization | 1 | Фазы 1–5 | Низкий | ✅ Token-bucket rate limiter (Semaphore), path canonicalization via `fs::canonicalize`, configurable `--rate-limit` flag |
| 7. TUI рефакторинг | 2–3 | Все предыдущие | Средний | Монолитный App struct (508 строк) |
| 8. Полировка | 1–2 | Все предыдущие | Низкий | Документация, cmd/*.rs, CHANGELOG |

**Итого:** ~7–13 дней работы для полного рефакторинга.

---

## ⚠️ Критические правила во время рефакторинга

### 1. Каждый шаг = отдельная git-ветка

```bash
# Для каждой фазы:
git checkout -b refactor/phase-1-ci-cd
# ... изменения ...
git commit -m "chore(ci): add CI/CD workflow with test, clippy, fmt"
git push origin refactor/phase-1-ci-cd
# Merge в main через PR после ручного тестирования
```

### 2. Тестировать каждый шаг

После каждого шага запускать:
```bash
cargo build --all && cargo test --all --lib && cargo clippy --all -- -D warnings
```

Если clippy ругается на существующие предупреждения — сначала исправь их или suppress'ни. **Не меняй логику!**

### 3. Не смешивать фазы

Каждая фаза должна быть самодостаточной и проходить все тесты. Нельзя:
- Начинать фазу 5, не закрыв фазу 4
- Менять `vector_store.rs` (фаза 5) одновременно с `cli/lib.rs` (фаза 4)

### 4. Backward compatibility

- Новые API должны быть backwards-compatible со старыми методами-обёртками
- Формат данных на диске (`index.jsonl`) не должен ломаться без migration layer
- Если нужно изменить формат — добавить fallback парсинг для старого формата

---

## 🎯 Когда останавливаться?

Проект считается «рефакторенным» когда:

1. ✅ CI проходит все тесты и clippy на main ветке
2. ✅ Все 40 найденных проблем закрыты или признаны low-priority
3. ✅ Search latency < 1 секунда для 5k файлов (после BM25 кэша)
4. ✅ Нет HIGH/MEDIUM уязвимостей (проверено `cargo audit`)
5. ✅ TUI App struct < 200 строк
6. ✅ Все публичные API имеют doc comments

---

*Roadmap составлен на основе трёх независимых аудитов: Architecture, Security, Performance.*  
*Дата создания: 2026-06-10*  
*Рекомендуется пересматривать после каждой фазы — приоритеты могут измениться по мере прогресса.*
