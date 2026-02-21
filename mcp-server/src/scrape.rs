use crate::rust_scraper::RustScraper;
use crate::types::*;
use crate::AppState;
use anyhow::{anyhow, Result};
use backoff::future::retry;
use backoff::ExponentialBackoffBuilder;
use futures::stream::{self, StreamExt};
use select::predicate::Predicate;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Rewrite GitHub blob URLs to raw.githubusercontent.com for clean content fetch.
/// Returns `Some(rewritten_url)` for blob URLs, `None` for all other URLs.
///
/// Only rewrites when ALL of the following are true:
///   1. The parsed host is exactly "github.com" (no subdomains, no lookalike hosts).
///   2. The URL *path* (not query string) contains the "/blob/" segment.
///   3. The URL is not already a raw.githubusercontent.com URL.
fn rewrite_url_for_clean_content(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;

    // Guard 1: exact host match — no subdomains, no lookalike hosts.
    if parsed.host_str() != Some("github.com") {
        return None;
    }

    // Guard 2: "/blob/" must appear in the *path*, not the query string.
    let path = parsed.path();
    let blob_seg = "/blob/";
    let blob_pos = path.find(blob_seg)?;

    // Guard 3: not already a raw URL (defensive; host check above already excludes it).
    // (This is implicitly satisfied by guard 1, but kept for clarity.)

    // Build the raw URL: swap host, drop "/blob" from the path segment.
    let repo_path = &path[..blob_pos]; // e.g. "/microsoft/vscode"
    let after_blob = &path[blob_pos + "/blob".len()..]; // e.g. "/main/README.md"

    // Preserve query and fragment if present (unlikely for blob URLs, but safe).
    let query_frag = match (parsed.query(), parsed.fragment()) {
        (Some(q), Some(f)) => format!("?{}#{}", q, f),
        (Some(q), None) => format!("?{}", q),
        (None, Some(f)) => format!("#{}", f),
        (None, None) => String::new(),
    };

    Some(format!(
        "{}://raw.githubusercontent.com{}{}{}",
        parsed.scheme(),
        repo_path,
        after_blob,
        query_frag,
    ))
}

pub async fn scrape_url(state: &Arc<AppState>, url: &str) -> Result<ScrapeResponse> {
    info!("Scraping URL: {}", url);

    // Validate the original input URL before any rewrite so callers get a clear
    // error for malformed input rather than a confusingly-rewritten one.
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow!("Invalid URL: must start with http:// or https://"));
    }

    let fetch_url = rewrite_url_for_clean_content(url).unwrap_or_else(|| url.to_string());

    // Check cache
    if let Some(cached) = state.scrape_cache.get(url).await {
        if cached.word_count == 0 || cached.clean_content.trim().is_empty() {
            // Invalidate poor/empty cache entries and recompute
            state.scrape_cache.invalidate(url).await;
        } else {
            return Ok(cached);
        }
    }

    // Concurrency control
    let _permit = state
        .outbound_limit
        .acquire()
        .await
        .expect("semaphore closed");

    // Only use Rust-native scraper with retries
    let rust_scraper = RustScraper::new();
    let url_owned = url.to_string();
    let mut result = retry(
        ExponentialBackoffBuilder::new()
            .with_initial_interval(std::time::Duration::from_millis(200))
            .with_max_interval(std::time::Duration::from_secs(2))
            .with_max_elapsed_time(Some(std::time::Duration::from_secs(6)))
            .build(),
        || async {
            match rust_scraper.scrape_url(&fetch_url).await {
                Ok(r) => Ok(r),
                Err(e) => {
                    // Treat network/temporary HTML parse errors as transient
                    Err(backoff::Error::transient(anyhow!("{}", e)))
                }
            }
        },
    )
    .await?;
    if result.word_count == 0 || result.clean_content.trim().is_empty() {
        info!(
            "Rust-native scraper returned empty content, using fallback for {}",
            url
        );
        result = scrape_url_fallback(state, &fetch_url).await?;
    } else {
        info!("Rust-native scraper succeeded for {}", url);
    }

    // Keep public response URL stable: always return the caller-requested URL,
    // even when we internally fetch via rewritten raw.githubusercontent.com URL.
    result.url = url_owned.clone();

    state
        .scrape_cache
        .insert(url_owned.clone(), result.clone())
        .await;

    // Auto-log to history if memory is enabled (Phase 1)
    if let Some(memory) = &state.memory {
        let summary = format!(
            "{} words, {} code blocks",
            result.word_count,
            result.code_blocks.len()
        );

        // Extract domain from URL
        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()));

        let result_json = serde_json::to_value(&result).unwrap_or_default();

        if let Err(e) = memory
            .log_scrape(
                url.to_string(),
                Some(result.title.clone()),
                summary,
                domain,
                &result_json,
            )
            .await
        {
            tracing::warn!("Failed to log scrape to history: {}", e);
        }
    }

    Ok(result)
}

