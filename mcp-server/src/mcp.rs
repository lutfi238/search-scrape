use crate::types::*;
use crate::{crawl, extract, history, research, scrape, search, AppState};
use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

#[derive(Debug, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpToolsResponse {
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpCallRequest {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpCallResponse {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

fn cap_json_payload(mut json_str: String, max_chars: usize) -> String {
    if json_str.chars().count() <= max_chars {
        return json_str;
    }

    let full_chars = json_str.chars().count();
    let marker = format!(
        " // JSON_PAYLOAD_TRUNCATED: full_payload_chars={}, max_chars={}",
        full_chars, max_chars
    );

    let marker_len = marker.chars().count();
    if max_chars <= marker_len {
        return marker.chars().take(max_chars).collect();
    }

    let keep = max_chars - marker_len;
    json_str = json_str.chars().take(keep).collect();
    json_str.push_str(&marker);
    json_str
}

fn push_warning_unique(warnings: &mut Vec<String>, warning: &str) {
    if !warnings.iter().any(|w| w == warning) {
        warnings.push(warning.to_string());
    }
}

fn maybe_add_raw_url_warning(url: &str, warnings: &mut Vec<String>) {
    if crate::extract::is_raw_content_url(url) {
        let warning = "raw_markdown_url: Extraction on raw .md/.mdx/.rst/.txt files is unreliable — fields may return null and confidence will be low.";
        push_warning_unique(warnings, warning);
    }
}

fn mcp_text_response(text: String, is_error: bool) -> Json<McpCallResponse> {
    Json(McpCallResponse {
        content: vec![McpContent {
            content_type: "text".to_string(),
            text,
        }],
        is_error,
    })
}

fn parse_string_list_arg(arguments: &serde_json::Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn map_crawl_config(
    arguments: &serde_json::Value,
    default_same_domain_only: bool,
) -> crawl::CrawlConfig {
    let max_depth = arguments
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(3);
    let max_pages = arguments
        .get("max_pages")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(50);
    let max_concurrent = arguments
        .get("max_concurrent")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(5);
    let same_domain_only = arguments
        .get("same_domain_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(default_same_domain_only);
    let max_chars_per_page = arguments
        .get("max_chars_per_page")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(5000);

    let include_patterns = parse_string_list_arg(arguments, "include_patterns");
    let exclude_patterns = parse_string_list_arg(arguments, "exclude_patterns");

    let mut config = crawl::CrawlConfig {
        max_depth: max_depth.min(10),
        max_pages: max_pages.min(500),
        max_concurrent: max_concurrent.min(20),
        same_domain_only,
        max_chars_per_page,
        ..crawl::CrawlConfig::default()
    };

    if !include_patterns.is_empty() {
        config.include_patterns = include_patterns;
    }

    if !exclude_patterns.is_empty() {
        for pattern in exclude_patterns {
            if !config.exclude_patterns.contains(&pattern) {
                config.exclude_patterns.push(pattern);
            }
        }
    }

    config
}

fn map_website_crawl_config(limit: usize, _include_subdomains: bool) -> crawl::CrawlConfig {
    crawl::CrawlConfig {
        max_pages: limit.min(5000),
        max_depth: 5,
        max_concurrent: 10,
        same_domain_only: true,
        max_chars_per_page: 100,
        ..crawl::CrawlConfig::default()
    }
}

pub async fn list_tools() -> Json<McpToolsResponse> {
    let tools = vec![
        McpTool {
            name: "search_web".to_string(),
            description: "Search the web using SearXNG federated search. AGENT GUIDANCE: (1) Set max_results=5-10 for quick lookups, 20-50 for comprehensive research. (2) Use time_range='week' or 'month' for recent topics. (3) Use categories='it' for tech, 'news' for current events, 'science' for research. (4) Check 'answers' field for instant facts before reading snippets. (5) If you see 'Did you mean' corrections, retry with the suggested spelling. (6) If unresponsive_engines > 3, consider retrying.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query. TIP: Use specific terms and quotes for exact phrases. Example: 'rust async' instead of just 'rust'"
                    },
                    "engines": {
                        "type": "string",
                        "description": "Comma-separated engines (e.g., 'google,bing'). TIP: Omit for default. Use 'google,bing' for English content, add 'duckduckgo' for privacy-focused results"
                    },
                    "categories": {
                        "type": "string",
                        "description": "Comma-separated categories. WHEN TO USE: 'it' for programming/tech, 'news' for current events, 'science' for research papers, 'general' for mixed. Omit for all categories"
                    },
                    "language": {
                        "type": "string",
                        "description": "Language code (e.g., 'en', 'es', 'fr'). TIP: Use 'en' for English-only results, omit for multilingual"
                    },
                    "safesearch": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 2,
                        "description": "Safe search: 0=off, 1=moderate (recommended), 2=strict. Default env setting usually sufficient"
                    },
                    "time_range": {
                        "type": "string",
                        "description": "Filter by recency. WHEN TO USE: 'day' for breaking news, 'week' for current events, 'month' for recent tech/trends, 'year' for last 12 months. Omit for all-time results"
                    },
                    "pageno": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Page number for pagination. TIP: Start with page 1, use page 2+ only if initial results insufficient"
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 10,
                        "description": "Max results to return. GUIDANCE: 5-10 for quick facts, 15-25 for balanced research, 30-50 for comprehensive surveys. Default 10 is good for most queries. Higher = more tokens"
                    },
                    "snippet_chars": {
                        "type": "integer",
                        "minimum": 20,
                        "maximum": 2000,
                        "description": "Max characters per result snippet. Default: 200"
                    }
                },
                "required": ["query"]
            }),
        },
        McpTool {
            name: "scrape_url".to_string(),
            description: "Scrape and extract clean content from URLs. AGENT GUIDANCE: (1) Set max_chars=3000-5000 for summaries, 10000-20000 for full articles, 30000+ for documentation. (2) Keep content_links_only=true (default) to get only relevant links. (3) Check word_count - if <50, page may be JS-heavy or paywalled. (4) Use [N] citation markers in content to reference specific sources. (5) For docs sites, increase max_chars to capture full tutorials.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Full URL to scrape. TIP: Works best with article/blog/docs pages. May have limited content for JS-heavy sites or paywalls"
                    },
                    "content_links_only": {
                        "type": "boolean",
                        "description": "Extract main content links only (true, default) or all page links (false). GUIDANCE: Keep true for articles/blogs to avoid nav clutter. Set false only when you need site-wide links like sitemaps",
                        "default": true
                    },
                    "max_links": {
                        "type": "integer",
                        "description": "Max links in Sources section. GUIDANCE: 20-30 for focused articles, 50-100 (default) for comprehensive pages, 200+ for navigation-heavy docs. Lower = faster response",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 100
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Max content length. WHEN TO ADJUST: 3000-5000 for quick summaries, 10000 (default) for standard articles, 20000-30000 for long-form content, 40000+ for full documentation pages. Truncated content shows a warning",
                        "minimum": 100,
                        "maximum": 50000,
                        "default": 10000
                    },
                    "output_format": {
                        "type": "string",
                        "enum": ["text", "json"],
                        "description": "Output format. 'text' (default) returns formatted markdown for humans. 'json' returns structured JSON for agents/parsing. AGENT TIP: Use 'json' to get extraction_score, truncated flag, code_blocks array, and all metadata as machine-readable fields",
                        "default": "text"
                    },
                    "short_content_threshold": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Word-count threshold for adding 'short_content' warning. Default: 50"
                    },
                    "extraction_score_threshold": {
                        "type": "number",
                        "minimum": 0.0,
                        "maximum": 1.0,
                        "description": "Score threshold for adding 'low_extraction_score' warning. Default: 0.4"
                    },
                    "max_headings": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Maximum headings to include in output. Omit to keep current default behavior"
                    },
                    "max_images": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Maximum images to include in output payload. Omit to keep current default behavior"
                    }
                },
                "required": ["url"]
            }),
        },
        McpTool {
            name: "research_history".to_string(),
            description: "Search past research using semantic similarity (vector search). Finds related searches/scrapes even with different wording. NOTE: Only available when Qdrant is running (QDRANT_URL configured).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Topic or question to search in history. Use natural language."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10,
                        "description": "Max number of results to return."
                    },
                    "threshold": {
                        "type": "number",
                        "minimum": 0.0,
                        "maximum": 1.0,
                        "default": 0.7,
                        "description": "Similarity threshold (0-1)."
                    },
                    "entry_type": {
                        "type": "string",
                        "enum": ["search", "scrape"],
                        "description": "Filter by entry type."
                    }
                },
                "required": ["query"]
            }),
        },
        McpTool {
            name: "scrape_batch".to_string(),
            description: "Scrape multiple URLs concurrently in a single request.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "urls": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of URLs to scrape. Maximum 100 URLs per request.",
                        "minItems": 1,
                        "maxItems": 100
                    },
                    "max_concurrent": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10,
                        "description": "Max concurrent requests."
                    },
                    "max_chars": {
                        "type": "integer",
                        "minimum": 100,
                        "maximum": 50000,
                        "default": 10000,
                        "description": "Max content chars per URL."
                    },
                    "output_format": {
                        "type": "string",
                        "enum": ["text", "json"],
                        "default": "json",
                        "description": "Output format."
                    }
                },
                "required": ["urls"]
            }),
        },
        // scrape_batch_async
        McpTool {
            name: "scrape_batch_async".to_string(),
            description: "Start an async batch scrape job. Returns a job_id to poll status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "urls": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of URLs to scrape (max 100)"
                    },
                    "max_concurrent": {
                        "type": "integer",
                        "description": "Max concurrent requests (default 10, max 50)"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Max chars per URL (default 10000)"
                    }
                },
                "required": ["urls"]
            }),
        },
        // check_batch_status
        McpTool {
            name: "check_batch_status".to_string(),
            description: "Check status of an async batch scrape job.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "Job ID from scrape_batch_async" },
                    "include_results": {
                        "type": "boolean",
                        "description": "Include results in response (default false)"
                    },
                    "offset": { "type": "integer", "description": "Offset for pagination" },
                    "limit": { "type": "integer", "description": "Limit for pagination (default 50)" }
                },
                "required": ["job_id"]
            }),
        },
        McpTool {
            name: "crawl_website".to_string(),
            description: "Recursively crawl a website to discover and extract content from multiple pages.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Starting URL to crawl."
                    },
                    "max_depth": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 3
                    },
                    "max_pages": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 50
                    },
                    "max_concurrent": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "default": 5
                    },
                    "same_domain_only": {
                        "type": "boolean",
                        "default": true
                    },
                    "include_patterns": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "exclude_patterns": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "max_chars_per_page": {
                        "type": "integer",
                        "minimum": 100,
                        "maximum": 50000,
                        "default": 5000
                    }
                },
                "required": ["url"]
            }),
        },
        McpTool {
            name: "extract_structured".to_string(),
            description: "Extract structured JSON data from a webpage using a BYO LLM.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to extract data from"
                    },
                    "schema": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string", "description": "Field name for the extracted data"},
                                "description": {"type": "string", "description": "What this field should contain"},
                                "field_type": {"type": "string", "enum": ["string", "number", "boolean", "array", "object"]},
                                "required": {"type": "boolean"}
                            },
                            "required": ["name", "description"]
                        },
                        "description": "Schema defining fields to extract"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Natural language description of what to extract"
                    },
                    "max_chars": {
                        "type": "integer",
                        "minimum": 100,
                        "maximum": 50000,
                        "default": 10000
                    }
                },
                "required": ["url"]
            }),
        },
        McpTool {
            name: "deep_research".to_string(),
            description: "Perform deep research on a topic by combining search, crawl, and analysis.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Research topic or question"
                    },
                    "max_search_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30,
                        "default": 10
                    },
                    "max_pages_per_site": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "default": 5
                    },
                    "max_total_pages": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 30
                    },
                    "crawl_depth": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 3,
                        "default": 2
                    },
                    "include_domains": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "exclude_domains": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "search_engines": {
                        "type": "string"
                    },
                    "time_range": {
                        "type": "string",
                        "enum": ["day", "week", "month", "year"]
                    },
                    "language": {
                        "type": "string"
                    }
                },
                "required": ["query"]
            }),
        },
        // deep_research_async
        McpTool {
            name: "deep_research_async".to_string(),
            description: "Start an async deep research job. Returns a job_id to poll status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Research query" },
                    "max_search_results": { "type": "integer" },
                    "crawl_depth": { "type": "integer" },
                    "max_pages_per_site": { "type": "integer" },
                    "language": { "type": "string" },
                    "time_range": { "type": "string" },
                    "include_domains": { "type": "array", "items": { "type": "string" } },
                    "exclude_domains": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["query"]
            }),
        },
        // check_agent_status
        McpTool {
            name: "check_agent_status".to_string(),
            description: "Check status of an async deep research job.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "Job ID from deep_research_async" },
                    "include_results": {
                        "type": "boolean",
                        "description": "Include final report in response (default false)"
                    }
                },
                "required": ["job_id"]
            }),
        },
        McpTool {
            name: "crawl_start".to_string(),
            description: "Start an async background crawl job for a website. Returns a job_id immediately; use crawl_status to poll progress and results.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Starting URL to crawl. Must be an absolute http/https URL. Crawl is website-scoped (same-domain only)."
                    },
                    "max_depth": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 3
                    },
                    "max_pages": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 50
                    },
                    "include_patterns": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "exclude_patterns": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                },
                "required": ["url"]
            }),
        },
        McpTool {
            name: "crawl_status".to_string(),
            description: "Poll the status of an async crawl job started with crawl_start. Returns current progress and optionally paginated results.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "Job ID returned by crawl_start."
                    },
                    "include_results": {
                        "type": "boolean",
                        "default": false,
                        "description": "If true, include crawled page results in the response."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "default": 0,
                        "description": "Offset for paginating results (used with include_results=true)."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "default": 50,
                        "description": "Max number of results to return per page (used with include_results=true)."
                    }
                },
                "required": ["job_id"]
            }),
        },
        McpTool {
            name: "map_website".to_string(),
            description: "Discover all URLs on a website by crawling and returning a URL list (sitemap-like). Lightweight discovery-only tool -- does not return page content.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Base URL of the website to map. Example: 'https://example.com'"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 5000,
                        "default": 100,
                        "description": "Maximum number of URLs to return."
                    },
                    "search": {
                        "type": "string",
                        "description": "Filter URLs by substring match."
                    },
                    "include_subdomains": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include URLs from subdomains."
                    },
                    "sitemap_mode": {
                        "type": "string",
                        "enum": ["crawl", "sitemap_xml"],
                        "default": "crawl",
                        "description": "Discovery method."
                    }
                },
                "required": ["url"]
            }),
        },
    ];

    Json(McpToolsResponse { tools })
}

