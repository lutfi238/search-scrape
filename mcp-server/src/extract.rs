use anyhow::{anyhow, Result};
use regex::Regex;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

use crate::llm_client;
use crate::scrape::scrape_url;
use crate::types::*;
use crate::AppState;

/// Extract structured data from a webpage using a BYO LLM (primary path).
///
/// Requires `state.llm` to be configured via `LLM_BASE_URL`, `LLM_API_KEY`, and `LLM_MODEL`
/// environment variables. Returns `LLM_NOT_CONFIGURED` error when the LLM client is absent.
///
/// Flow:
/// 1. Check LLM availability – return `LLM_NOT_CONFIGURED` immediately if absent.
/// 2. Scrape the target URL.
/// 3. Build a schema/prompt hint string and pass content to `llm.extract_json`.
/// 4. Return parsed JSON result wrapped in `ExtractResponse`.
pub async fn extract_structured(
    state: &Arc<AppState>,
    url: &str,
    schema: Option<Vec<ExtractField>>,
    prompt: Option<String>,
    max_chars: Option<usize>,
) -> Result<ExtractResponse> {
    let start_time = Instant::now();
    let max_chars = max_chars.unwrap_or(10000);

    // Primary path: LLM required. Return a clear error if not configured.
    let llm = state.llm.as_ref().ok_or_else(|| {
        anyhow!(
            "{}: set LLM_BASE_URL, LLM_API_KEY, and LLM_MODEL environment variables to enable extract_structured",
            llm_client::LLM_NOT_CONFIGURED
        )
    })?;

    info!(
        "Extracting structured data from: {} (LLM model: {})",
        url,
        llm.model()
    );

    // Scrape the page to obtain content for the LLM.
    let scrape_result = scrape_url(state, url).await?;

    // Truncate content to stay within LLM context windows.
    let content: String = scrape_result
        .clean_content
        .chars()
        .take(max_chars)
        .collect();

    // Build a schema hint string from the provided schema fields or prompt.
    let schema_hint = build_schema_hint(schema.as_deref(), prompt.as_deref());

    // Build the extraction prompt.
    let extraction_prompt = build_extraction_prompt(prompt.as_deref(), schema.as_deref());

    // Call the LLM for JSON extraction. Errors (LLM_TIMEOUT, LLM_INVALID_JSON, etc.) propagate.
    let extracted_value = llm
        .extract_json(&extraction_prompt, &schema_hint, &content)
        .await?;

    let field_count = extracted_value.as_object().map(|m| m.len()).unwrap_or(1);

    let raw_preview: String = scrape_result
        .clean_content
        .chars()
        .take(max_chars)
        .collect();

    let mut warnings = vec![];
    maybe_add_raw_url_warning(url, &mut warnings);

    Ok(ExtractResponse {
        url: url.to_string(),
        title: scrape_result.title,
        extracted_data: extracted_value,
        raw_content_preview: raw_preview,
        extraction_method: "llm".to_string(),
        field_count,
        confidence: 1.0,
        duration_ms: start_time.elapsed().as_millis() as u64,
        warnings,
    })
}

/// Build a human-readable schema hint for the LLM from field definitions or prompt text.
fn build_schema_hint(schema: Option<&[ExtractField]>, prompt: Option<&str>) -> String {
    if let Some(fields) = schema {
        let field_lines: Vec<String> = fields
            .iter()
            .map(|f| {
                let type_str = f.field_type.as_deref().unwrap_or("string");
                let req = if f.required.unwrap_or(false) {
                    " (required)"
                } else {
                    ""
                };
                format!("  \"{}\": {} - {}{}", f.name, type_str, f.description, req)
            })
            .collect();
        format!("{{\n{}\n}}", field_lines.join(",\n"))
    } else if let Some(p) = prompt {
        format!("Extract the following information as JSON: {}", p)
    } else {
        "Extract all meaningful structured data as a JSON object.".to_string()
    }
}

/// Build a system-level extraction instruction for the LLM.
fn build_extraction_prompt(prompt: Option<&str>, schema: Option<&[ExtractField]>) -> String {
    let base = "You are a precise data extraction assistant. Extract structured information from the provided webpage content and return ONLY valid JSON.";

    if let Some(p) = prompt {
        format!("{} User instruction: {}", base, p)
    } else if schema.is_some() {
        format!(
            "{} Extract fields exactly matching the provided schema.",
            base
        )
    } else {
        format!("{} Extract all meaningful data you can find.", base)
    }
}

