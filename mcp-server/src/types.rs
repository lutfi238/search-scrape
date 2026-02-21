use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub content: String,
    pub engine: Option<String>,
    pub score: Option<f64>,
    // New Priority 2 fields for better filtering
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>, // docs, repo, blog, news, other
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScrapeRequest {
    pub url: String,
    #[serde(default)]
    pub content_links_only: Option<bool>,
    #[serde(default)]
    pub max_links: Option<usize>,
    #[serde(default)]
    pub max_images: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScrapeResponse {
    pub url: String,
    pub title: String,
    pub content: String,
    pub clean_content: String,
    pub meta_description: String,
    pub meta_keywords: String,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    pub images: Vec<Image>,
    pub timestamp: String,
    pub status_code: u16,
    pub content_type: String,
    pub word_count: usize,
    pub language: String,
    #[serde(default)]
    pub canonical_url: Option<String>,
    #[serde(default)]
    pub site_name: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub og_title: Option<String>,
    #[serde(default)]
    pub og_description: Option<String>,
    #[serde(default)]
    pub og_image: Option<String>,
    #[serde(default)]
    pub reading_time_minutes: Option<u32>,
    // New Priority 1 fields
    #[serde(default)]
    pub code_blocks: Vec<CodeBlock>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub actual_chars: usize,
    #[serde(default)]
    pub max_chars_limit: Option<usize>,
    #[serde(default)]
    pub extraction_score: Option<f64>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub domain: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub code: String,
    #[serde(default)]
    pub start_char: Option<usize>,
    #[serde(default)]
    pub end_char: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Heading {
    pub level: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Link {
    pub url: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Image {
    pub src: String,
    pub alt: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatResponse {
    pub response: String,
    pub search_results: Vec<SearchResult>,
    pub scraped_content: Vec<ScrapeResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// Batch scraping types
#[derive(Debug, Serialize, Deserialize)]
pub struct ScrapeBatchRequest {
    pub urls: Vec<String>,
    #[serde(default)]
    pub max_concurrent: Option<usize>,
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub output_format: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScrapeBatchResult {
    pub url: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ScrapeResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScrapeBatchResponse {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub total_duration_ms: u64,
    pub results: Vec<ScrapeBatchResult>,
}

// Website crawling types
#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlRequest {
    pub url: String,
    #[serde(default)]
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub max_pages: Option<usize>,
    #[serde(default)]
    pub max_concurrent: Option<usize>,
    #[serde(default)]
    pub include_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub same_domain_only: Option<bool>,
    #[serde(default)]
    pub max_chars_per_page: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CrawlPageResult {
    pub url: String,
    pub depth: usize,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links_found: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlResponse {
    pub start_url: String,
    pub pages_crawled: usize,
    pub pages_failed: usize,
    pub max_depth_reached: usize,
    pub total_duration_ms: u64,
    pub unique_domains: Vec<String>,
    pub results: Vec<CrawlPageResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sitemap: Option<Vec<String>>,
}

// Structured extraction types
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExtractField {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub field_type: Option<String>,  // string, number, boolean, array, object
    #[serde(default)]
    pub required: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractRequest {
    pub url: String,
    #[serde(default)]
    pub schema: Option<Vec<ExtractField>>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractResponse {
    pub url: String,
    pub title: String,
    pub extracted_data: serde_json::Value,
    pub raw_content_preview: String,
    pub extraction_method: String,
    pub field_count: usize,
    pub confidence: f64,
    pub duration_ms: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

// SearXNG API types
#[derive(Debug, Deserialize)]
pub struct SearxngResponse {
    pub query: String,
    pub number_of_results: u32,
    pub results: Vec<SearxngResult>,
    #[serde(default)]
    pub infoboxes: Option<serde_json::Value>,
    #[serde(default)]
    pub suggestions: Option<serde_json::Value>,
    #[serde(default)]
    pub answers: Option<serde_json::Value>,
    #[serde(default)]
    pub corrections: Option<serde_json::Value>,
    #[serde(default)]
    pub unresponsive_engines: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SearxngResult {
    pub url: String,
    pub title: String,
    pub content: String,
    pub engine: String,
    #[serde(default)]
    pub parsed_url: Option<Vec<String>>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub engines: Option<Vec<String>>,
    #[serde(default)]
    pub positions: Option<serde_json::Value>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub img_src: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(rename = "publishedDate", default)]
    pub published_date: Option<serde_json::Value>,
}

// Async crawl job types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrawlJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Expired,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlStartRequest {
    pub url: String,
    #[serde(default)]
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub max_pages: Option<usize>,
    #[serde(default)]
    pub include_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_patterns: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlStartResponse {
    pub job_id: String,
    pub status: CrawlJobStatus,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlStatusRequest {
    pub job_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlStatusResponse {
    pub job_id: String,
    pub status: CrawlJobStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages_crawled: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages_total: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<CrawlPageResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// BYO LLM configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
}

// Standardized tool error envelope
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolErrorEnvelope {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id_or_job_id: Option<String>,
}

// Website mapping types
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MapWebsiteResponse {
    pub url: String,
    pub total_urls: usize,
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_filter: Option<String>,
    pub include_subdomains: bool,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- CrawlJobStatus serialization tests ---

    #[test]
    fn test_crawl_job_status_serialization() {
        let s = serde_json::to_string(&CrawlJobStatus::Queued).unwrap();
        assert_eq!(s, "\"queued\"");
    }

    #[test]
    fn test_crawl_job_status_all_variants() {
        assert_eq!(serde_json::to_string(&CrawlJobStatus::Queued).unwrap(), "\"queued\"");
        assert_eq!(serde_json::to_string(&CrawlJobStatus::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&CrawlJobStatus::Completed).unwrap(), "\"completed\"");
        assert_eq!(serde_json::to_string(&CrawlJobStatus::Failed).unwrap(), "\"failed\"");
        assert_eq!(serde_json::to_string(&CrawlJobStatus::Expired).unwrap(), "\"expired\"");
    }

    #[test]
    fn test_crawl_job_status_deserialization() {
        let status: CrawlJobStatus = serde_json::from_str("\"running\"").unwrap();
        assert_eq!(status, CrawlJobStatus::Running);
    }

    // --- CrawlStartRequest / CrawlStartResponse roundtrip tests ---

    #[test]
    fn test_crawl_start_request_roundtrip() {
        let req = CrawlStartRequest {
            url: "https://example.com".to_string(),
            max_depth: Some(3),
            max_pages: Some(100),
            include_patterns: None,
            exclude_patterns: Some(vec!["*.pdf".to_string()]),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: CrawlStartRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.url, "https://example.com");
        assert_eq!(deserialized.max_depth, Some(3));
        assert_eq!(deserialized.max_pages, Some(100));
        assert!(deserialized.include_patterns.is_none());
        assert_eq!(deserialized.exclude_patterns.as_ref().unwrap()[0], "*.pdf");
    }

    #[test]
    fn test_crawl_start_response_roundtrip() {
        let resp = CrawlStartResponse {
            job_id: "abc-123".to_string(),
            status: CrawlJobStatus::Queued,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: CrawlStartResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.job_id, "abc-123");
        assert_eq!(deserialized.status, CrawlJobStatus::Queued);
    }

    // --- CrawlStatusRequest / CrawlStatusResponse roundtrip tests ---

    #[test]
    fn test_crawl_status_request_roundtrip() {
        let req = CrawlStatusRequest {
            job_id: "abc-123".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: CrawlStatusRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.job_id, "abc-123");
    }

    #[test]
    fn test_crawl_status_response_roundtrip() {
        let resp = CrawlStatusResponse {
            job_id: "abc-123".to_string(),
            status: CrawlJobStatus::Completed,
            pages_crawled: Some(42),
            pages_total: Some(100),
            results: Some(vec![]),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: CrawlStatusResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.job_id, "abc-123");
        assert_eq!(deserialized.status, CrawlJobStatus::Completed);
        assert_eq!(deserialized.pages_crawled, Some(42));
        assert_eq!(deserialized.pages_total, Some(100));
        assert!(deserialized.results.unwrap().is_empty());
        assert!(deserialized.error.is_none());
    }

    // --- LlmConfig roundtrip tests ---

    #[test]
    fn test_llm_config_roundtrip() {
        let config = LlmConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "sk-test-key".to_string(),
            model: "gpt-4".to_string(),
            timeout_ms: Some(30000),
            max_tokens: Some(4096),
            temperature: Some(0.7),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: LlmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.base_url, "https://api.openai.com/v1");
        assert_eq!(deserialized.api_key, "sk-test-key");
        assert_eq!(deserialized.model, "gpt-4");
        assert_eq!(deserialized.timeout_ms, Some(30000));
        assert_eq!(deserialized.max_tokens, Some(4096));
        assert!((deserialized.temperature.unwrap() - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_llm_config_minimal() {
        let json = r#"{"base_url":"http://localhost:11434/v1","api_key":"ollama","model":"llama3"}"#;
        let config: LlmConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "http://localhost:11434/v1");
        assert_eq!(config.model, "llama3");
        assert!(config.timeout_ms.is_none());
        assert!(config.max_tokens.is_none());
        assert!(config.temperature.is_none());
    }

    // --- ToolErrorEnvelope roundtrip tests ---

    #[test]
    fn test_tool_error_envelope_roundtrip() {
        let envelope = ToolErrorEnvelope {
            code: "CRAWL_FAILED".to_string(),
            message: "Connection timed out".to_string(),
            details: Some("TCP timeout after 30s".to_string()),
            retryable: true,
            request_id_or_job_id: Some("job-xyz-789".to_string()),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: ToolErrorEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.code, "CRAWL_FAILED");
        assert_eq!(deserialized.message, "Connection timed out");
        assert_eq!(deserialized.details.as_deref(), Some("TCP timeout after 30s"));
        assert!(deserialized.retryable);
        assert_eq!(deserialized.request_id_or_job_id.as_deref(), Some("job-xyz-789"));
    }

    #[test]
    fn test_tool_error_envelope_minimal() {
        let json = r#"{"code":"NOT_FOUND","message":"Resource not found","retryable":false}"#;
        let envelope: ToolErrorEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.code, "NOT_FOUND");
        assert_eq!(envelope.message, "Resource not found");
        assert!(envelope.details.is_none());
        assert!(!envelope.retryable);
        assert!(envelope.request_id_or_job_id.is_none());
    }

    // --- MapWebsiteResponse tests ---

    #[test]
    fn test_map_website_response_roundtrip() {
        let resp = MapWebsiteResponse {
            url: "https://example.com".to_string(),
            total_urls: 3,
            urls: vec![
                "https://example.com/".to_string(),
                "https://example.com/about".to_string(),
                "https://example.com/contact".to_string(),
            ],
            search_filter: Some("about".to_string()),
            include_subdomains: false,
            duration_ms: 1234,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: MapWebsiteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.url, "https://example.com");
        assert_eq!(deserialized.total_urls, 3);
        assert_eq!(deserialized.urls.len(), 3);
        assert_eq!(deserialized.search_filter.as_deref(), Some("about"));
        assert!(!deserialized.include_subdomains);
        assert_eq!(deserialized.duration_ms, 1234);
    }

    #[test]
    fn test_map_website_response_without_search_filter() {
        let resp = MapWebsiteResponse {
            url: "https://example.com".to_string(),
            total_urls: 1,
            urls: vec!["https://example.com/".to_string()],
            search_filter: None,
            include_subdomains: true,
            duration_ms: 500,
        };
        let json = serde_json::to_string(&resp).unwrap();
        // search_filter should not appear in JSON when None
        assert!(!json.contains("search_filter"));
        let deserialized: MapWebsiteResponse = serde_json::from_str(&json).unwrap();
        assert!(deserialized.search_filter.is_none());
        assert!(deserialized.include_subdomains);
    }
}