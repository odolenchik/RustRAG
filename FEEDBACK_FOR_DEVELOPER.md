# Feedback for RustRAG Developer

**From:** Real-world agent user (Kimi Code / AI coding agent)  
**Project analyzed:** swarmx — 7-crate Rust workspace (~642 code chunks, ~80+ files)  
**Usage pattern:** MCP stdio tool integrated into AI agent workflow, called repeatedly across a session  

---

## Overall Assessment

RustRAG is the best local semantic search tool for Rust workspaces I've used. AST-aware chunking + hybrid BM25+vector search works very well out of the box. The index builds correctly, incremental updates are fast, and results are genuinely useful for understanding unfamiliar codebases.

That said, the MCP integration has friction points that make it feel clunkier than it needs to be. Below are actionable suggestions ordered by impact.

---

## 1. Persistent Server Mode (High Impact)

**Problem:** Every tool call spawns a new subprocess (`rust-rag-serve mcp /workspace`). This means:
- ~200–500ms process startup overhead per call
- No shared state between calls beyond the on-disk index
- Each invocation re-parses config and reloads the embedding model if not cached

**Suggestion:** Add a `--persistent` or `--daemon` mode that:
- Keeps the MCP server alive after receiving `initialize`/first tool call
- Exposes a fixed port (e.g., via WebSocket or HTTP) for multiple agent sessions to connect
- Or keeps stdio open until EOF is explicitly sent

**Why it matters:** AI agents typically make 5–20 searches per session. Spawning 20 subprocesses adds 1–4 seconds of wasted time on a single query cycle.

---

## 2. Structured JSON Output for Tool Results (High Impact)

**Problem:** `rag_search` returns results as formatted text with manually-parsed scores, paths, and snippets:
```
[1] Score: 0.730 | /path/to/file.rs:10628
fn test_hash_to_16_deterministic() { ... }
```

The agent must regex-parse this to extract the file path, line number, snippet text, and score per result. Error-prone if formatting changes slightly.

**Suggestion:** Add a `--json` flag or make JSON output the default:
```json
{
  "results": [
    {
      "id": "swarm-core/src/main.rs:10628",
      "file_path": "swarm-core/src/main.rs",
      "line_start": 10628,
      "line_end": 10635,
      "score": 0.730,
      "vector_score": 0.71,
      "bm25_score": 0.75,
      "module_name": "test_hash_to_16_deterministic",
      "symbol_kind": "Function",
      "text": "fn test_hash_to_16_deterministic() {\n..."
    }
  ],
  "query": "...",
  "total_matched": 42
}
```

**Why it matters:** Structured output lets agents consume results directly without regex parsing. This is the single biggest UX improvement for agent integration.

---

## 3. `rag_search` with File/Kind Filters (Medium Impact)

**Problem:** No way to filter by file extension, symbol kind, or crate during search. If you're looking for traits specifically, you get impl blocks, tests, and macros mixed in.

**Suggestion:** Add optional filters to `rag_search`:
```json
{
  "name": "rag_search",
  "arguments": {
    "query": "peer authentication",
    "top_k": 5,
    "filter": {
      "symbol_kind": ["Trait"],
      "file_extension": [".rs"],
      "crates": ["swarm-core"]
    }
}
```

**Why it matters:** Reduces noise and improves precision for targeted queries. Especially useful in large workspaces with many crates.

---

## 4. Chunk Preview Length Config (Low Impact)

**Problem:** Results show raw chunk text which can be very long for large AST nodes. There's no way to control preview length — sometimes you get the full impl block, sometimes just a function.

**Suggestion:** Add `max_preview_lines` or `preview_size` parameter to limit snippet size while still returning the full chunk via a separate lookup mechanism.

---

## 5. `rag_file_read` Enhancements (Medium Impact)

**Problem:** Currently reads entire files up to 100KB. Fine for small files, but large source files in Rust workspaces can be several hundred KB or MB.

**Suggestion:** Add optional line range support:
```json
{
  "name": "rag_file_read",
  "arguments": {
    "file_path": "swarm-core/src/lib.rs",
    "line_start": 100,
    "line_end": 200
  }
}
```

**Why it matters:** Agents rarely need the entire file. Line-range reads save bandwidth and parsing time.

---

## 6. Call Graph / Import Graph Queries (Medium Impact)

**Problem:** RustRAG already builds call graph edges during indexing (`ra_ap_syntax` AST traversal), but this isn't exposed via MCP tools. You can search for code that *mentions* a function, but not find all functions that *call* it.

