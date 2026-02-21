# 09c2def Scrape-Only Design: GitHub Discussions Hydration + Short-Content Bypass + Contextual Code Blocks

Date: 2026-02-21
Status: Approved
Scope: Scrape-only parity port for shortlist commit `09c2def`

## 1) Goal

Improve scrape quality for GitHub discussion/thread pages by hydrating real conversation content, preserving short-but-meaningful text, and improving code block usefulness, without changing MCP tool contracts.

## 2) Scope (Locked)

In scope:
1. Add GitHub Discussions/thread hydration in shared scrape extraction pipeline.
2. Add short-content bypass behavior for discussion-like pages so conversational context is not over-trimmed.
3. Improve contextual code-block extraction quality while preserving existing response schema.

Out of scope:
- MCP tool name/schema changes.
- Non-GitHub domain-specific extraction expansions.
- Broad auth-wall/CDP refactors beyond what is required for this shortlist item.

## 3) Constraints

- Preserve backward compatibility for `scrape_url` output shape.
- Keep implementation minimal and targeted (YAGNI).
- Apply behavior in shared scrape pipeline so both HTTP and stdio surfaces inherit parity.

## 4) Selected Approach

Selected approach: **Full targeted port of `09c2def` scrape behaviors**.

Why:
- Completes the remaining scrape-side parity item after `df86a11`.
- Keeps blast radius limited to scraper internals.
- Delivers practical quality gains for GitHub discussion pages used in research workflows.

## 5) Architecture and File-Level Design

### 5.1 `mcp-server/src/rust_scraper.rs`

Primary implementation file.

1) Extend GitHub embedded payload extraction:
- Build on existing `extract_github_embedded_content` helper.
- Add discussion/thread-oriented field traversal in embedded JSON payloads.
- Prefer readable body/comment text; fallback to current extraction path when fields are absent.

2) Add short-content bypass behavior:
- For identified discussion/thread pages, avoid aggressive trimming/filtering on short documents.
- Keep current cleaning pipeline, but guard against dropping key conversational lines.

3) Improve contextual code-block quality:
- Keep public `CodeBlock` schema unchanged (`language`, `code`, optional ranges).
- Use surrounding prose/structure internally to improve which blocks are captured and retained.
- Do not add new response fields.

4) Maintain existing JSON noise filtering path:
- Keep `is_json_noise_line` conservative.
- Tune only where needed to avoid false positives on human conversation lines.

### 5.2 `mcp-server/src/scrape.rs`

No contract-level changes.

- Preserve existing URL normalization behavior from `df86a11`.
- Ensure scrape orchestration continues to route through shared extractor path so new GitHub hydration behavior applies consistently.

## 6) Data Flow

`request -> scrape::scrape_url`
`-> url normalization (existing)`
`-> rust_scraper fetch`
`-> GitHub discussion embedded payload hydration (if matched)`
`-> fallback readability extraction`
`-> post-clean + short-content bypass guards + JSON-noise filter`
`-> contextual code-block selection`
`-> ScrapeResponse (existing schema)`

## 7) Error Handling and Safety

- Embedded JSON parse failure is non-fatal; continue with normal extraction.
- Missing discussion fields produce best-effort partial extraction, not hard error.
- Short-content bypass activates only in discussion/thread contexts.
- No secrets or auth material are introduced or exposed by this feature.

## 8) Testing Strategy (TDD)

Planned tests:
1. Discussion hydration:
   - Embedded discussion payload yields non-empty conversational content.
   - Invalid/missing payload falls back cleanly.
2. Short-content bypass:
   - Short discussion content is preserved (not over-pruned).
3. JSON noise filter regression:
   - JSON-fragment lines are still filtered.
   - Normal prose lines remain.
4. Contextual code-block extraction:
   - Relevant code blocks from discussion-like markdown/HTML are retained and stable.
5. No-regression checks:
   - Existing GitHub blob rewrite behavior remains correct.
   - Existing scrape output schema remains unchanged.

## 9) Acceptance Criteria

- GitHub discussion/thread pages produce materially better `clean_content` than shell/noise output.
- Short discussion pages preserve key conversational context.
- Extracted `code_blocks` are more useful for downstream agent consumption.
- No MCP contract changes are introduced.

## 10) Approval Record

User approved moving forward with full remaining parity scope and selected option 1 in-session.