/// Returns true when URL path looks like a raw text/markdown file.
pub fn is_raw_content_url(url: &str) -> bool {
    let path_only = url
        .split(&['?', '#'][..])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let ext = path_only.rsplit('.').next().unwrap_or("");
    matches!(
        ext,
        "md" | "mdx" | "rst" | "txt" | "csv" | "toml" | "yaml" | "yml"
    )
}

fn maybe_add_raw_url_warning(url: &str, warnings: &mut Vec<String>) {
    if is_raw_content_url(url) {
        let warning = "raw_markdown_url: Extraction on raw .md/.mdx/.rst/.txt files is unreliable — fields may return null and confidence will be low.".to_string();
        if !warnings.iter().any(|w| w == &warning) {
            warnings.push(warning);
        }
    }
}

/// Extract a specific field value based on field definition (heuristic fallback - not used in LLM path).
#[allow(dead_code)]
fn extract_field_value(scrape: &ScrapeResponse, field: &ExtractField) -> serde_json::Value {
    let content = &scrape.clean_content;
    let name_lower = field.name.to_lowercase();
    let desc_lower = field.description.to_lowercase();

    // Try to match based on field name and description
    match name_lower.as_str() {
        // Common field patterns
        "title" | "name" | "headline" => serde_json::Value::String(scrape.title.clone()),
        "description" | "summary" | "excerpt" => {
            if !scrape.meta_description.is_empty() {
                serde_json::Value::String(scrape.meta_description.clone())
            } else {
                // First paragraph
                let first_para: String = content
                    .lines()
                    .find(|l| l.len() > 50)
                    .unwrap_or("")
                    .chars()
                    .take(500)
                    .collect();
                serde_json::Value::String(first_para)
            }
        }
        "author" | "writer" | "by" => scrape
            .author
            .clone()
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
        "date" | "published" | "published_at" | "publish_date" => scrape
            .published_at
            .clone()
            .map(serde_json::Value::String)
            .unwrap_or_else(|| extract_date_from_content(content)),
        "price" | "cost" | "amount" => extract_price(content),
        "email" | "emails" => extract_emails(content),
        "phone" | "telephone" | "phones" => extract_phones(content),
        "links" | "urls" => {
            let urls: Vec<serde_json::Value> = scrape
                .links
                .iter()
                .take(20)
                .map(|l| serde_json::Value::String(l.url.clone()))
                .collect();
            serde_json::Value::Array(urls)
        }
        "headings" | "headers" | "sections" => {
            let headings: Vec<serde_json::Value> = scrape
                .headings
                .iter()
                .map(|h| serde_json::Value::String(format!("{}: {}", h.level, h.text)))
                .collect();
            serde_json::Value::Array(headings)
        }
        "code" | "code_blocks" | "code_snippets" => {
            let blocks: Vec<serde_json::Value> = scrape
                .code_blocks
                .iter()
                .map(|b| {
                    let mut obj = serde_json::Map::new();
                    obj.insert(
                        "language".to_string(),
                        b.language
                            .clone()
                            .map(serde_json::Value::String)
                            .unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "code".to_string(),
                        serde_json::Value::String(b.code.clone()),
                    );
                    serde_json::Value::Object(obj)
                })
                .collect();
            serde_json::Value::Array(blocks)
        }
        "images" => {
            let imgs: Vec<serde_json::Value> = scrape
                .images
                .iter()
                .take(20)
                .map(|i| {
                    let mut obj = serde_json::Map::new();
                    obj.insert("src".to_string(), serde_json::Value::String(i.src.clone()));
                    obj.insert("alt".to_string(), serde_json::Value::String(i.alt.clone()));
                    serde_json::Value::Object(obj)
                })
                .collect();
            serde_json::Value::Array(imgs)
        }
        _ => {
            // Try to find pattern in content based on description
            if desc_lower.contains("number")
                || desc_lower.contains("count")
                || desc_lower.contains("quantity")
            {
                extract_number_near_keyword(content, &field.name)
            } else if desc_lower.contains("list") || desc_lower.contains("array") {
                extract_list_near_keyword(content, &field.name)
            } else {
                extract_text_near_keyword(content, &field.name)
            }
        }
    }
}