**Suggestion:** Add `rag_call_graph` tool:
```json
{
  "name": "rag_call_graph",
  "arguments": {
    "function_name": "derive_encrypting_key_with_hash",
    "direction": "callers" // or "callees"
  }
}
```

**Why it matters:** Understanding call chains is critical in Rust (where ownership and lifetimes create tight coupling). This is a unique advantage over generic vector search.

---

## 7. Cross-Crate Symbol Resolution (Medium Impact)

**Problem:** When searching for `PeerStore`, the agent gets results but doesn't know which crate defines it vs which crates use it as a type parameter. The workspace_info tool helps, but it's not integrated with search results.

**Suggestion:** In search results, include the defining crate name (if different from where it's used):
```json
{
  "symbol": "PeerStore",
  "defined_in_crate": "swarm-p2p",
  "used_in_crates": ["swarm-core", "swarm-stealth"],
  ...
}
```

**Why it matters:** In a multi-crate workspace, knowing where something is defined vs used reduces navigation overhead significantly.

---

## 8. MCP Tools List Should Report Available Filters (Low Impact)

**Problem:** The `tools/list` response doesn't document what optional arguments or filters each tool supports. Agents have to infer them from docs or trial-and-error.

**Suggestion:** Include argument schemas with descriptions in the tool definition:
```json
{
  "name": "rag_search",
  "description": "...",
  "inputSchema": {
    "properties": {
      "query": {"type": "string"},
      "top_k": {"type": "integer", "minimum": 1, "maximum": 100},
      "filter": {
        "type": "object",
        "properties": {
          "symbol_kind": {"type": "array", "items": {"type": "string"}},
          "crates": {"type": "array", "items": {"type": "string"}}
        }
      }
    }
  }
}
```

**Why it matters:** Better self-documentation means agents can use tools correctly on first try.

---

## Summary Priority Matrix

| # | Feature | Effort | Impact | Priority |
|---|---------|--------|--------|----------|
| 2 | Structured JSON output | Low | High | **P0** |
| 1 | Persistent server mode | Medium | High | **P1** |
| 3 | Search filters | Low | Medium | **P1** |
| 7 | Cross-crate symbol resolution | Low-Medium | Medium | **P2** |
| 5 | Line-range file read | Low | Medium | **P2** |
| 6 | Call graph queries | Medium | Medium | **P3** |
| 4 | Preview length config | Low | Low | **P3** |
| 8 | Better tool schemas in MCP | Low | Low | **P3** |

---

---

## 9. Stale Index Warning on Search (Medium Impact)

**Problem:** When working as an AI agent, I often modify code across multiple files before doing another search. The MCP `rag_search` tool returns results from whatever is currently indexed — but there's no indication whether the index matches the actual filesystem state. If files were modified after the last index build, I might:
- Search for a renamed function and get old results
- Get code chunks that no longer match what's on disk
- Not realize I need to reindex until I've already made incorrect inferences

**Current behavior:** `rag_search` silently returns whatever is indexed. The only hint is the "No index found" error before indexing, or manual `rust-rag info`/`reindex` commands.

**What would be ideal (compromise):**
RustRAG could add a hint to the agent when working via MCP:

```json
{
  "warning": {
    "message": "Index is potentially stale. X files changed since last index.",
    "suggestion": "Run 'rust-rag reindex' to refresh."
  }
}
```

This isn't auto-reindexing — it's RustRAG telling me: "hey, the index might be stale." Then *I* (the agent) decide what to do with that information. This is my job, not RustRAG's.

**Implementation note:** `rust_rag_state::IndexState` already has a `compare()` method and a `has_changes()` method that computes new/changed/removed files against the current workspace. You just need to:
1. Load `index_state.json` from `.rustrag/` in the MCP server (it's already loaded during `initialize`)
2. Walk the workspace once per tool call (or cache the file list between calls if persistent mode is added)
3. Compare current hashes against stored hashes using `IndexState::has_changes()` or `compare()`
4. Append a `"warning"` field to every search result with staleness info

**Why it matters:** This is the single most impactful feature for agent reliability. Without it, I have to either:
- Blindly assume the index is current (risky)
- Run `rust-rag reindex` before every search (slow and redundant)
- Manually check `rust-rag info` between searches (friction)

A staleness warning lets me make an informed decision: "should I trust this result or reindex?" — instead of silently getting wrong answers.

---

## Bonus: One-liner for the README

> "RustRAG is purpose-built for AI agents, but currently optimized for CLI users. Adding structured JSON output and persistent server mode would make it a first-class agent integration."
