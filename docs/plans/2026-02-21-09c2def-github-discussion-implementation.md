# 09c2def GitHub Discussions Hydration (Scrape-Only) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Port remaining scrape-only parity from shortlist commit `09c2def` by adding GitHub discussion hydration, short-content bypass guards, and contextual code-block improvements without changing MCP contracts.

**Architecture:** Keep all behavior changes inside the shared scrape pipeline so both HTTP and stdio tool surfaces inherit parity automatically. Extend `rust_scraper.rs` extraction internals first (payload parsing + post-clean guards + code-block relevance), then make minimal orchestration alignment in `scrape.rs` only where needed. Preserve existing fallback paths and response schema.

**Tech Stack:** Rust, Tokio, serde_json, scraper, html2text, cargo test, cargo clippy.

---

### Task 1: Extend GitHub embedded payload extraction for discussion/thread content

**Files:**
- Modify: `mcp-server/src/rust_scraper.rs`
- Test: `mcp-server/src/rust_scraper.rs` (existing `#[cfg(test)]` module)

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_github_embedded_content_discussion_body_and_comments() {
    let json: serde_json::Value = serde_json::json!({
        "payload": {
            "discussion": {
                "title": "How to use the API?",
                "body": "I need help using this endpoint.",
                "comments": {
                    "nodes": [
                        { "body": "Use token auth and retry on 429." },
                        { "body": "Also check rate-limit headers." }
                    ]
                }
            }
        }
    });

    let out = extract_github_embedded_content(&json).unwrap_or_default();
    assert!(out.contains("How to use the API?"));
    assert!(out.contains("I need help using this endpoint."));
    assert!(out.contains("Use token auth and retry on 429."));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_discussion_body_and_comments -- --exact`
Expected: FAIL because discussion fields are not handled yet.

**Step 3: Write minimal implementation**

- Extend `extract_github_embedded_content(...)` to append readable content from discussion/thread payloads when present.
- Keep existing priority for `payload.blob.text` and `payload.readme.text` unchanged.
- Add a small internal helper to gather string fields safely (`as_str`, non-empty, trimmed).
- Keep function return as `Option<String>` and avoid changing public schemas.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_discussion_body_and_comments -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_blob_text -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_readme_text -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/rust_scraper.rs
git commit -m "feat(scraper): hydrate github discussion embedded payload content"
```

---

### Task 2: Add short-content bypass guard for discussion-like pages in post-clean path

**Files:**
- Modify: `mcp-server/src/rust_scraper.rs`
- Test: `mcp-server/src/rust_scraper.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_post_clean_text_keeps_short_discussion_lines() {
    let scraper = RustScraper::new();

    let input = "Question: why does this fail?\n\nUse retries with backoff.\n\nThanks";
    let out = scraper.post_clean_text(input);

    assert!(out.contains("Question: why does this fail?"));
    assert!(out.contains("Use retries with backoff."));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_post_clean_text_keeps_short_discussion_lines -- --exact`
Expected: FAIL if current cleaning drops short conversational lines.

**Step 3: Write minimal implementation**

- Introduce a conservative bypass branch for discussion-like text in `post_clean_text(...)`.
- Keep existing JSON-noise filter active.
- Limit bypass to short, conversation-oriented text to avoid broad relaxation.
- Ensure bypass logic is local and does not alter unrelated extraction flows.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_post_clean_text_keeps_short_discussion_lines -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_is_json_noise_line_detects_json_fragments -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_is_json_noise_line_keeps_prose -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/rust_scraper.rs
git commit -m "feat(scraper): bypass aggressive cleanup for short discussion content"
```

---

### Task 3: Improve contextual code-block extraction quality without schema changes

**Files:**
- Modify: `mcp-server/src/rust_scraper.rs`
- Test: `mcp-server/src/rust_scraper.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_code_blocks_prefers_substantive_blocks_over_inline_noise() {
    let scraper = RustScraper::new();
    let html = r#"
    <html><body>
      <p>Use <code>ok</code> for quick checks.</p>
      <pre><code class=\"language-rust\">fn main() { println!(\"hello\"); }</code></pre>
    </body></html>
    "#;

    let doc = scraper::Html::parse_document(html);
    let blocks = scraper.extract_code_blocks(&doc);

    assert!(blocks.iter().any(|b| b.code.contains("fn main()")));
    assert!(!blocks.iter().any(|b| b.code.trim() == "ok"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_extract_code_blocks_prefers_substantive_blocks_over_inline_noise -- --exact`
Expected: FAIL if tiny inline snippets are still treated as equivalent noise blocks.

**Step 3: Write minimal implementation**

- Refine `extract_code_blocks(...)` scoring/filtering to prefer substantive fenced/pre code over tiny inline code.
- Preserve current public `CodeBlock` fields and semantics.
- Keep dedupe logic and language detection intact.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_code_blocks_prefers_substantive_blocks_over_inline_noise -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_clean_content_prefers_github_embedded_payload -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/rust_scraper.rs
git commit -m "feat(scraper): improve contextual code-block extraction quality"
```

---

### Task 4: Wire discussion hydration path in clean-content extraction flow

**Files:**
- Modify: `mcp-server/src/rust_scraper.rs`
- Modify: `mcp-server/src/scrape.rs` (only if orchestration alignment is required)
- Test: `mcp-server/src/rust_scraper.rs`, `mcp-server/src/scrape.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_clean_content_prefers_discussion_embedded_payload() {
    let scraper = RustScraper::new();
    let html = r##"
    <html><body>
      <script type=\"application/json\" data-target=\"react-app.embeddedData\">
        {"payload":{"discussion":{"title":"Thread","body":"Root post body"}}}
      </script>
      <div>fallback shell</div>
    </body></html>
    "##;

    let base = url::Url::parse("https://github.com/org/repo/discussions/1").unwrap();
    let text = scraper.extract_clean_content(html, &base);

    assert!(text.contains("Root post body"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_extract_clean_content_prefers_discussion_embedded_payload -- --exact`
Expected: FAIL before full wiring.

**Step 3: Write minimal implementation**

- Ensure `extract_clean_content(...)` invokes extended GitHub payload extraction before readability pass.
- Keep fallback behavior unchanged on parse/field misses.
- In `scrape.rs`, keep existing URL rewrite and response URL stability unchanged; only adjust if needed for discussion path parity.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_clean_content_prefers_discussion_embedded_payload -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_rewrite_url_for_clean_content_github_blob -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/rust_scraper.rs mcp-server/src/scrape.rs
git commit -m "feat(scraper): wire github discussion hydration into clean extraction"
```

---

### Task 5: Final verification pass and fixups

**Files:**
- No planned modifications (fixups only if checks fail)

**Step 1: Run formatter check**

Run: `cargo fmt --manifest-path mcp-server/Cargo.toml --all --check`
Expected: PASS for touched files; if unrelated baseline drift appears, document and isolate.

**Step 2: Run focused regression tests**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_discussion_body_and_comments -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_post_clean_text_keeps_short_discussion_lines -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_code_blocks_prefers_substantive_blocks_over_inline_noise -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_clean_content_prefers_discussion_embedded_payload -- --exact`
Expected: PASS.

**Step 3: Run full suite**

Run: `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 4: Run lint**

Run: `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets`
Expected: PASS.

**Step 5: Commit fixups only if needed**

```bash
# Only if step 1-4 required additional changes
git add <fixed-files>
git commit -m "chore(scraper): fix verification issues for 09c2def port"
```

---

## Execution Notes

- Apply @superpowers:test-driven-development on each task (strict Red → Green).
- Apply @superpowers:verification-before-completion before status claims/commits.
- Keep scope to scraper internals; no MCP contract changes.
- Prefer minimal, reviewable commits per task.