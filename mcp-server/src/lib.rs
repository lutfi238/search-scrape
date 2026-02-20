pub mod search;
pub mod scrape;
pub mod crawl;
pub mod extract;
pub mod research;
pub mod types;
pub mod mcp;
pub mod rust_scraper;
pub mod stdio_service;
pub mod history;
pub mod query_rewriter;
pub mod llm_client;
pub mod crawl_jobs;

#[derive(Clone)]
pub struct AppState {
    pub searxng_url: String,
    pub http_client: reqwest::Client,
    // Caches for performance
    pub search_cache: moka::future::Cache<String, Vec<types::SearchResult>>, // key: query
    pub scrape_cache: moka::future::Cache<String, types::ScrapeResponse>,     // key: url
    // Concurrency control for external calls
    pub outbound_limit: std::sync::Arc<tokio::sync::Semaphore>,
    // Memory manager for research history (optional)
    pub memory: Option<std::sync::Arc<history::MemoryManager>>,
    // BYO LLM client for structured extraction (optional)
    pub llm: Option<std::sync::Arc<llm_client::LlmClient>>,
    // Async crawl job store
    pub crawl_jobs: std::sync::Arc<crawl_jobs::CrawlJobStore>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("searxng_url", &self.searxng_url)
            .field("memory_enabled", &self.memory.is_some())
            .field("llm_enabled", &self.llm.is_some())
            .finish()
    }
}

// Re-export AppState for easy access
pub use types::*;

impl AppState {
    pub fn new(searxng_url: String, http_client: reqwest::Client) -> Self {
        // Attempt to initialize LLM client from env; None if not configured
        let llm = match llm_client::LlmClient::from_env() {
            Ok(client) => {
                tracing::info!("LLM client configured: {:?}", client);
                Some(std::sync::Arc::new(client))
            }
            Err(e) => {
                tracing::debug!("LLM client not configured (optional): {}", e);
                None
            }
        };

        Self {
            searxng_url,
            http_client,
            search_cache: moka::future::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(std::time::Duration::from_secs(60 * 10))
                .build(),
            scrape_cache: moka::future::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(std::time::Duration::from_secs(60 * 30))
                .build(),
            outbound_limit: std::sync::Arc::new(tokio::sync::Semaphore::new(32)),
            memory: None, // Will be initialized if QDRANT_URL is set
            llm,
            crawl_jobs: std::sync::Arc::new(crawl_jobs::CrawlJobStore::new(
                std::time::Duration::from_secs(24 * 60 * 60), // 24-hour TTL
            )),
        }
    }

    pub fn with_memory(mut self, memory: std::sync::Arc<history::MemoryManager>) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn with_llm(mut self, llm: std::sync::Arc<llm_client::LlmClient>) -> Self {
        self.llm = Some(llm);
        self
    }
}