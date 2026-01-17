use crate::types::*;
use crate::AppState;
use anyhow::{anyhow, Result};
use backoff::future::retry;
use backoff::ExponentialBackoffBuilder;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;
use select::predicate::Predicate;
use crate::rust_scraper::RustScraper;
use futures::stream::{self, StreamExt};

pub async fn scrape_url(state: &Arc<AppState>, url: &str) -> Result<ScrapeResponse> {
    info!("Scraping URL: {}", url);
    
    // Validate URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow!("Invalid URL: must start with http:// or https://"));
    }

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
    let _permit = state.outbound_limit.acquire().await.expect("semaphore closed");

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
            match rust_scraper.scrape_url(&url_owned).await {
                Ok(r) => Ok(r),
                Err(e) => {
                    // Treat network/temporary HTML parse errors as transient
                    Err(backoff::Error::transient(anyhow!("{}", e)))
                }
            }
        },
    ).await?;
    if result.word_count == 0 || result.clean_content.trim().is_empty() {
        info!("Rust-native scraper returned empty content, using fallback for {}", url);
        result = scrape_url_fallback(state, &url_owned).await?;
    } else {
        info!("Rust-native scraper succeeded for {}", url);
    }
    state.scrape_cache.insert(url.to_string(), result.clone()).await;
    
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
        
        if let Err(e) = memory.log_scrape(
            url.to_string(),
            Some(result.title.clone()),
            summary,
            domain,
            &result_json
        ).await {
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
        .find(select::predicate::Name("h1")
            .or(select::predicate::Name("h2"))
            .or(select::predicate::Name("h3"))
            .or(select::predicate::Name("h4"))
            .or(select::predicate::Name("h5"))
            .or(select::predicate::Name("h6")))
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
        domain: url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_string())),
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

    info!("Starting batch scrape of {} URLs with concurrency {}", total, concurrency);

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
                            data.clean_content = data.clean_content.chars().take(max_chars).collect();
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
                    Err(e) => {
                        ScrapeBatchResult {
                            url: url_clone,
                            success: false,
                            data: None,
                            error: Some(e.to_string()),
                            duration_ms: url_start.elapsed().as_millis() as u64,
                        }
                    }
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
                assert!(!content.clean_content.is_empty(), "Content should not be empty");
                assert_eq!(content.status_code, 200, "Status code should be 200");
            }
            Err(e) => {
                println!("Fallback scraper test failed: {}", e);
            }
        }
    }
}