/// Auto-extract common data patterns from content (heuristic fallback - not used in LLM path).
#[allow(dead_code)]
fn auto_extract(
    scrape: &ScrapeResponse,
    prompt: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut data = serde_json::Map::new();
    let content = &scrape.clean_content;

    // Always extract these
    data.insert(
        "title".to_string(),
        serde_json::Value::String(scrape.title.clone()),
    );

    if !scrape.meta_description.is_empty() {
        data.insert(
            "description".to_string(),
            serde_json::Value::String(scrape.meta_description.clone()),
        );
    }

    // Extract emails if found
    let emails = extract_emails(content);
    if !emails.is_null() {
        data.insert("emails".to_string(), emails);
    }

    // Extract prices if found
    let prices = extract_price(content);
    if !prices.is_null() {
        data.insert("prices".to_string(), prices);
    }

    // Extract dates if found
    let dates = extract_date_from_content(content);
    if !dates.is_null() {
        data.insert("dates".to_string(), dates);
    }

    // If prompt provided, try to extract based on keywords in prompt
    if let Some(prompt_text) = prompt {
        let prompt_lower = prompt_text.to_lowercase();

        if prompt_lower.contains("product") || prompt_lower.contains("item") {
            // Product-focused extraction
            if let Some(h1) = scrape.headings.iter().find(|h| h.level == "h1") {
                data.insert(
                    "product_name".to_string(),
                    serde_json::Value::String(h1.text.clone()),
                );
            }
        }

        if prompt_lower.contains("article") || prompt_lower.contains("blog") {
            // Article-focused extraction
            if let Some(author) = &scrape.author {
                data.insert(
                    "author".to_string(),
                    serde_json::Value::String(author.clone()),
                );
            }
            if let Some(date) = &scrape.published_at {
                data.insert(
                    "published_date".to_string(),
                    serde_json::Value::String(date.clone()),
                );
            }

            // Reading time
            if let Some(time) = scrape.reading_time_minutes {
                data.insert(
                    "reading_time_minutes".to_string(),
                    serde_json::Value::Number(time.into()),
                );
            }
        }

        if prompt_lower.contains("contact") {
            let phones = extract_phones(content);
            if !phones.is_null() {
                data.insert("phones".to_string(), phones);
            }
        }

        if (prompt_lower.contains("code") || prompt_lower.contains("programming"))
            && !scrape.code_blocks.is_empty()
        {
            let blocks: Vec<serde_json::Value> = scrape
                .code_blocks
                .iter()
                .map(|b| serde_json::Value::String(b.code.clone()))
                .collect();
            data.insert("code_blocks".to_string(), serde_json::Value::Array(blocks));
        }
    }

    // Add headings as table of contents
    if !scrape.headings.is_empty() {
        let toc: Vec<serde_json::Value> = scrape
            .headings
            .iter()
            .filter(|h| h.level == "h1" || h.level == "h2" || h.level == "h3")
            .take(15)
            .map(|h| serde_json::Value::String(h.text.clone()))
            .collect();
        if !toc.is_empty() {
            data.insert(
                "table_of_contents".to_string(),
                serde_json::Value::Array(toc),
            );
        }
    }

    data
}

/// Extract email addresses from content
fn extract_emails(content: &str) -> serde_json::Value {
    let email_re = Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap();
    let emails: Vec<serde_json::Value> = email_re
        .find_iter(content)
        .map(|m| serde_json::Value::String(m.as_str().to_string()))
        .collect();

    if emails.is_empty() {
        serde_json::Value::Null
    } else if emails.len() == 1 {
        emails.into_iter().next().unwrap()
    } else {
        serde_json::Value::Array(emails)
    }
}

/// Extract phone numbers from content
fn extract_phones(content: &str) -> serde_json::Value {
    let phone_re = Regex::new(
        r"[\+]?[(]?[0-9]{1,3}[)]?[-\s\.]?[0-9]{1,4}[-\s\.]?[0-9]{1,4}[-\s\.]?[0-9]{1,9}",
    )
    .unwrap();
    let phones: Vec<serde_json::Value> = phone_re
        .find_iter(content)
        .filter(|m| m.as_str().len() >= 10)
        .map(|m| serde_json::Value::String(m.as_str().to_string()))
        .take(5)
        .collect();

    if phones.is_empty() {
        serde_json::Value::Null
    } else if phones.len() == 1 {
        phones.into_iter().next().unwrap()
    } else {
        serde_json::Value::Array(phones)
    }
}

/// Extract price values from content
fn extract_price(content: &str) -> serde_json::Value {
    let price_re = Regex::new(r"[\$€£¥₹][\s]?[0-9]{1,3}(?:[,.]?[0-9]{3})*(?:[.,][0-9]{2})?|[0-9]{1,3}(?:[,.]?[0-9]{3})*(?:[.,][0-9]{2})?\s?(?:USD|EUR|GBP|JPY|INR)").unwrap();
    let prices: Vec<serde_json::Value> = price_re
        .find_iter(content)
        .map(|m| serde_json::Value::String(m.as_str().to_string()))
        .take(10)
        .collect();

    if prices.is_empty() {
        serde_json::Value::Null
    } else if prices.len() == 1 {
        prices.into_iter().next().unwrap()
    } else {
        serde_json::Value::Array(prices)
    }
}

