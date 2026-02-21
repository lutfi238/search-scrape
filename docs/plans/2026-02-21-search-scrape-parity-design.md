# Search-Scrape Parity Design (Core 6 Features)

Date: 2026-02-21
Status: Approved design lock
Owner: Search-Scrape

## 1) Goal

Bring Search-Scrape to feature parity with the core Firecrawl capability set while keeping Search-Scrape identity (tool names and contracts remain project-owned).

Target parity scope for v1:
1. Search
2. Scrape
3. Map
4. Crawl start (async)
5. Crawl status
6. Structured extract (LLM-backed)

## 2) Non-Goals (v1)

- No forced tool-name mirroring of Firecrawl prefixes.
- No full enterprise feature parity in v1 (webhooks, multi-tenant governance, advanced scheduling UI, etc.).
- No mandatory provider lock-in for LLM extraction.

## 3) Product Decisions Locked

1. Tool identity remains Search-Scrape specific.
2. Crawl model is async job-based (`start` + `status`), not synchronous-only.
3. Structured extraction is LLM-based and must support BYO model endpoint and API key.
4. Existing tools remain available for compatibility; new flow becomes preferred.

## 4) Final Tool Surface (v1)

1. `search_web` (existing, parity-enhanced behavior)
2. `scrape_url` (existing, parity-enhanced behavior)
3. `map_website` (new)
4. `crawl_start` (new)
5. `crawl_status` (new)
6. `extract_structured` (existing name, upgraded to LLM-BYO pipeline)

## 5) Contract Summary

### 5.1 `search_web`
Purpose: Search the web with filtering and optional content enrichment behavior.

Input (high-level):
- `query`
- `limit`
- `sources` (web/news/images style)
- `categories`
- `time_range`
- `location/country`
- optional scrape options for result enrichment

Output:
- ranked search results + metadata + optional scraped enrichments

### 5.2 `scrape_url`
Purpose: Scrape a single URL with extraction controls.

Input (high-level):
- `url`
- `formats`
- `max_chars`
- `only_main_content`
- `include_tags` / `exclude_tags`
- `wait_for`
- `max_age`

Output:
- normalized content payload + metadata + truncation/warning signals

### 5.3 `map_website` (new)
Purpose: Discover indexed/discoverable URLs on a site.

Input (high-level):
- `url`
- `limit`
- `search` (URL relevance filter)
- `include_subdomains`
- `sitemap_mode`

Output:
- URL list + optional metadata summary

### 5.4 `crawl_start` (new, async)
Purpose: Start crawl job and return immediately.

Input (high-level):
- `url`
- crawl limits/depth filters
- include/exclude rules
- same-domain controls
- optional scrape options for each visited page

Output:
- `job_id`
- initial `status` (`queued`/`running`)
- accepted config snapshot

### 5.5 `crawl_status` (new)
Purpose: Fetch crawl progress and results.

Input (high-level):
- `job_id`
- `include_results` (bool)
- pagination window for large result sets

Output:
- status (`queued`/`running`/`completed`/`failed`/`expired`)
- progress counters
- partial/final results
- error summary

### 5.6 `extract_structured` (upgraded)
Purpose: Structured extraction from URL/content using BYO LLM.

Input (high-level):
- `url` or raw content
- `schema` and/or `prompt`
- optional per-request LLM override block

Output:
- extracted structured JSON
- validation outcome
- confidence/warnings/errors

## 6) BYO LLM Architecture (Hard Requirement)

Structured extraction uses user-provided model endpoint and credentials.

Environment variables:
- `LLM_BASE_URL`
- `LLM_API_KEY`
- `LLM_MODEL`
- optional: `LLM_TIMEOUT_MS`, `LLM_MAX_TOKENS`, `LLM_TEMPERATURE`

Behavior:
- Missing config => explicit `LLM_NOT_CONFIGURED`
- Secrets are never logged
- Responses are parsed and schema-validated before returning

Provider strategy:
- Start with OpenAI-compatible HTTP interface
- Keep adapter boundary to add providers later without changing tool contract

## 7) Internal Architecture Changes

### 7.1 New/Updated modules
- New: `mcp-server/src/llm_client.rs`
- New: `mcp-server/src/crawl_jobs.rs`
- Upgrade: `mcp-server/src/extract.rs` (or split `extract_llm.rs`)
- Update: `mcp-server/src/stdio_service.rs` (new tool registration and handlers)
- Update: `mcp-server/src/types.rs` (request/response schemas)
- Update: `mcp-server/src/lib.rs` (shared state: LLM config + crawl job store)

### 7.2 Data flow

Search:
`request -> search.rs -> enrich/cache/history -> response`

Scrape:
`request -> scrape.rs + rust_scraper.rs -> normalize/cache -> response`

Map:
`request -> discovery pipeline -> dedupe/filter -> response`

Crawl async:
`crawl_start -> create job -> background worker`
`crawl_status -> query job store -> progress/results`

Extract structured:
`content acquisition -> prompt/schema build -> llm_client -> JSON parse -> schema validation -> response`

## 8) Error Model

Unified error envelope across tools:
- `code`
- `message`
- `details` (optional)
- `retryable`
- trace key (`request_id` or `job_id`)

Key error codes:
- `CRAWL_JOB_NOT_FOUND`
- `CRAWL_JOB_EXPIRED`
- `LLM_NOT_CONFIGURED`
- `LLM_AUTH_FAILED`
- `LLM_RATE_LIMITED`
- `LLM_TIMEOUT`
- `LLM_INVALID_JSON`
- `LLM_SCHEMA_VALIDATION_FAILED`

## 9) Security and Reliability

- Redact all secrets from logs and error payloads.
- Apply strict request size/input char bounds before LLM calls.
- Configure bounded retries and hard timeouts.
- Keep deterministic extraction defaults (low temperature).
- Enforce crawl result TTL to prevent unbounded memory growth.

## 10) Testing Strategy

### Unit tests
- crawl state transitions
- job store behavior and TTL expiry
- LLM response parser and schema validator
- error mapping and retry classification

### Integration tests
- mock OpenAI-compatible LLM server
- crawl async lifecycle: start -> running -> completed/failed
- extraction failure modes (timeout/auth/invalid-json)

### End-to-end tests
- local SearXNG integration flow: search -> scrape -> extract
- async crawl flow using `crawl_start` + `crawl_status`

## 11) Rollout Plan

### Phase 1
- Add `map_website`, `crawl_start`, `crawl_status`
- keep current tools operational

### Phase 2
- BYO LLM pipeline for `extract_structured`
- schema validation + error taxonomy

### Phase 3
- hardening: retries, metrics, perf tuning, docs

### Phase 4 (optional)
- webhook notifications for async jobs
- persistent job store backend
- multi-provider model routing

## 12) Compatibility Policy

- Existing tools remain functional during transition.
- New async crawl tools become recommended workflow.
- Legacy synchronous crawl patterns may be deprecated in later release with clear migration notice.

## 13) Success Criteria

Design is considered successful when:
1. All 6 core capabilities are available and stable.
2. Async crawl supports production-like progress reporting.
3. Structured extraction works with user-supplied LLM base URL + API key.
4. Error behavior is consistent and debuggable.
5. Existing users can migrate without breaking changes.
