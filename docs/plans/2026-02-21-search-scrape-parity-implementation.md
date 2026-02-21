# Search-Scrape Parity (Core 6 Features) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Deliver six Firecrawl-like core capabilities in Search-Scrape with Search-Scrape-native tool names, including async crawl jobs and BYO-LLM structured extraction.

**Architecture:** Extend the current MCP stdio service with three new capability layers: (1) website mapping entry point, (2) async crawl job orchestration with status polling, and (3) provider-agnostic LLM client for `extract_structured`. Keep existing modules reusable (search/scrape/crawl) and add minimal new state in `AppState` for jobs + LLM config.

**Tech Stack:** Rust, Tokio async runtime, rmcp, reqwest, serde/serde_json, moka cache, existing crawl/search/scrape modules.

---

### Task 1: Add LLM and async job domain types

**Files:**
- Modify: `mcp-server/src/types.rs`
- Test: `mcp-server/src/types.rs` (inline unit tests in `#[cfg(test)]` module)

**Step 1: Write the failing test**

Add tests asserting serialization/deserialization and enum stability for:
- `CrawlJobStatus` (`queued`, `running`, `completed`, `failed`, `expired`)
- `CrawlStartRequest`, `CrawlStartResponse`, `CrawlStatusRequest`, `CrawlStatusResponse`
- `LlmConfig`
- `ToolErrorEnvelope`

```rust
#[test]
fn test_crawl_job_status_serialization() {
    let s = serde_json::to_string(&CrawlJobStatus::Queued).unwrap();
    assert_eq!(s, "\"queued\"");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_crawl_job_status_serialization -- --exact`
Expected: FAIL because new types are not defined.

**Step 3: Write minimal implementation**

Add new structs/enums in `types.rs`:
- `CrawlJobStatus`
- `CrawlStartRequest`, `CrawlStartResponse`
- `CrawlStatusRequest`, `CrawlStatusResponse`
- `LlmConfig` (base_url/api_key/model/timeout/max_tokens/temperature)
- `ToolErrorEnvelope` (`code`, `message`, `details`, `retryable`, `request_id_or_job_id`)

