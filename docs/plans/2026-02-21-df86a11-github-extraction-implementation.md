# df86a11 GitHub Extraction (Scrape-Only) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Port scrape-only parity from shortlist commit `df86a11` by improving GitHub content extraction and filtering leaked JSON noise without changing MCP contracts.

**Architecture:** Keep changes inside the shared scrape pipeline so both HTTP MCP (`mcp.rs`) and stdio MCP (`stdio_service.rs`) benefit automatically. Add a small URL rewrite helper in `scrape.rs`, then extend `rust_scraper.rs` with GitHub embedded payload extraction and JSON-noise line filtering. Preserve existing fallback/readability behavior when GitHub-specific paths are unavailable.

**Tech Stack:** Rust, Tokio, reqwest, scraper, serde_json, html2text, cargo test.

---

### Task 1: Add GitHub blob URL rewrite in scrape entry path

**Files:**
- Modify: `mcp-server/src/scrape.rs`
- Test: `mcp-server/src/scrape.rs` (existing `#[cfg(test)]` module)

**Step 1: Write the failing test**

```rust
#[test]
fn test_rewrite_url_for_clean_content_github_blob() {
    let input = "https://github.com/microsoft/vscode/blob/main/README.md";
    let rewritten = rewrite_url_for_clean_content(input);
    assert_eq!(
        rewritten.as_deref(),
        Some("https://raw.githubusercontent.com/microsoft/vscode/main/README.md")
    );
}

#[test]
fn test_rewrite_url_for_clean_content_non_blob_is_none() {
    assert!(rewrite_url_for_clean_content("https://github.com/user/repo").is_none());
    assert!(rewrite_url_for_clean_content("https://github.com/user/repo/issues/1").is_none());
    assert!(rewrite_url_for_clean_content("https://raw.githubusercontent.com/user/repo/main/a.rs").is_none());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_rewrite_url_for_clean_content_github_blob -- --exact`
Expected: FAIL because `rewrite_url_for_clean_content` does not exist yet.

**Step 3: Write minimal implementation**

```rust
fn rewrite_url_for_clean_content(url: &str) -> Option<String> {
    if url.contains("github.com/") && url.contains("/blob/") && !url.contains("raw.githubusercontent.com") {
        if let Some(blob_idx) = url.find("/blob/") {
            let prefix = &url[..blob_idx];
            let after_blob = &url[blob_idx + "/blob".len()..];
            if let Some(gh_idx) = prefix.find("github.com") {
                let scheme_prefix = &prefix[..gh_idx];
                let repo_path = &prefix[(gh_idx + "github.com".len())..];
                return Some(format!("{}raw.githubusercontent.com{}{}", scheme_prefix, repo_path, after_blob));
            }
        }
    }
    None
}
```

Then call it at the beginning of `scrape_url(...)` and use rewritten URL for fetch path.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_rewrite_url_for_clean_content_github_blob -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_rewrite_url_for_clean_content_non_blob_is_none -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/scrape.rs
git commit -m "feat(scrape): rewrite github blob URLs to raw content"
```

---

### Task 2: Add GitHub embedded payload extractor helper

**Files:**
- Modify: `mcp-server/src/rust_scraper.rs`
- Test: `mcp-server/src/rust_scraper.rs` (existing `#[cfg(test)]` module)

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_github_embedded_content_blob_text() {
    let json: serde_json::Value = serde_json::json!({
        "payload": { "blob": { "text": "# Hello\nreal file content" } }
    });
    assert_eq!(
        extract_github_embedded_content(&json).as_deref(),
        Some("# Hello\nreal file content")
    );
}

