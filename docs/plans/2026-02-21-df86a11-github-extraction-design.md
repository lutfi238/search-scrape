# df86a11 Scrape-Only Design: GitHub Content Extraction + JSON Noise Filter

Date: 2026-02-21
Status: Approved
Scope: Scrape-only parity port for shortlist commit `df86a11`

## 1) Goal

Improve scrape quality for GitHub pages by reducing SPA/JSON noise and extracting meaningful content more reliably, while keeping existing MCP tool contracts unchanged.

## 2) Scope (Locked)

In scope:
1. Rewrite GitHub blob URLs to raw content URLs before scrape.
2. Extract readable content from GitHub embedded React JSON payload (`react-app.embeddedData`) when available.
3. Filter JSON-like noisy lines that leak into final cleaned text.

Out of scope:
- New MCP tool names or schema contract changes.
- Non-GitHub domain-specific extraction logic.
- Discussion hydration and broader GitHub enhancements planned for later shortlist items (e.g. `09c2def`).

## 3) Constraints

- Preserve backward compatibility for `scrape_url` outputs.
- Keep implementation minimal and targeted (YAGNI).
- Maintain parity behavior for both runtime surfaces (HTTP MCP and stdio MCP) by implementing inside shared scrape pipeline.

## 4) Selected Approach

Selected approach: **Minimal targeted parity**.

Why:
- Lowest risk and smallest blast radius.
- Directly matches the intent of `df86a11` without broad refactors.
- Leaves room to layer additional GitHub-specific improvements in subsequent shortlist commits.

## 5) Architecture and File-Level Design

### 5.1 `mcp-server/src/scrape.rs`

Add URL rewrite helper in scrape entry path:
- `rewrite_url_for_clean_content(url: &str) -> Option<String>`
- Rewrite pattern:
  - `https://github.com/{owner}/{repo}/blob/{ref}/{path}`
  - -> `https://raw.githubusercontent.com/{owner}/{repo}/{ref}/{path}`

Apply rewrite at start of scrape flow before fetch/caching logic uses the URL for extraction.

### 5.2 `mcp-server/src/rust_scraper.rs`

Enhance extraction pipeline with two GitHub-focused improvements:

1) Embedded payload extraction:
- Detect script node: `script[data-target='react-app.embeddedData']`
- Parse JSON and prioritize readable fields:
  - `payload.blob.text`
  - `payload.blob.richText` (HTML -> markdown conversion path if needed)
  - `payload.readme.richText` / `payload.readme.text`
  - `payload.issue.body`, `payload.pullRequest.body`, `payload.discussion.body`
- If targeted fields are absent, fallback to existing extraction path.

2) JSON noise filtering:
- Add `is_json_noise_line(line: &str) -> bool` heuristic for long JSON-structural fragments.
- Integrate into final text cleanup stage to drop noisy lines while preserving normal prose.

## 6) Data Flow

`request -> scrape::scrape_url`
`-> optional GitHub blob URL rewrite`
`-> RustScraper::scrape_url fetch`
`-> try GitHub embedded payload extraction`
`-> fallback readability/heuristic extraction`
`-> post-clean + JSON-noise filter`
`-> ScrapeResponse (existing contract)`

## 7) Error Handling and Safety

- If rewrite does not match expected pattern, use original URL unchanged.
- If embedded JSON parse fails, continue with existing extraction flow.
- Noise filter remains conservative to avoid removing regular human-readable lines.

## 8) Testing Strategy (TDD)

Planned tests:
1. Rewrite helper tests:
   - Blob URL rewrites correctly.
   - Non-blob URLs remain unchanged (`None`).
2. JSON noise classifier tests:
   - JSON-fragment lines detected.
   - Human-readable lines preserved.
3. Embedded payload extraction tests:
   - `payload.blob.text` extraction success.
   - `payload.readme.text` extraction success.
4. Full suite verification:
   - `cargo test --manifest-path mcp-server/Cargo.toml`

## 9) Acceptance Criteria

- GitHub blob URLs are rewritten to raw URLs in scrape path.
- GitHub embedded payload content is extracted when available.
- Final cleaned content contains materially less JSON fragment noise.
- Existing tool contracts and response shapes remain compatible.

## 10) Approval Record

User approved continuation after design presentation in-session ("oke lanjut aja").