// Fallback scraper using direct HTTP request (legacy simple mode) -- optional; keeping for troubleshooting
pub async fn scrape_url_fallback(state: &Arc<AppState>, url: &str) -> Result<ScrapeResponse> {
    info!("Using fallback scraper for: {}", url);

    // Make direct HTTP request
    let response = state
        .http_client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (compatible; MCP-Server/1.0)")
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch URL: {}", e))?;

    let status_code = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html")
        .to_string();

    let html = response
        .text()
        .await
        .map_err(|e| anyhow!("Failed to read response body: {}", e))?;

    let document = select::document::Document::from(html.as_str());

    let title = document
        .find(select::predicate::Name("title"))
        .next()
        .map(|n| n.text())
        .unwrap_or_else(|| "No Title".to_string());

    let meta_description = document
        .find(select::predicate::Attr("name", "description"))
        .next()
        .and_then(|n| n.attr("content"))
        .unwrap_or("")
        .to_string();

    let meta_keywords = document
        .find(select::predicate::Attr("name", "keywords"))
        .next()
        .and_then(|n| n.attr("content"))
        .unwrap_or("")
        .to_string();

    let body_html = document
        .find(select::predicate::Name("body"))
        .next()
        .map(|n| n.html())
        .unwrap_or_else(|| html.clone());

    let clean_content = html2text::from_read(body_html.as_bytes(), 80);
    let word_count = clean_content.split_whitespace().count();

    let headings: Vec<Heading> = document
        .find(
            select::predicate::Name("h1")
                .or(select::predicate::Name("h2"))
                .or(select::predicate::Name("h3"))
                .or(select::predicate::Name("h4"))
                .or(select::predicate::Name("h5"))
                .or(select::predicate::Name("h6")),
        )
        .map(|n| Heading {
            level: n.name().unwrap_or("h1").to_string(),
            text: n.text(),
        })
        .collect();

    let links: Vec<Link> = document
        .find(select::predicate::Name("a"))
        .filter_map(|n| {
            n.attr("href").map(|href| Link {
                url: href.to_string(),
                text: n.text(),
            })
        })
        .collect();

    let images: Vec<Image> = document
        .find(select::predicate::Name("img"))
        .filter_map(|n| {
            n.attr("src").map(|src| Image {
                src: src.to_string(),
                alt: n.attr("alt").unwrap_or("").to_string(),
                title: n.attr("title").unwrap_or("").to_string(),
            })
        })
        .collect();

    let result = ScrapeResponse {
        url: url.to_string(),
        title,
        content: html,
        clean_content,
        meta_description,
        meta_keywords,
        headings,
        links,
        images,
        timestamp: chrono::Utc::now().to_rfc3339(),
        status_code,
        content_type,
        word_count,
        language: "unknown".to_string(),
        canonical_url: None,
        site_name: None,
        author: None,
        published_at: None,
        og_title: None,
        og_description: None,
        og_image: None,
        reading_time_minutes: None,
        // New Priority 1 fields (fallback scraper)
        code_blocks: Vec::new(),
        truncated: false,
        actual_chars: 0,
        max_chars_limit: None,
        extraction_score: Some(0.3), // Lower score for fallback
        warnings: vec!["fallback_scraper_used".to_string()],
        domain: url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string())),
    };

    info!("Fallback scraper extracted {} words", result.word_count);
    Ok(result)
}