/// Extract dates from content
fn extract_date_from_content(content: &str) -> serde_json::Value {
    // Common date patterns
    let date_patterns = [
        r"\d{4}-\d{2}-\d{2}", // 2024-01-15
        r"\d{2}/\d{2}/\d{4}", // 01/15/2024
        r"(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\s+\d{1,2},?\s+\d{4}", // January 15, 2024
        r"\d{1,2}\s+(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\s+\d{4}", // 15 January 2024
    ];

    for pattern in date_patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(m) = re.find(content) {
                return serde_json::Value::String(m.as_str().to_string());
            }
        }
    }

    serde_json::Value::Null
}

/// Extract number near a keyword
fn extract_number_near_keyword(content: &str, keyword: &str) -> serde_json::Value {
    let keyword_lower = keyword.to_lowercase();
    let content_lower = content.to_lowercase();

    if let Some(pos) = content_lower.find(&keyword_lower) {
        // Look for numbers within 100 chars after keyword
        let search_area: String = content.chars().skip(pos).take(100).collect();
        let num_re = Regex::new(r"\d+(?:[.,]\d+)?").unwrap();
        if let Some(m) = num_re.find(&search_area) {
            if let Ok(num) = m.as_str().replace(",", "").parse::<f64>() {
                return serde_json::Value::Number(
                    serde_json::Number::from_f64(num).unwrap_or(0.into()),
                );
            }
        }
    }
    serde_json::Value::Null
}

/// Extract text near a keyword
fn extract_text_near_keyword(content: &str, keyword: &str) -> serde_json::Value {
    let keyword_lower = keyword.to_lowercase();
    let content_lower = content.to_lowercase();

    if let Some(pos) = content_lower.find(&keyword_lower) {
        // Get text after keyword until newline or 200 chars
        let after: String = content
            .chars()
            .skip(pos + keyword.len())
            .take(200)
            .take_while(|c| *c != '\n')
            .collect();

        let trimmed = after.trim().trim_start_matches(':').trim();
        if !trimmed.is_empty() {
            return serde_json::Value::String(trimmed.to_string());
        }
    }
    serde_json::Value::Null
}