pub async fn call_tool(
    State(state): State<Arc<AppState>>,
    Json(request): Json<McpCallRequest>,
) -> Result<Json<McpCallResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!(
        "MCP tool call: {} with args: {:?}",
        request.name, request.arguments
    );

    match request.name.as_str() {
        "search_web" => {
            // Extract query from arguments
            let query = request
                .arguments
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: query".to_string(),
                        }),
                    )
                })?;
            // Optional SearXNG overrides
            let mut overrides = search::SearchParamOverrides::default();
            if let Some(v) = request.arguments.get("engines").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    overrides.engines = Some(v.to_string());
                }
            }
            if let Some(v) = request.arguments.get("categories").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    overrides.categories = Some(v.to_string());
                }
            }
            if let Some(v) = request.arguments.get("language").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    overrides.language = Some(v.to_string());
                }
            }
            if let Some(v) = request.arguments.get("time_range").and_then(|v| v.as_str()) {
                overrides.time_range = Some(v.to_string());
            }
            if let Some(v) = request.arguments.get("safesearch").and_then(|v| v.as_u64()) {
                overrides.safesearch = Some(v as u8);
            }
            if let Some(v) = request.arguments.get("pageno").and_then(|v| v.as_u64()) {
                overrides.pageno = Some(v as u32);
            }

            let max_results = request
                .arguments
                .get("max_results")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(10);
            let snippet_chars = request
                .arguments
                .get("snippet_chars")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            // Perform search
            let ov_opt = Some(overrides);
            match search::search_web_with_params(&state, query, ov_opt).await {
                Ok((results, extras)) => {
                    let content_text = if results.is_empty() {
                        let mut text =
                            format!("No search results found for query: '{}'\n\n", query);

                        if !extras.suggestions.is_empty() {
                            text.push_str(&format!(
                                "**Suggestions:** {}\n",
                                extras.suggestions.join(", ")
                            ));
                        }
                        if !extras.corrections.is_empty() {
                            text.push_str(&format!(
                                "**Did you mean:** {}\n",
                                extras.corrections.join(", ")
                            ));
                        }
                        if !extras.unresponsive_engines.is_empty() {
                            text.push_str(&format!("\n**Note:** {} search engine(s) did not respond. Try different engines or retry.\n", extras.unresponsive_engines.len()));
                        }
                        text
                    } else {
                        let limited_results = results.iter().take(max_results);
                        let result_count = results.len();

                        let mut text =
                            format!("Found {} search results for '{}':", result_count, query);
                        if result_count > max_results {
                            text.push_str(&format!(" (showing top {})\n", max_results));
                        }
                        text.push_str("\n\n");

                        if !extras.answers.is_empty() {
                            text.push_str("**Instant Answers:**\n");
                            for answer in &extras.answers {
                                text.push_str(&format!("📌 {}\n\n", answer));
                            }
                        }

                        for (i, result) in limited_results.enumerate() {
                            let limit = snippet_chars.unwrap_or(200);
                            text.push_str(&format!(
                                "{}. **{}**\n   URL: {}\n   Snippet: {}\n\n",
                                i + 1,
                                result.title,
                                result.url,
                                result.content.chars().take(limit).collect::<String>()
                            ));
                        }

                        if !extras.suggestions.is_empty() {
                            text.push_str(&format!(
                                "\n**Related searches:** {}\n",
                                extras.suggestions.join(", ")
                            ));
                        }
                        if !extras.unresponsive_engines.is_empty() {
                            text.push_str(&format!("\n⚠️ **Note:** {} engine(s) did not respond (may affect completeness)\n", extras.unresponsive_engines.len()));
                        }

                        text
                    };

                    Ok(Json(McpCallResponse {
                        content: vec![McpContent {
                            content_type: "text".to_string(),
                            text: content_text,
                        }],
                        is_error: false,
                    }))
                }
                Err(e) => {
                    error!("Search tool error: {}", e);
                    Ok(Json(McpCallResponse {
                        content: vec![McpContent {
                            content_type: "text".to_string(),
                            text: format!("Search failed: {}", e),
                        }],
                        is_error: true,
                    }))
                }
            }
        }
        "scrape_url" => {
            // Extract URL from arguments
            let url = request
                .arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: url".to_string(),
                        }),
                    )
                })?;

            // Perform scraping - only Rust-native path
            match scrape::scrape_url(&state, url).await {
                Ok(mut content) => {
                    let max_chars = request
                        .arguments
                        .get("max_chars")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .or_else(|| {
                            std::env::var("MAX_CONTENT_CHARS")
                                .ok()
                                .and_then(|s| s.parse().ok())
                        })
                        .unwrap_or(10000);

                    // Set truncation metadata (Priority 1)
                    content.actual_chars = content.clean_content.len();
                    content.max_chars_limit = Some(max_chars);
                    content.truncated = content.clean_content.len() > max_chars;

                    if content.truncated {
                        push_warning_unique(&mut content.warnings, "content_truncated");
                    }
                    maybe_add_raw_url_warning(url, &mut content.warnings);
                    let short_content_threshold = request
                        .arguments
                        .get("short_content_threshold")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(50);
                    let extraction_score_threshold = request
                        .arguments
                        .get("extraction_score_threshold")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.4);

                    if content.word_count < short_content_threshold {
                        push_warning_unique(&mut content.warnings, "short_content");
                    }
                    if content
                        .extraction_score
                        .map(|s| s < extraction_score_threshold)
                        .unwrap_or(false)
                    {
                        push_warning_unique(&mut content.warnings, "low_extraction_score");
                    }

                    // Check for output_format parameter (Priority 1)
                    let output_format = request
                        .arguments
                        .get("output_format")
                        .and_then(|v| v.as_str())
                        .unwrap_or("text");

                    if output_format == "json" {
                        let max_images = request
                            .arguments
                            .get("max_images")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as usize)
                            .unwrap_or(content.images.len());
                        if content.images.len() > max_images {
                            content.images.truncate(max_images);
                        }

                        // Return JSON format directly as text (capped total payload size)
                        let json_str = serde_json::to_string_pretty(&content).unwrap_or_else(|e| {
                            format!(r#"{{"error": "Failed to serialize: {}"}}"#, e)
                        });
                        let capped = cap_json_payload(json_str, max_chars);
                        return Ok(Json(McpCallResponse {
                            content: vec![McpContent {
                                content_type: "text".to_string(),
                                text: capped,
                            }],
                            is_error: false,
                        }));
                    }

                    // Otherwise return formatted text (backward compatible)
                    let content_text = {
                        let content_preview = if content.clean_content.is_empty() {
                            "[No content extracted]\n\n**Possible reasons:**\n\
                            • Page is JavaScript-heavy (requires browser execution)\n\
                            • Content is behind authentication/paywall\n\
                            • Site blocks automated access\n\n\
                            **Suggestion:** For JS-heavy sites, try using the Playwright MCP tool instead.".to_string()
                        } else if content.word_count < 10 {
                            format!("{}\n\n⚠️ **Very short content** ({} words). Page may be mostly dynamic/JS-based.", 
                                content.clean_content.chars().take(max_chars).collect::<String>(),
                                content.word_count)
                        } else {
                            let preview = content
                                .clean_content
                                .chars()
                                .take(max_chars)
                                .collect::<String>();
                            if content.clean_content.len() > max_chars {
                                format!("{}\n\n[Content truncated: {}/{} chars shown. Increase max_chars parameter to see more]",
                                    preview, max_chars, content.clean_content.len())
                            } else {
                                preview
                            }
                        };

                        let max_headings = request
                            .arguments
                            .get("max_headings")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as usize)
                            .unwrap_or(10);
                        let max_images = request
                            .arguments
                            .get("max_images")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as usize)
                            .unwrap_or(content.images.len());

                        let headings = content
                            .headings
                            .iter()
                            .take(max_headings)
                            .map(|h| format!("- {} {}", h.level.to_uppercase(), h.text))
                            .collect::<Vec<_>>()
                            .join("\n");

                        // Build Sources section from links
                        let sources_section = if content.links.is_empty() {
                            String::new()
                        } else {
                            let mut sources = String::from("\n\nSources:\n");
                            // Get max_links from args or env var or default
                            let max_sources = request
                                .arguments
                                .get("max_links")
                                .and_then(|v| v.as_u64())
                                .map(|n| n as usize)
                                .or_else(|| {
                                    std::env::var("MAX_LINKS").ok().and_then(|s| s.parse().ok())
                                })
                                .unwrap_or(100);
                            let link_count = content.links.len();
                            for (i, link) in content.links.iter().take(max_sources).enumerate() {
                                if !link.text.is_empty() {
                                    sources.push_str(&format!(
                                        "[{}]: {} ({})",
                                        i + 1,
                                        link.url,
                                        link.text
                                    ));
                                } else {
                                    sources.push_str(&format!("[{}]: {}", i + 1, link.url));
                                }
                                sources.push('\n');
                            }
                            if link_count > max_sources {
                                sources.push_str(&format!(
                                    "\n(Showing {} of {} total links)\n",
                                    max_sources, link_count
                                ));
                            }
                            sources
                        };

                        format!(
                            "{}\nURL: {}\nCanonical: {}\nWord Count: {} ({}m)\nLanguage: {}\nSite: {}\nAuthor: {}\nPublished: {}\n\nDescription: {}\nOG Image: {}\n\nHeadings:\n{}\n\nLinks: {}  Images: {}\n\nPreview:\n{}{}",
                            content.title,
                            content.url,
                            content.canonical_url.as_deref().unwrap_or("-"),
                            content.word_count,
                            content.reading_time_minutes.unwrap_or(((content.word_count as f64 / 200.0).ceil() as u32).max(1)),
                            content.language,
                            content.site_name.as_deref().unwrap_or("-"),
                            content.author.as_deref().unwrap_or("-"),
                            content.published_at.as_deref().unwrap_or("-"),
                            content.meta_description,
                            content.og_image.as_deref().unwrap_or("-"),
                            headings,
                            content.links.len(),
                            content.images.len().min(max_images),
                            content_preview,
                            sources_section
                        )
                    };

                    Ok(Json(McpCallResponse {
                        content: vec![McpContent {
                            content_type: "text".to_string(),
                            text: content_text,
                        }],
                        is_error: false,
                    }))
                }
                Err(e) => {
                    error!("Scrape tool error: {}", e);
                    Ok(Json(McpCallResponse {
                        content: vec![McpContent {
                            content_type: "text".to_string(),
                            text: format!("Scraping failed: {}", e),
                        }],
                        is_error: true,
                    }))
                }
            }
        }
        "scrape_batch_async" => {
            let urls = request
                .arguments
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: urls (array of strings)"
                                .to_string(),
                        }),
                    )
                })?;

            if urls.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "urls array cannot be empty".to_string(),
                    }),
                ));
            }

            let max_concurrent = request
                .arguments
                .get("max_concurrent")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let max_chars = request
                .arguments
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            match scrape::scrape_batch_async(&state, urls, max_concurrent, max_chars).await {
                Ok(response) => Ok(Json(McpCallResponse {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string(&response)
                            .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                    }],
                    is_error: false,
                })),
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Async batch scrape error: {}", e),
                    }),
                )),
            }
        }
        "check_batch_status" => {
            let job_id = request
                .arguments
                .get("job_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: job_id".to_string(),
                        }),
                    )
                })?;

            let include_results = request
                .arguments
                .get("include_results")
                .and_then(|v| v.as_bool());
            let offset = request
                .arguments
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let limit = request
                .arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            match scrape::check_batch_status(&state, job_id, include_results, offset, limit).await {
                Ok(response) => Ok(Json(McpCallResponse {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string(&response)
                            .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                    }],
                    is_error: false,
                })),
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Batch status error: {}", e),
                    }),
                )),
            }
        }
        "deep_research_async" => {
            let query = request
                .arguments
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: query".to_string(),
                        }),
                    )
                })?;

            let config = crate::types::ResearchJobRequest {
                query: query.to_string(),
                max_search_results: request
                    .arguments
                    .get("max_search_results")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize),
                crawl_depth: request
                    .arguments
                    .get("crawl_depth")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize),
                max_pages_per_site: request
                    .arguments
                    .get("max_pages_per_site")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize),
                language: request
                    .arguments
                    .get("language")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                time_range: request
                    .arguments
                    .get("time_range")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                include_domains: request
                    .arguments
                    .get("include_domains")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    }),
                exclude_domains: request
                    .arguments
                    .get("exclude_domains")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    }),
            };

            match research::deep_research_async(&state, query.to_string(), config).await {
                Ok(response) => Ok(Json(McpCallResponse {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string(&response)
                            .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                    }],
                    is_error: false,
                })),
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Async deep research error: {}", e),
                    }),
                )),
            }
        }
        "check_agent_status" => {
            let job_id = request
                .arguments
                .get("job_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: job_id".to_string(),
                        }),
                    )
                })?;

            let include_results = request
                .arguments
                .get("include_results")
                .and_then(|v| v.as_bool());

            match research::check_agent_status(&state, job_id, include_results).await {
                Ok(response) => Ok(mcp_text_response(
                    serde_json::to_string(&response)
                        .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                    false,
                )),
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Agent status error: {}", e),
                    }),
                )),
            }
        }
        "research_history" => {
            let query = request
                .arguments
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: query".to_string(),
                        }),
                    )
                })?;

            let memory = match &state.memory {
                Some(m) => m,
                None => {
                    return Ok(mcp_text_response(
                        "Research history feature is not available. Set QDRANT_URL environment variable to enable.\n\nExample: QDRANT_URL=http://localhost:6333".to_string(),
                        false,
                    ));
                }
            };

            let limit = request
                .arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(10);
            let threshold = request
                .arguments
                .get("threshold")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.7) as f32;

            let entry_type_filter = request
                .arguments
                .get("entry_type")
                .and_then(|v| v.as_str())
                .and_then(|s| match s.to_lowercase().as_str() {
                    "search" => Some(history::EntryType::Search),
                    "scrape" => Some(history::EntryType::Scrape),
                    _ => None,
                });

            match memory
                .search_history(query, limit, threshold, entry_type_filter)
                .await
            {
                Ok(results) => {
                    if results.is_empty() {
                        Ok(mcp_text_response(
                            format!(
                                "No relevant history found for: '{}'\n\nTry:\n- Lower threshold (currently {:.2})\n- Broader search terms\n- Check if you have any saved history",
                                query, threshold
                            ),
                            false,
                        ))
                    } else {
                        let mut text = format!(
                            "Found {} relevant entries for '{}':\n\n",
                            results.len(),
                            query
                        );

                        for (i, (entry, score)) in results.iter().enumerate() {
                            text.push_str(&format!(
                                "{}. [Similarity: {:.3}] **{}** ({})\n   Type: {:?}\n   When: {}\n   Summary: {}\n",
                                i + 1,
                                score,
                                entry.topic,
                                entry.domain.as_deref().unwrap_or("N/A"),
                                entry.entry_type,
                                entry.timestamp.format("%Y-%m-%d %H:%M UTC"),
                                entry.summary.chars().take(150).collect::<String>()
                            ));
                            text.push_str(&format!("   Query: {}\n\n", entry.query));
                        }

                        text.push_str(&format!(
                            "\n💡 Tip: Use threshold={:.2} for similar results, or higher (0.8-0.9) for more specific matches",
                            threshold
                        ));

                        Ok(mcp_text_response(text, false))
                    }
                }
                Err(e) => Ok(mcp_text_response(
                    format!("History search failed: {}", e),
                    true,
                )),
            }
        }
        "scrape_batch" => {
            let urls = request
                .arguments
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: urls (array of strings)"
                                .to_string(),
                        }),
                    )
                })?;

            if urls.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "urls array cannot be empty".to_string(),
                    }),
                ));
            }

            if urls.len() > 100 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Maximum 100 URLs per request, got {}", urls.len()),
                    }),
                ));
            }

            let max_concurrent = request
                .arguments
                .get("max_concurrent")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let max_chars = request
                .arguments
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let output_format = request
                .arguments
                .get("output_format")
                .and_then(|v| v.as_str())
                .unwrap_or("json");

            match scrape::scrape_batch(&state, urls, max_concurrent, max_chars).await {
                Ok(response) => {
                    if output_format == "json" {
                        Ok(mcp_text_response(
                            serde_json::to_string_pretty(&response)
                                .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                            false,
                        ))
                    } else {
                        let mut text = format!(
                            "**Batch Scrape Results**\n\nTotal: {} | Successful: {} | Failed: {} | Duration: {}ms\n\n",
                            response.total,
                            response.successful,
                            response.failed,
                            response.total_duration_ms
                        );

                        for (i, result) in response.results.iter().enumerate() {
                            if result.success {
                                if let Some(data) = &result.data {
                                    text.push_str(&format!(
                                        "{}. ✅ **{}**\n   URL: {}\n   Words: {} | Score: {:.2} | {}ms\n\n",
                                        i + 1,
                                        data.title.chars().take(60).collect::<String>(),
                                        result.url,
                                        data.word_count,
                                        data.extraction_score.unwrap_or(0.0),
                                        result.duration_ms
                                    ));
                                }
                            } else {
                                text.push_str(&format!(
                                    "{}. ❌ {}\n   Error: {}\n\n",
                                    i + 1,
                                    result.url,
                                    result.error.as_deref().unwrap_or("Unknown error")
                                ));
                            }
                        }

                        Ok(mcp_text_response(text, false))
                    }
                }
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Batch scrape error: {}", e),
                    }),
                )),
            }
        }
        "crawl_website" => {
            let url = request
                .arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: url".to_string(),
                        }),
                    )
                })?;

            let config = map_crawl_config(&request.arguments, true);

            match crawl::crawl_website(&state, url, config).await {
                Ok(response) => {
                    let mut text = format!(
                        "**Crawl Results for {}**\n\n📊 **Summary:**\n• Pages crawled: {}\n• Pages failed: {}\n• Max depth reached: {}\n• Unique domains: {}\n• Total duration: {}ms\n\n",
                        response.start_url,
                        response.pages_crawled,
                        response.pages_failed,
                        response.max_depth_reached,
                        response.unique_domains.len(),
                        response.total_duration_ms
                    );

                    text.push_str("**📄 Pages Crawled:**\n\n");
                    for (i, result) in response.results.iter().enumerate() {
                        if result.success {
                            text.push_str(&format!(
                                "{}. ✅ **{}**\n   URL: {}\n   Depth: {} | Words: {} | Links: {} | {}ms\n",
                                i + 1,
                                result
                                    .title
                                    .as_deref()
                                    .unwrap_or("Untitled")
                                    .chars()
                                    .take(60)
                                    .collect::<String>(),
                                result.url,
                                result.depth,
                                result.word_count.unwrap_or(0),
                                result.links_found.unwrap_or(0),
                                result.duration_ms
                            ));
                            if let Some(preview) = &result.content_preview {
                                text.push_str(&format!(
                                    "   Preview: {}...\n",
                                    preview
                                        .chars()
                                        .take(200)
                                        .collect::<String>()
                                        .replace('\n', " ")
                                ));
                            }
                            text.push('\n');
                        } else {
                            text.push_str(&format!(
                                "{}. ❌ {}\n   Depth: {} | Error: {}\n\n",
                                i + 1,
                                result.url,
                                result.depth,
                                result.error.as_deref().unwrap_or("Unknown error")
                            ));
                        }
                    }

                    if let Some(sitemap) = &response.sitemap {
                        if !sitemap.is_empty() {
                            text.push_str(&format!("\n**🗺️ Sitemap ({} URLs):**\n", sitemap.len()));
                            for sitemap_url in sitemap.iter().take(50) {
                                text.push_str(&format!("• {}\n", sitemap_url));
                            }
                            if sitemap.len() > 50 {
                                text.push_str(&format!(
                                    "... and {} more URLs\n",
                                    sitemap.len() - 50
                                ));
                            }
                        }
                    }

                    Ok(mcp_text_response(text, false))
                }
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Crawl failed: {}", e),
                    }),
                )),
            }
        }
        "extract_structured" => {
            let url = request
                .arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: url".to_string(),
                        }),
                    )
                })?;

            let schema: Option<Vec<crate::types::ExtractField>> = request
                .arguments
                .get("schema")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let name = item.get("name")?.as_str()?.to_string();
                            let description = item.get("description")?.as_str()?.to_string();
                            let field_type = item
                                .get("field_type")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let required = item.get("required").and_then(|v| v.as_bool());
                            Some(crate::types::ExtractField {
                                name,
                                description,
                                field_type,
                                required,
                            })
                        })
                        .collect()
                });

            let prompt = request
                .arguments
                .get("prompt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let max_chars = request
                .arguments
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            match extract::extract_structured(&state, url, schema, prompt, max_chars).await {
                Ok(response) => {
                    let mut text = format!(
                        "**Structured Extraction Results**\n\n📊 **Extraction Info:**\n• URL: {}\n• Title: {}\n• Method: {}\n• Fields Extracted: {}\n• Confidence: {:.0}%\n• Duration: {}ms\n\n",
                        response.url,
                        response.title,
                        response.extraction_method,
                        response.field_count,
                        response.confidence * 100.0,
                        response.duration_ms
                    );

                    if !response.warnings.is_empty() {
                        text.push_str(&format!(
                            "⚠️ **Warnings:** {}\n\n",
                            response.warnings.join(", ")
                        ));
                    }

                    text.push_str("**📋 Extracted Data:**\n```json\n");
                    let json_str = serde_json::to_string_pretty(&response.extracted_data)
                        .unwrap_or_else(|_| "{}".to_string());
                    text.push_str(&json_str);
                    text.push_str("\n```\n\n");

                    text.push_str("**📄 Raw Content Preview:**\n");
                    let preview: String = response.raw_content_preview.chars().take(1000).collect();
                    text.push_str(&preview);
                    if response.raw_content_preview.len() > 1000 {
                        text.push_str("...\n[truncated]");
                    }

                    Ok(mcp_text_response(text, false))
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let message = if err_str.contains(crate::llm_client::LLM_NOT_CONFIGURED) {
                        crate::llm_client::LLM_NOT_CONFIGURED.to_string()
                    } else if err_str.contains(crate::llm_client::LLM_AUTH_FAILED) {
                        crate::llm_client::LLM_AUTH_FAILED.to_string()
                    } else if err_str.contains(crate::llm_client::LLM_RATE_LIMITED) {
                        crate::llm_client::LLM_RATE_LIMITED.to_string()
                    } else if err_str.contains(crate::llm_client::LLM_TIMEOUT) {
                        crate::llm_client::LLM_TIMEOUT.to_string()
                    } else if err_str.contains(crate::llm_client::LLM_INVALID_JSON) {
                        crate::llm_client::LLM_INVALID_JSON.to_string()
                    } else {
                        format!("EXTRACT_FAILED: {}", err_str)
                    };

                    Ok(mcp_text_response(message, true))
                }
            }
        }
        "deep_research" => {
            let query = request
                .arguments
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: query".to_string(),
                        }),
                    )
                })?;

            let max_search_results = request
                .arguments
                .get("max_search_results")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(10);
            let max_pages_per_site = request
                .arguments
                .get("max_pages_per_site")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(5);
            let max_total_pages = request
                .arguments
                .get("max_total_pages")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(30);
            let crawl_depth = request
                .arguments
                .get("crawl_depth")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(2);
            let search_engines = request
                .arguments
                .get("search_engines")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let time_range = request
                .arguments
                .get("time_range")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let language = request
                .arguments
                .get("language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let include_domains = parse_string_list_arg(&request.arguments, "include_domains");
            let exclude_domains = parse_string_list_arg(&request.arguments, "exclude_domains");

            let config = research::DeepResearchConfig {
                max_search_results: max_search_results.min(30),
                max_pages_per_site: max_pages_per_site.min(20),
                max_total_pages: max_total_pages.min(100),
                crawl_depth: crawl_depth.min(3),
                max_concurrent: 5,
                include_domains,
                exclude_domains,
                search_engines,
                time_range,
                language,
                max_chars_per_page: 5000,
            };

            match research::deep_research(&state, query, config).await {
                Ok(response) => {
                    let mut text = format!(
                        "# 🔬 Deep Research Results\n\n**Query:** {}\n\n## 📊 Statistics\n• Search results: {}\n• Pages scraped: {}\n• Pages crawled: {}\n• Total words: {}\n• Unique domains: {}\n• Code blocks found: {}\n• Duration: {}ms\n\n",
                        response.query,
                        response.statistics.search_results_found,
                        response.statistics.pages_scraped,
                        response.statistics.pages_crawled,
                        response.statistics.total_words,
                        response.statistics.unique_domains,
                        response.statistics.code_blocks_found,
                        response.statistics.duration_ms
                    );

                    if !response.warnings.is_empty() {
                        text.push_str(&format!(
                            "⚠️ **Warnings:** {}\n\n",
                            response.warnings.join(", ")
                        ));
                    }

                    text.push_str("## 📝 Summary\n\n");
                    text.push_str(&response.summary.overview);
                    text.push_str("\n\n");

                    if !response.summary.key_points.is_empty() {
                        text.push_str("**Key Points:**\n");
                        for point in &response.summary.key_points {
                            text.push_str(&format!("• {}\n", point));
                        }
                        text.push('\n');
                    }

                    if !response.key_findings.is_empty() {
                        text.push_str("## 💡 Key Findings\n\n");
                        for finding in &response.key_findings {
                            text.push_str(&format!("• {}\n", finding));
                        }
                        text.push('\n');
                    }

                    if !response.topics.is_empty() {
                        text.push_str("## 🏷️ Topics Discovered\n\n");
                        for topic in response.topics.iter().take(8) {
                            text.push_str(&format!(
                                "• **{}** (mentioned {} times across {} sources)\n",
                                topic.topic,
                                topic.mentions,
                                topic.sources.len()
                            ));
                        }
                        text.push('\n');
                    }

                    text.push_str("## 📚 Sources\n\n");
                    for (i, source) in response.sources.iter().enumerate().take(15) {
                        let crawl_indicator = if source.from_crawl { " 🔗" } else { "" };
                        text.push_str(&format!(
                            "{}. **{}**{}\n   {} | {} | {} words | {:.0}% relevance\n",
                            i + 1,
                            source.title.chars().take(60).collect::<String>(),
                            crawl_indicator,
                            source.url,
                            source.source_type,
                            source.word_count,
                            source.relevance_score * 100.0
                        ));

                        if !source.headings.is_empty() {
                            let headings_preview = source
                                .headings
                                .iter()
                                .take(3)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" | ");
                            text.push_str(&format!("   📑 {}\n", headings_preview));
                        }

                        let preview: String = source.content_preview.chars().take(150).collect();
                        text.push_str(&format!("   {}\n\n", preview.replace('\n', " ")));
                    }

                    if response.sources.len() > 15 {
                        text.push_str(&format!(
                            "... and {} more sources\n\n",
                            response.sources.len() - 15
                        ));
                    }

                    if !response.related_queries.is_empty() {
                        text.push_str("## 🔍 Related Queries for Further Research\n\n");
                        for related_query in response.related_queries.iter().take(5) {
                            text.push_str(&format!("• {}\n", related_query));
                        }
                    }

                    text.push_str("\n## 📊 Content Types\n\n");
                    for (content_type, count) in &response.summary.content_types {
                        text.push_str(&format!("• {}: {}\n", content_type, count));
                    }

                    Ok(mcp_text_response(text, false))
                }
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Deep research failed: {}", e),
                    }),
                )),
            }
        }
        "crawl_start" => {
            let url = request
                .arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: url".to_string(),
                        }),
                    )
                })?
                .to_string();

            let parsed_url = url::Url::parse(&url).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!(
                            "Invalid URL: '{}'. Must be a valid absolute URL with http or https scheme (e.g., https://example.com)",
                            url
                        ),
                    }),
                )
            })?;

            if parsed_url.scheme() != "http" && parsed_url.scheme() != "https" {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!(
                            "Unsupported URL scheme '{}': only http and https are supported for crawling",
                            parsed_url.scheme()
                        ),
                    }),
                ));
            }

            let config = map_crawl_config(&request.arguments, true);
            let job_id = state.crawl_jobs.create_job(url.clone()).await;

            state.crawl_jobs.mark_running(&job_id).await;
            let store = Arc::clone(&state.crawl_jobs);
            let state_bg = Arc::clone(&state);
            let job_id_bg = job_id.clone();
            let url_bg = url.clone();

            tokio::spawn(async move {
                match crawl::crawl_website(&state_bg, &url_bg, config).await {
                    Ok(response) => {
                        store.mark_completed(&job_id_bg, response.results).await;
                    }
                    Err(e) => {
                        store.mark_failed(&job_id_bg, e.to_string()).await;
                    }
                }
            });

            Ok(mcp_text_response(
                serde_json::to_string(&crate::types::CrawlStartResponse {
                    job_id,
                    status: crate::types::CrawlJobStatus::Running,
                })
                .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                false,
            ))
        }
        "crawl_status" => {
            let job_id = request
                .arguments
                .get("job_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: job_id".to_string(),
                        }),
                    )
                })?
                .to_string();

            let include_results = request
                .arguments
                .get("include_results")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let offset = request
                .arguments
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            let limit = request
                .arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(50);

            match state.crawl_jobs.get_job(&job_id).await {
                None => Ok(mcp_text_response(
                    format!("{{\"code\":\"JOB_NOT_FOUND\",\"message\":\"No crawl job found with id: {}\",\"retryable\":false,\"request_id_or_job_id\":\"{}\"}}", job_id, job_id),
                    true,
                )),
                Some(job) => {
                    let results_page = if include_results {
                        job.results.as_ref().map(|all| {
                            all.iter()
                                .skip(offset)
                                .take(limit)
                                .cloned()
                                .collect::<Vec<_>>()
                        })
                    } else {
                        None
                    };

                    let response = crate::types::CrawlStatusResponse {
                        job_id: job.job_id.clone(),
                        status: job.status.clone(),
                        pages_crawled: Some(job.pages_crawled),
                        pages_total: job.pages_total,
                        results: results_page,
                        error: job.error.clone(),
                    };

                    Ok(mcp_text_response(
                        serde_json::to_string(&response)
                            .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                        false,
                    ))
                }
            }
        }
        "map_website" => {
            let url = request
                .arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Missing required parameter: url".to_string(),
                        }),
                    )
                })?;

            let limit = request
                .arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(100);
            let search_filter = request
                .arguments
                .get("search")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let include_subdomains = request
                .arguments
                .get("include_subdomains")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let start_time = std::time::Instant::now();
            let config = map_website_crawl_config(limit, include_subdomains);

            let base_host = url::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_lowercase()));

            match crawl::crawl_website(&state, url, config).await {
                Ok(response) => {
                    let mut discovered_urls: Vec<String> = response
                        .results
                        .iter()
                        .filter(|r| r.success)
                        .map(|r| r.url.clone())
                        .collect();

                    if !include_subdomains {
                        if let Some(ref base) = base_host {
                            discovered_urls.retain(|u| {
                                url::Url::parse(u)
                                    .ok()
                                    .and_then(|parsed| {
                                        parsed.host_str().map(|h| h.to_lowercase() == *base)
                                    })
                                    .unwrap_or(false)
                            });
                        }
                    }

                    if let Some(ref filter) = search_filter {
                        let filter_lower = filter.to_lowercase();
                        discovered_urls.retain(|u| u.to_lowercase().contains(&filter_lower));
                    }

                    discovered_urls.sort();
                    discovered_urls.dedup();
                    discovered_urls.truncate(limit);

                    let map_response = crate::types::MapWebsiteResponse {
                        url: url.to_string(),
                        total_urls: discovered_urls.len(),
                        urls: discovered_urls,
                        search_filter,
                        include_subdomains,
                        duration_ms: start_time.elapsed().as_millis() as u64,
                    };

                    Ok(mcp_text_response(
                        serde_json::to_string_pretty(&map_response)
                            .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e)),
                        false,
                    ))
                }
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Map website failed: {}", e),
                    }),
                )),
            }
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Unknown tool: {}", request.name),
            }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState::new(
            "http://localhost:8888".to_string(),
            reqwest::Client::new(),
        ))
    }

    #[tokio::test]
    async fn test_list_tools_contains_expected_14_names_and_no_duplicates() {
        let Json(response) = list_tools().await;
        let names: Vec<String> = response.tools.into_iter().map(|tool| tool.name).collect();

        let expected = vec![
            "search_web",
            "scrape_url",
            "research_history",
            "scrape_batch",
            "scrape_batch_async",
            "check_batch_status",
            "crawl_website",
            "extract_structured",
            "deep_research",
            "deep_research_async",
            "check_agent_status",
            "crawl_start",
            "crawl_status",
            "map_website",
        ];

        assert_eq!(expected.len(), 14);
        assert_eq!(names.len(), expected.len());

        let unique: HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "tool names should be unique");

        for tool_name in expected {
            assert!(
                names.iter().any(|name| name == tool_name),
                "missing tool: {tool_name}"
            );
        }
    }

    #[tokio::test]
    async fn test_new_tools_missing_required_params_return_bad_request() {
        let cases = vec![
            ("research_history", json!({}), "query"),
            ("scrape_batch", json!({}), "urls"),
            ("crawl_website", json!({}), "url"),
            ("extract_structured", json!({}), "url"),
            ("deep_research", json!({}), "query"),
            ("crawl_start", json!({}), "url"),
            ("crawl_status", json!({}), "job_id"),
            ("map_website", json!({}), "url"),
        ];

        for (name, arguments, required_param) in cases {
            let result = call_tool(
                State(test_state()),
                Json(McpCallRequest {
                    name: name.to_string(),
                    arguments,
                }),
            )
            .await;

            match result {
                Err((status, Json(error))) => {
                    assert_eq!(status, StatusCode::BAD_REQUEST, "tool: {name}");
                    assert!(
                        error.error.contains("Missing required parameter")
                            && error.error.contains(required_param),
                        "tool {name} should mention missing required parameter {required_param}, got: {}",
                        error.error
                    );
                }
                Ok(_) => panic!("tool {name} should return BAD_REQUEST for missing params"),
            }
        }
    }

    #[tokio::test]
    async fn test_crawl_start_rejects_non_http_https_url() {
        let result = call_tool(
            State(test_state()),
            Json(McpCallRequest {
                name: "crawl_start".to_string(),
                arguments: json!({ "url": "ftp://example.com" }),
            }),
        )
        .await;

        match result {
            Err((status, Json(error))) => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert!(
                    error.error.contains("http") && error.error.contains("https"),
                    "unexpected error message: {}",
                    error.error
                );
            }
            Ok(_) => panic!("crawl_start should reject non-http/https URL"),
        }
    }
}
