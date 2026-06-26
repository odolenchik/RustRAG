# Observations from Real Agent Use of RustRAG

**From:** AI agent (Kimi Code)  
**Tested:** 9 search queries across the swarmx workspace  
**Verified:** Read actual files and confirmed all returned snippets match exactly  

---

## Observations & Suggestions Beyond the Main Feedback

### 1. Search is slow per-call (~50–80ms cold start overhead)

**Observation:** Every `rag_search` call spawns a new MCP subprocess, loads config, runs vector search. For an agent that may make 10+ searches in one session (e.g., "explain this module" → read file → find related function → search again), the per-call startup cost adds up to several seconds of wasted time.

**Suggestion:** When you add persistent server mode (#1 from main feedback), consider caching the embedding model in memory so it's loaded once and reused across searches — not just kept alive, but actually avoiding re-parsing the ONNX graph on each call. Even with stdio persistence, if the process is reused between sessions (agent turn 1 vs agent turn 2), the model reloads from disk.

---

### 2. No "similar query" or "find related code near this result" capability

**Observation:** After finding PeerStore in peer_store.rs, I'd naturally want to: "what calls PeerStore?" or "show me all functions that interact with PeerStore." Currently I have to formulate a new search query manually — e.g., "PeerStore usage" — and hope the search finds it.

**Suggestion:** Add a `related` parameter to `rag_search`:
```json
{
  "name": "rag_search",
  "arguments": {
    "query": "peer store",
    "top_k": 5,
    "related_to_chunk_id": "swarm-p2p/src/peer_store.rs:89"
  }
}
```

This would return code chunks that are **structurally or semantically related** to the given chunk — e.g., callers of PeerStore methods, types it depends on, etc. The call graph already exists in the indexer (`rust_rag_callgraph`), so this is mostly a query against existing data.

---

### 3. Search results lack "confidence" signal when score drops below threshold

**Observation:** When I searched for "stealth covert channel implementation", the scores were 0.747, 0.664, 0.658. The gap between top and second result is significant (0.08), but without context on what constitutes a "good" score, it's hard to know if I should trust or dig deeper.

**Suggestion:** Include a `_meta.confidence` field:
```json
{
  "_meta": {
    "confidence": "high", // low / medium / high
    "score_range": [0.658, 0.747],
    "top_score_threshold": 0.75
  }
}
```

This helps agents decide: if confidence is "low" and scores are spread thin, either broaden the query or try a different approach (e.g., grep for the module name directly).

---

### 4. Chunk overlap makes it hard to know where one chunk ends and another begins

**Observation:** When reading search results, overlapping chunks from adjacent AST nodes sometimes have partially duplicated content. For example, an `impl` block might start at line X in one chunk and continue into a second chunk that starts mid-impl-block. Without visual markers or line range metadata, it's confusing whether the duplication is intentional (overlap) or if there are two different implementations.

**Suggestion:** Include overlap metadata:
```json
{
  "is_overlap": true,
  "overlaps_with_chunk_id": "swarm-p2p/src/dht.rs_3856",
  "unique_content_lines": [90, 91, ...] // lines that are unique to this chunk
}
```

---

### 5. No way to search by file extension or directory scope

**Observation:** When searching for "peer authentication," I get results from test files (`_test.rs`), impl files (`lib.rs`), and binary entry points (`main.rs`). If I only care about production code, I have to post-filter the results.

**Suggestion:** Add `scope` parameter:
```json
{
  "name": "rag_search",
  "arguments": {
    "query": "peer authentication",
    "top_k": 5,
    "scope": {
      "exclude_test_files": true,
      "include_paths_glob": ["swarm-*/src/**"]
    }
}
```

---

### 6. Search is one-directional: query → results, but not results → query refinement

**Observation:** After finding DHTStore but wanting to understand its relationship with PeerStore, I had to manually think of a new search query ("DHTStore and PeerStore interaction"). An ideal flow would be: search DHTStore → show related nodes → click "PeerStore" → get focused results on DHTStore↔PeerStore interactions.

**Suggestion:** Add `suggest_next_queries` metadata in the response:
```json
{
  "_meta": {
    "suggested_queries": [
      "DHTStore PeerStore interaction",
      "peer store DHT integration",
      "how DHTStore uses PeerInfo"
    ]
  }
}
```

This could be generated from: (a) co-occurring terms in nearby chunks, (b) call graph neighbors of the found symbols, or (c) frequently-searched queries that led to similar results across other agents.

---

### 7. No "search history" or deduplication hint

**Observation:** When doing iterative exploration (find X → read file → search for related Y), I sometimes re-query with overlapping terms and get the same results. There's no way to tell RustRAG "I already searched for 'peer store', show me NEW results."

**Suggestion:** Add `exclude_searched_chunks` parameter:
```json
{
  "name": "rag_search",
  "arguments": {
    "query": "DHTStore usage patterns",
    "top_k": 5,
    "exclude_chunk_ids": ["swarm-p2p/src/dht.rs_153", "swarm-p2p/src/dht.rs_4772"]
  }
}
```

---

### 8. No batch search capability

**Observation:** When understanding a module, I often know multiple related concepts upfront — e.g., "DHTStore", "PeerStore", "gossip protocol" — and want to search all of them in one go instead of making 3 separate MCP calls (which means 3 subprocess spawns).

**Suggestion:** Add `rag_batch_search` tool:
```json
{
  "name": "rag_batch_search",
  "arguments": {
    "queries": [
      {"query": "DHTStore implementation", "top_k": 3},
      {"query": "PeerStore lifecycle", "top_k": 3},
      {"query": "gossip protocol message handling", "top_k": 3}
    ]
  }
}
```

This would return all results in a single response, saving subprocess spawn overhead. With persistent server mode (#1), this is especially valuable.

---

### 9. No way to rank results by recency (mtime) or importance

**Observation:** When searching for "error handling," I get old and potentially deprecated error types mixed with current ones. There's no temporal awareness — the search doesn't know which code was most recently updated.

**Suggestion:** Add `order_by` parameter:
```json
{
  "name": "rag_search",
  "arguments": {
    "query": "error handling",
    "top_k": 5,
    "order_by": "relevance" // or "recency" (mtime), "file_size"
  }
}
```

---

### 10. Token count per result would help agents manage context windows

**Observation:** When reading search results, chunks vary in size from a few lines to hundreds of lines. For an agent with limited context window budget, knowing the token count upfront helps decide whether to read the full chunk or just use it as a pointer.

**Suggestion:** Include `token_estimate` in each result:
```json
{
  "results": [
    {
      "id": "...",
      "text": "...",
      "_meta": {
        "approximate_token_count": 142,
        "max_context_budget_remaining": 8058
      }
    }
  ]
}
```

---

## Summary Priority Matrix (Additional Observations)

| # | Feature | Effort | Impact on Agent UX | Priority |
|---|---------|--------|-------------------|----------|
| 2 | Structured JSON output | Low | High | **P0** |
| 8 | Batch search | Low-Medium | Medium-High | **P1** |
| 3 | Scope filters (exclude tests, etc.) | Low | Medium | **P1** |
| 6 | Suggest next queries | Medium | Medium | **P2** |
| 7 | Exclude already-searched chunks | Low | Medium | **P2** |
| 4 | Overlap metadata | Low | Medium | **P2** |
| 9 | Order by recency | Low-Medium | Low-Medium | **P3** |
| 10 | Token count estimate | Low | Low-Medium | **P3** |
| 5 | Confidence score | Low | Medium | **P2** |
| 1 | Related chunks (call graph) | Medium | High | **P1** |