#[test]
fn test_extract_github_embedded_content_readme_text() {
    let json: serde_json::Value = serde_json::json!({
        "payload": { "readme": { "text": "## Readme\nrepo overview" } }
    });
    assert_eq!(
        extract_github_embedded_content(&json).as_deref(),
        Some("## Readme\nrepo overview")
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_blob_text -- --exact`
Expected: FAIL because `extract_github_embedded_content` does not exist yet.

**Step 3: Write minimal implementation**

```rust
fn extract_github_embedded_content(json: &serde_json::Value) -> Option<String> {
    let payload = json.get("payload")?;

    if let Some(blob) = payload.get("blob") {
        if let Some(text) = blob.get("text").and_then(|v| v.as_str()) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }

    if let Some(readme) = payload.get("readme") {
        if let Some(text) = readme.get("text").and_then(|v| v.as_str()) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }

    None
}
```

(After this minimal pass, extend with optional fields: `blob.richText`, `issue.body`, `pullRequest.body`, `discussion.body` if needed by acceptance criteria.)

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_blob_text -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_readme_text -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/rust_scraper.rs
git commit -m "feat(scraper): extract github embedded payload text"
```

---

### Task 3: Add JSON-noise classifier and apply it in post-clean stage

**Files:**
- Modify: `mcp-server/src/rust_scraper.rs`
- Test: `mcp-server/src/rust_scraper.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_is_json_noise_line_detects_json_fragments() {
    assert!(is_json_noise_line(r#"{"payload":{"a":1,"b":2,"c":3}}"#));
    assert!(is_json_noise_line(r#"{"allShortcutsEnabled":false,"refInfo":{}}"#));
}

#[test]
fn test_is_json_noise_line_keeps_prose() {
    assert!(!is_json_noise_line("This is a regular sentence about Rust scraping."));
    assert!(!is_json_noise_line("## Installation"));
    assert!(!is_json_noise_line(r#"{"a":1}"#)); // short JSON stays
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_is_json_noise_line_detects_json_fragments -- --exact`
Expected: FAIL because `is_json_noise_line` does not exist.

**Step 3: Write minimal implementation**

```rust
fn is_json_noise_line(line: &str) -> bool {
    if line.len() < 20 {
        return false;
    }

    let first = line.chars().next().unwrap_or(' ');
    if matches!(first, '{' | '[') && line.len() > 40 {
        return true;
    }

    let structural = line
        .chars()
        .filter(|c| matches!(c, '{' | '}' | '[' | ']' | '"' | ':' | ','))
        .count();

    (structural as f32 / line.len() as f32) >= 0.55
}
```

Then apply in `post_clean_text(...)` before `kept.push(...)`:

```rust
if is_json_noise_line(line_trim) {
    continue;
}
```

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_is_json_noise_line_detects_json_fragments -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_is_json_noise_line_keeps_prose -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/rust_scraper.rs
git commit -m "feat(scraper): filter leaked JSON noise lines"
```

---

### Task 4: Wire embedded GitHub extraction into `extract_clean_content`

**Files:**
- Modify: `mcp-server/src/rust_scraper.rs`
- Test: `mcp-server/src/rust_scraper.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_clean_content_prefers_github_embedded_payload() {
    let scraper = RustScraper::new();
    let html = r#"
    <html><body>
      <script type="application/json" data-target="react-app.embeddedData">
        {"payload":{"blob":{"text":"# Title\nactual content from blob"}}}
      </script>
      <div>fallback noise</div>
    </body></html>
    "#;

    let base = url::Url::parse("https://github.com/org/repo/blob/main/README.md").unwrap();
    let text = scraper.extract_clean_content(html, &base);

    assert!(text.contains("actual content from blob"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path mcp-server/Cargo.toml test_extract_clean_content_prefers_github_embedded_payload -- --exact`
Expected: FAIL because extraction pipeline does not read GitHub embedded payload yet.

**Step 3: Write minimal implementation**

At the start of `extract_clean_content(...)`, add GitHub-specific fast path:

```rust
if html.contains("react-app.embeddedData") {
    if let Ok(sel) = Selector::parse("script[data-target='react-app.embeddedData']") {
        let doc = Html::parse_document(html);
        if let Some(el) = doc.select(&sel).next() {
            let raw = el.text().collect::<String>();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(content) = extract_github_embedded_content(&json) {
                    return self.post_clean_text(&content);
                }
            }
        }
    }
}
```

Then continue existing readability/heuristic flow as fallback.

**Step 4: Run test to verify it passes**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_clean_content_prefers_github_embedded_payload -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_blob_text -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_is_json_noise_line_detects_json_fragments -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add mcp-server/src/rust_scraper.rs
git commit -m "feat(scraper): use github embedded payload in clean extraction"
```

---

### Task 5: Verification pass before completion

**Files:**
- No planned file modifications

**Step 1: Run formatter check**

Run: `cargo fmt --manifest-path mcp-server/Cargo.toml --all --check`
Expected: PASS.

**Step 2: Run focused regression tests**

Run:
- `cargo test --manifest-path mcp-server/Cargo.toml test_rewrite_url_for_clean_content_github_blob -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_extract_github_embedded_content_blob_text -- --exact`
- `cargo test --manifest-path mcp-server/Cargo.toml test_is_json_noise_line_detects_json_fragments -- --exact`
Expected: PASS.

**Step 3: Run full test suite**

Run: `cargo test --manifest-path mcp-server/Cargo.toml`
Expected: PASS.

**Step 4: Run lint check**

Run: `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets`
Expected: PASS (no new warnings from this change set).

**Step 5: Commit fixups only if needed**

```bash
# Only if step 1-4 required code changes
git add <fixed-files>
git commit -m "chore(scrape): fix verification issues for df86a11 port"
```

---

## Execution Notes

- Apply @superpowers:test-driven-development on every task (strict Red → Green).
- Apply @superpowers:verification-before-completion before claiming done.
- Keep implementation minimal; no unrelated refactors.
- Do not change MCP response schema for this port.