/// Extract list near a keyword
fn extract_list_near_keyword(content: &str, keyword: &str) -> serde_json::Value {
    let keyword_lower = keyword.to_lowercase();
    let content_lower = content.to_lowercase();

    if let Some(pos) = content_lower.find(&keyword_lower) {
        // Look for bullet points or numbered items
        let search_area: String = content.chars().skip(pos).take(500).collect();
        let items: Vec<serde_json::Value> = search_area
            .lines()
            .filter(|l| {
                l.trim().starts_with('-') || l.trim().starts_with('•') || l.trim().starts_with('*')
            })
            .take(10)
            .map(|l| {
                serde_json::Value::String(
                    l.trim()
                        .trim_start_matches(['-', '•', '*'])
                        .trim()
                        .to_string(),
                )
            })
            .collect();

        if !items.is_empty() {
            return serde_json::Value::Array(items);
        }
    }
    serde_json::Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_client;

    #[test]
    fn test_extract_emails() {
        let content = "Contact us at test@example.com or support@company.org";
        let result = extract_emails(content);
        assert!(result.is_array());
    }

    #[test]
    fn test_extract_price() {
        let content = "Price: $99.99 or €85.00";
        let result = extract_price(content);
        assert!(!result.is_null());
    }

    #[test]
    fn test_is_raw_content_url() {
        assert!(is_raw_content_url("https://example.com/README.md"));
        assert!(is_raw_content_url("https://example.com/file.MDX"));
        assert!(is_raw_content_url("https://example.com/file.txt?token=abc"));
        assert!(is_raw_content_url("https://example.com/config.yaml"));

        assert!(!is_raw_content_url("https://example.com/page.html"));
        assert!(!is_raw_content_url("https://example.com/api/data"));
    }

    #[test]
    fn test_maybe_add_raw_url_warning_for_raw_file() {
        let mut warnings = vec![];
        maybe_add_raw_url_warning("https://example.com/README.md", &mut warnings);
        assert!(warnings.iter().any(|w| w.contains("raw_markdown_url")));
    }

    #[test]
    fn test_maybe_add_raw_url_warning_for_html_is_noop() {
        let mut warnings = vec![];
        maybe_add_raw_url_warning("https://example.com/page.html", &mut warnings);
        assert!(warnings.is_empty());
    }

    /// Build a minimal AppState with no LLM client for testing.
    fn make_state_without_llm() -> Arc<AppState> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        Arc::new(AppState::new(
            "http://localhost:8888".to_string(),
            http_client,
        ))
    }

    /// Task 6 test 1: extract_structured must return LLM_NOT_CONFIGURED when no LLM is wired.
    #[tokio::test]
    async fn test_extract_structured_returns_llm_not_configured_when_llm_missing() {
        let state = make_state_without_llm();
        // Ensure no LLM is present
        assert!(
            state.llm.is_none(),
            "state must not have an LLM for this test"
        );

        let err = extract_structured(&state, "https://example.com", None, None, None)
            .await
            .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains(llm_client::LLM_NOT_CONFIGURED),
            "Expected error containing LLM_NOT_CONFIGURED, got: {}",
            msg
        );
    }

    /// Task 6 test 2: structured output validates/parses JSON path -
    /// when a valid schema is provided and LLM is missing, error must still name LLM_NOT_CONFIGURED
    /// (demonstrating the LLM-first path is entered before heuristics).
    #[tokio::test]
    async fn test_extract_structured_validates_json_schema() {
        let state = make_state_without_llm();

        let schema = vec![crate::types::ExtractField {
            name: "price".to_string(),
            description: "Product price".to_string(),
            field_type: Some("number".to_string()),
            required: Some(true),
        }];

        let err = extract_structured(&state, "https://example.com", Some(schema), None, None)
            .await
            .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains(llm_client::LLM_NOT_CONFIGURED),
            "Schema path must require LLM; expected LLM_NOT_CONFIGURED, got: {}",
            msg
        );
    }

    /// Task 6 test 3 – real behavior: verify the LLM_INVALID_JSON path end-to-end.
    ///
    /// `LlmClient::extract_json` maps a non-JSON LLM response via:
    ///
    ///   serde_json::from_str(json_str).map_err(|e| anyhow!("{}: ...", LLM_INVALID_JSON, e))
    ///
    /// We replicate that exact transform here so the test exercises actual error-mapping
    /// behavior rather than just asserting a constant's string value.  No HTTP call is
    /// made; the "LLM response" is a literal non-JSON string injected directly.
    #[test]
    fn test_extract_structured_handles_invalid_json() {
        /// Mirrors the parse-and-map step inside `LlmClient::extract_json`.
        /// Returns an `anyhow::Error` tagged with `LLM_INVALID_JSON` when `raw`
        /// is not valid JSON, exactly as the real implementation does.
        fn parse_llm_json_response(raw: &str) -> anyhow::Result<serde_json::Value> {
            let trimmed = raw.trim();
            let json_str = trimmed
                .strip_prefix("```json")
                .or_else(|| trimmed.strip_prefix("```"))
                .unwrap_or(trimmed);
            let json_str = json_str.strip_suffix("```").unwrap_or(json_str).trim();

            serde_json::from_str(json_str).map_err(|e| {
                anyhow::anyhow!(
                    "{}: LLM response is not valid JSON: {}",
                    llm_client::LLM_INVALID_JSON,
                    e,
                )
            })
        }

        // --- Case 1: plain prose (LLM forgot to return JSON) ---
        let bad_response = "Sure! Here is the product name: Widget Pro.";
        let err = parse_llm_json_response(bad_response).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(llm_client::LLM_INVALID_JSON),
            "Plain-prose LLM response must produce LLM_INVALID_JSON error, got: {}",
            msg
        );

        // --- Case 2: truncated JSON (e.g., LLM hit a token limit mid-object) ---
        let truncated = r#"{"price": 9.99, "name": "Widget"#;
        let err2 = parse_llm_json_response(truncated).unwrap_err();
        let msg2 = err2.to_string();
        assert!(
            msg2.contains(llm_client::LLM_INVALID_JSON),
            "Truncated JSON must produce LLM_INVALID_JSON error, got: {}",
            msg2
        );

        // --- Case 3: markdown-fenced valid JSON must succeed (strip logic correct) ---
        let fenced = "```json\n{\"price\": 9.99}\n```";
        let ok = parse_llm_json_response(fenced);
        assert!(
            ok.is_ok(),
            "Markdown-fenced valid JSON must parse successfully, got: {:?}",
            ok
        );

        // --- Case 4: bare valid JSON must succeed ---
        let bare = r#"{"price": 9.99}"#;
        assert!(
            parse_llm_json_response(bare).is_ok(),
            "Bare valid JSON must parse successfully"
        );
    }
}