Keep fields minimal and `serde`-friendly.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_crawl_job_status_serialization -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/types.rs
git commit -m "feat(types): add async crawl job and llm config schemas"
```

---

### Task 2: Add LLM client module for BYO provider

**Files:**
- Create: `mcp-server/src/llm_client.rs`
- Modify: `mcp-server/src/lib.rs`
- Test: `mcp-server/src/llm_client.rs` (unit tests with mocked payload conversion)

**Step 1: Write the failing test**

Add tests:
- `test_llm_config_from_env_missing_required_returns_error`
- `test_openai_compatible_payload_shape`
- `test_redact_api_key_in_debug`

```rust
#[test]
fn test_llm_config_from_env_missing_required_returns_error() {
    std::env::remove_var("LLM_BASE_URL");
    std::env::remove_var("LLM_API_KEY");
    std::env::remove_var("LLM_MODEL");
    let err = LlmClient::from_env().unwrap_err();
    assert!(err.to_string().contains("LLM_NOT_CONFIGURED"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_llm_config_from_env_missing_required_returns_error -- --exact`
Expected: FAIL because module/type is missing.

**Step 3: Write minimal implementation**

Implement `llm_client.rs` with:
- `LlmClient` struct
- `LlmClient::from_env()`
- `LlmClient::extract_json(prompt, schema, content)` (OpenAI-compatible chat/completions request)
- error mapping: `LLM_NOT_CONFIGURED`, `LLM_AUTH_FAILED`, `LLM_RATE_LIMITED`, `LLM_TIMEOUT`, `LLM_INVALID_JSON`

Update `lib.rs`:
- `pub mod llm_client;`
- add `llm: Option<Arc<llm_client::LlmClient>>` to `AppState`
- initialize from env in constructor path (no panic if absent)

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_llm_config_from_env_missing_required_returns_error -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/llm_client.rs mcp-server/src/lib.rs
git commit -m "feat(llm): add BYO LLM client with env-based config"
```

---

### Task 3: Add async crawl job store and runner

**Files:**
- Create: `mcp-server/src/crawl_jobs.rs`
- Modify: `mcp-server/src/lib.rs`
- Test: `mcp-server/src/crawl_jobs.rs`

**Step 1: Write the failing test**

Add tests:
- `test_create_job_initial_state_is_queued`
- `test_transition_to_running_and_completed`
- `test_job_not_found`
- `test_expire_old_job`

```rust
#[tokio::test]
async fn test_create_job_initial_state_is_queued() {
    let store = CrawlJobStore::new(std::time::Duration::from_secs(60));
    let job_id = store.create_job("https://example.com".to_string()).await;
    let job = store.get_job(&job_id).await.unwrap();
    assert_eq!(job.status, CrawlJobStatus::Queued);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_create_job_initial_state_is_queued -- --exact`
Expected: FAIL because store doesn’t exist.

**Step 3: Write minimal implementation**

Implement in-memory store with:
- `create_job`
- `mark_running`
- `mark_completed`
- `mark_failed`
- `get_job`
- `expire_jobs`

Use `tokio::sync::RwLock<HashMap<String, CrawlJobRecord>>` and UUID-like ID generation.

Update `lib.rs` state:
- add `crawl_jobs: Arc<crawl_jobs::CrawlJobStore>`
- initialize with TTL (e.g., 24 hours)

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_create_job_initial_state_is_queued -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/crawl_jobs.rs mcp-server/src/lib.rs
git commit -m "feat(crawl): add async crawl job store and lifecycle"
```

---

### Task 4: Add `map_website` tool handler using existing crawl/discovery logic

**Files:**
- Modify: `mcp-server/src/stdio_service.rs`
- Modify: `mcp-server/src/types.rs`
- Test: `mcp-server/src/stdio_service.rs` (tool argument validation tests)

**Step 1: Write the failing test**

Add a test asserting `list_tools` includes `map_website` and required input fields.

```rust
#[tokio::test]
async fn test_list_tools_contains_map_website() {
    let svc = test_service().await;
    let result = svc.list_tools(None, test_ctx()).await.unwrap();
    assert!(result.tools.iter().any(|t| t.name == "map_website"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_list_tools_contains_map_website -- --exact`
Expected: FAIL because tool absent.

**Step 3: Write minimal implementation**

In `stdio_service.rs`:
- register new tool `map_website`
- parse args (`url`, `limit`, `search`, `include_subdomains`, `sitemap_mode`)
- implement handler by calling a minimal map function:
  - either a lightweight path in `crawl.rs` (discovery only), or
  - run constrained crawl and return URL list only

Keep v1 behavior deterministic and documented.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_list_tools_contains_map_website -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/stdio_service.rs mcp-server/src/types.rs
git commit -m "feat(mcp): add map_website tool"
```

---

### Task 5: Add `crawl_start` and `crawl_status` MCP tools

**Files:**
- Modify: `mcp-server/src/stdio_service.rs`
- Modify: `mcp-server/src/crawl_jobs.rs`
- Modify: `mcp-server/src/types.rs`
- Test: `mcp-server/src/stdio_service.rs`

**Step 1: Write the failing test**

Add tests:
- `test_list_tools_contains_crawl_start_and_crawl_status`
- `test_crawl_start_returns_job_id`
- `test_crawl_status_not_found_returns_expected_error`

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_list_tools_contains_crawl_start_and_crawl_status -- --exact`
Expected: FAIL.

**Step 3: Write minimal implementation**

In `stdio_service.rs`:
- register tools: `crawl_start`, `crawl_status`
- `crawl_start`:
  - validate args
  - create job in store
  - spawn `tokio::spawn` background crawl task
  - return `{job_id, status}`
- `crawl_status`:
  - fetch job from store
  - return progress + optional paginated results

In `crawl_jobs.rs`:
- add progress update helper methods used by background runner.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_list_tools_contains_crawl_start_and_crawl_status -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/stdio_service.rs mcp-server/src/crawl_jobs.rs mcp-server/src/types.rs
git commit -m "feat(crawl): add async crawl_start and crawl_status tools"
```

---

### Task 6: Upgrade `extract_structured` to LLM-BYO path

**Files:**
- Modify: `mcp-server/src/extract.rs`
- Modify: `mcp-server/src/stdio_service.rs`
- Modify: `mcp-server/src/types.rs`
- Test: `mcp-server/src/extract.rs`

**Step 1: Write the failing test**

Add tests:
- `test_extract_structured_returns_llm_not_configured_when_llm_missing`
- `test_extract_structured_validates_json_schema`
- `test_extract_structured_handles_invalid_json`

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_extract_structured_returns_llm_not_configured_when_llm_missing -- --exact`
Expected: FAIL.

**Step 3: Write minimal implementation**

In `extract.rs`:
- keep existing heuristic fallback path behind explicit branch only if desired
- primary path:
  1. scrape content (existing flow)
  2. call `state.llm` client
  3. parse JSON
  4. validate against provided schema/prompt constraints
  5. map errors to typed envelope

In `stdio_service.rs`:
- update tool description and schema notes to mention BYO LLM requirement.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_structured_returns_llm_not_configured_when_llm_missing -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/extract.rs mcp-server/src/stdio_service.rs mcp-server/src/types.rs
git commit -m "feat(extract): enable BYO LLM structured extraction"
```

---

### Task 7: Standardize MCP error envelope and tool docs

**Files:**
- Modify: `mcp-server/src/stdio_service.rs`
- Modify: `mcp-server/src/types.rs`
- Test: `mcp-server/src/stdio_service.rs`

**Step 1: Write the failing test**

Add tests asserting known failures return consistent error shape with `code`, `message`, `retryable`.

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_error_envelope_shape -- --exact`
Expected: FAIL.

**Step 3: Write minimal implementation**

- unify all tool errors through shared helper in `stdio_service.rs`
- ensure sensitive data is redacted from error strings

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_error_envelope_shape -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/stdio_service.rs mcp-server/src/types.rs
git commit -m "refactor(mcp): standardize tool error envelope"
```

---

### Task 8: Wire environment configuration and update docs

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `mcp-server/mcp.env` (if present)
- Test: manual command verification

**Step 1: Write the failing check**

Create doc checklist in commit message draft and verify missing sections before edits:
- new tools listed
- async crawl workflow documented
- LLM env vars documented

**Step 2: Run verification to confirm gap**

Run:
- `grep -n "crawl_start\|crawl_status\|map_website\|LLM_BASE_URL\|LLM_API_KEY\|LLM_MODEL" README.md CLAUDE.md`
Expected: missing or incomplete entries.

**Step 3: Write minimal implementation**

Update docs with:
- tool list and example payloads
- async crawl lifecycle examples
- BYO LLM setup examples
- troubleshooting notes for auth/timeouts

**Step 4: Run verification to confirm presence**

Run:
- `grep -n "crawl_start\|crawl_status\|map_website\|LLM_BASE_URL\|LLM_API_KEY\|LLM_MODEL" README.md CLAUDE.md`
Expected: entries present.

**Step 5: Commit**

```bash
git add README.md CLAUDE.md mcp-server/mcp.env
git commit -m "docs: add async crawl and BYO LLM extraction configuration"
```

---

### Task 9: Full validation pass before merge

**Files:**
- No code changes expected

**Step 1: Run formatting**

Run: `cargo fmt --manifest-path mcp-server/Cargo.toml --all --check`
Expected: PASS.

**Step 2: Run linting**

Run: `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets`
Expected: PASS with zero new warnings.

**Step 3: Run tests**

Run: `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 4: Smoke test MCP startup**

Run:
- `cargo run --release --manifest-path mcp-server/Cargo.toml --bin search-scrape-mcp`
Expected: service starts and lists tools including new ones.

**Step 5: Commit (if any fixups from validation)**

```bash
git add <fixed-files>
git commit -m "chore: finalize parity feature validation fixes"
```

---

## Notes for Executor

- Keep steps DRY/YAGNI: no extra abstraction unless needed by two or more new call-sites.
- Preserve backward compatibility of existing tools while adding new tool surface.
- Prefer incremental tests per task; avoid large unreviewed code jumps.
- Do not log secrets from env or request bodies.