/// Scrape multiple URLs concurrently with configurable parallelism
/// Returns results for all URLs, including failures
pub async fn scrape_batch(
    state: &Arc<AppState>,
    urls: Vec<String>,
    max_concurrent: Option<usize>,
    max_chars: Option<usize>,
) -> Result<ScrapeBatchResponse> {
    let start_time = Instant::now();
    let total = urls.len();

    // Default to 10 concurrent, max 50 to avoid overwhelming
    let concurrency = max_concurrent.unwrap_or(10).min(50);
    let max_chars = max_chars.unwrap_or(10000);

    info!(
        "Starting batch scrape of {} URLs with concurrency {}",
        total, concurrency
    );

    // Process URLs concurrently using buffered stream
    let state_clone = Arc::clone(state);
    let results: Vec<ScrapeBatchResult> = stream::iter(urls)
        .map(|url| {
            let state = Arc::clone(&state_clone);
            let url_clone = url.clone();
            async move {
                let url_start = Instant::now();

                match scrape_url(&state, &url_clone).await {
                    Ok(mut data) => {
                        // Apply max_chars truncation
                        data.actual_chars = data.clean_content.len();
                        data.max_chars_limit = Some(max_chars);
                        data.truncated = data.clean_content.len() > max_chars;

                        if data.truncated {
                            data.clean_content =
                                data.clean_content.chars().take(max_chars).collect();
                            if !data.warnings.contains(&"content_truncated".to_string()) {
                                data.warnings.push("content_truncated".to_string());
                            }
                        }

                        ScrapeBatchResult {
                            url: url_clone,
                            success: true,
                            data: Some(data),
                            error: None,
                            duration_ms: url_start.elapsed().as_millis() as u64,
                        }
                    }
                    Err(e) => ScrapeBatchResult {
                        url: url_clone,
                        success: false,
                        data: None,
                        error: Some(e.to_string()),
                        duration_ms: url_start.elapsed().as_millis() as u64,
                    },
                }
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    let successful = results.iter().filter(|r| r.success).count();
    let failed = results.iter().filter(|r| !r.success).count();
    let total_duration_ms = start_time.elapsed().as_millis() as u64;

    info!(
        "Batch scrape completed: {}/{} successful, {} failed, {}ms total",
        successful, total, failed, total_duration_ms
    );

    Ok(ScrapeBatchResponse {
        total,
        successful,
        failed,
        total_duration_ms,
        results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        assert!(rewrite_url_for_clean_content(
            "https://raw.githubusercontent.com/user/repo/main/a.rs"
        )
        .is_none());
    }

    /// A host that merely contains "github.com" as a substring (e.g. "notgithub.com") must NOT be rewritten.
    #[test]
    fn test_rewrite_url_non_github_host_substring_should_not_rewrite() {
        // Host is "notgithub.com", not exactly "github.com"
        let url = "https://notgithub.com/user/repo/blob/main/README.md";
        assert!(
            rewrite_url_for_clean_content(url).is_none(),
            "hosts that merely contain 'github.com' as a substring must not be rewritten"
        );
    }

    /// A GitHub Enterprise host (e.g. "github.mycompany.com") must NOT be rewritten.
    #[test]
    fn test_rewrite_url_github_enterprise_host_should_not_rewrite() {
        let url = "https://github.mycompany.com/user/repo/blob/main/README.md";
        assert!(
            rewrite_url_for_clean_content(url).is_none(),
            "github enterprise hosts must not be rewritten (only github.com exactly)"
        );
    }

    /// A URL where '/blob/' appears only in the query string must NOT be rewritten.
    #[test]
    fn test_rewrite_url_blob_in_query_string_should_not_rewrite() {
        // The path is "/search", the query string contains "/blob/"
        let url = "https://github.com/search?q=%2Fblob%2Fmain&type=code";
        assert!(
            rewrite_url_for_clean_content(url).is_none(),
            "'/blob/' appearing only in the query string must not trigger a rewrite"
        );
        // Unencoded variant just in case a caller passes it raw
        let url2 = "https://github.com/search?q=/blob/main&type=code";
        assert!(
            rewrite_url_for_clean_content(url2).is_none(),
            "unencoded '/blob/' in query string must not trigger a rewrite"
        );
    }

    /// After rewriting, the original URL is preserved in ScrapeResponse.url.
    /// We test rewrite_url_for_clean_content produces a raw URL so the caller can
    /// still use the original URL when constructing the response.
    #[test]
    fn test_rewrite_returns_raw_url_while_original_is_separate() {
        let original = "https://github.com/rust-lang/rust/blob/master/src/lib.rs";
        let rewritten = rewrite_url_for_clean_content(original);
        assert!(
            rewritten.is_some(),
            "should produce a rewritten URL for valid blob path"
        );
        let rw = rewritten.unwrap();
        assert!(
            rw.contains("raw.githubusercontent.com"),
            "rewritten URL must use raw host"
        );
        // The original URL must not equal the rewritten URL
        assert_ne!(rw, original, "rewritten URL must differ from original");
    }

    #[tokio::test]
    async fn test_scrape_url_fallback() {
        let state = Arc::new(AppState::new(
            "http://localhost:8888".to_string(),
            reqwest::Client::new(),
        ));

        let result = scrape_url_fallback(&state, "https://httpbin.org/html").await;

        match result {
            Ok(content) => {
                assert!(!content.title.is_empty(), "Title should not be empty");
                assert!(
                    !content.clean_content.is_empty(),
                    "Content should not be empty"
                );
                assert_eq!(content.status_code, 200, "Status code should be 200");
            }
            Err(e) => {
                println!("Fallback scraper test failed: {}", e);
            }
        }
    }
}
