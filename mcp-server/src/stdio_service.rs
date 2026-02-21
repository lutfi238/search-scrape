use rmcp::{model::*, ServiceExt};
use std::env;
use std::sync::Arc;
use tracing::{error, info, warn};
use std::borrow::Cow;
use crate::{search, scrape, crawl, extract, research, AppState, history};

#[derive(Clone, Debug)]
pub struct McpService {
    pub state: Arc<AppState>,
}

impl McpService {
    pub async fn new() -> anyhow::Result<Self> {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();

        let searxng_url = env::var("SEARXNG_URL")
            .unwrap_or_else(|_| "http://localhost:8888".to_string());

        info!("Starting MCP Service");
        info!("SearXNG URL: {}", searxng_url);

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let mut state = AppState::new(searxng_url, http_client);

        // Initialize memory if QDRANT_URL is set
        if let Ok(qdrant_url) = std::env::var("QDRANT_URL") {
            info!("Initializing memory with Qdrant at: {}", qdrant_url);
            match history::MemoryManager::new(&qdrant_url).await {
                Ok(memory) => {
                    state = state.with_memory(Arc::new(memory));
                    info!("Memory initialized successfully");
                }
                Err(e) => {
                    warn!("Failed to initialize memory: {}. Continuing without memory feature.", e);
                }
            }
        } else {
            info!("QDRANT_URL not set. Memory feature disabled.");
        }

        Ok(Self { state: Arc::new(state) })
    }

    /// Returns all tool definitions. Extracted for testability.
    pub fn tool_definitions(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: Cow::Borrowed("search_web"),
                description: Some(Cow::Borrowed("Search the web using SearXNG federated search. Returns ranked results with domain classification and automatic query optimization.\n\nKEY FEATURES:\n• Auto-rewrites developer queries (e.g., 'rust docs' → adds 'site:doc.rust-lang.org')\n• Duplicate detection warns if query searched within 6 hours\n• Extracts domains and classifies sources (docs/repo/blog/news)\n• Shows query suggestions and instant answers when available\n\nAGENT BEST PRACTICES:\n1. Use categories='it' for programming/tech queries (gets better results)\n2. Start with max_results=5-10, increase to 20-50 for comprehensive research\n3. Check duplicate warnings - use research_history tool instead if duplicate detected\n4. Look for 'Query Optimization Tips' in output for better refinements\n5. Use time_range='week' for recent news, 'month' for current tech trends")),
                input_schema: match serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query. TIP: Use specific terms and quotes for exact phrases. Example: 'rust async' instead of just 'rust'"},
                        "engines": {"type": "string", "description": "Comma-separated engines (e.g., 'google,bing'). TIP: Omit for default. Use 'google,bing' for English content, add 'duckduckgo' for privacy-focused results"},
                        "categories": {"type": "string", "description": "Comma-separated categories. WHEN TO USE: 'it' for programming/tech, 'news' for current events, 'science' for research papers, 'general' for mixed. Omit for all categories"},
                        "language": {"type": "string", "description": "Language code (e.g., 'en', 'es', 'fr'). TIP: Use 'en' for English-only results, omit for multilingual"},
                        "safesearch": {"type": "integer", "minimum": 0, "maximum": 2, "description": "Safe search: 0=off, 1=moderate (recommended), 2=strict. Default env setting usually sufficient"},
                        "time_range": {"type": "string", "description": "Filter by recency. WHEN TO USE: 'day' for breaking news, 'week' for current events, 'month' for recent tech/trends, 'year' for last 12 months. Omit for all-time results"},
                        "pageno": {"type": "integer", "minimum": 1, "description": "Page number for pagination. TIP: Start with page 1, use page 2+ only if initial results insufficient"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Max results to return. GUIDANCE: 5-10 for quick facts, 15-25 for balanced research, 30-50 for comprehensive surveys. Default 10 is good for most queries. Higher = more tokens"}
                    },
                    "required": ["query"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
            Tool {
                name: Cow::Borrowed("scrape_url"),
                description: Some(Cow::Borrowed("Extract clean content from URLs with automatic code block detection, quality scoring, and metadata extraction.\n\nKEY FEATURES:\n• Extracts code blocks with language detection (returns array of {language, code})\n• Quality scoring (0.0-1.0) indicates content reliability\n• Automatic metadata: title, author, publish date, reading time\n• Citation-ready: Use [N] markers to reference extracted links\n• JSON mode: Set output_format='json' for structured data with all metadata\n\nAGENT BEST PRACTICES:\n1. For code examples: Use output_format='json' to get code_blocks array\n2. Set max_chars based on need: 3000-5000 (summary), 10000 (article), 30000+ (docs)\n3. Check extraction_score: <0.4 = low quality, >0.7 = high quality\n4. Check warnings array: 'short_content' = likely JS-heavy, 'low_extraction_score' = may need browser\n5. For documentation sites: Increase max_chars to 40000+ to capture full tutorials\n6. Use content_links_only=false only when you need navigation/sitemap links")),
                input_schema: match serde_json::json!({
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
                        }
                    },
                    "required": ["url"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
            Tool {
                name: Cow::Borrowed("research_history"),
                description: Some(Cow::Borrowed("Search past research using semantic similarity (vector search). Finds related searches/scrapes even with different wording.\n\nKEY FEATURES:\n• Semantic search finds related topics (e.g., 'rust tutorials' finds 'learning rust')\n• Returns similarity scores (0.0-1.0) showing relevance\n• Shows when each search was performed (helps avoid stale info)\n• Includes summaries and domains from past research\n• Persists across sessions (uses Qdrant vector DB)\n• Filter by type: 'search' for web searches, 'scrape' for scraped pages\n\nAGENT BEST PRACTICES:\n1. **Use FIRST before new searches** - Saves API calls and finds existing research\n2. Set threshold=0.6-0.7 for broad exploration, 0.75-0.85 for specific matches\n3. Use entry_type='search' to find past searches, 'scrape' for scraped content history\n4. Check timestamps: Recent results (<24h) are more reliable than old ones\n5. Use limit=5-10 for quick checks, 20+ for comprehensive review\n6. If similarity >0.9, you likely already researched this exact topic\n7. Combine with search_web/scrape_url: Check history first, then fetch if not found\n\nNOTE: Only available when Qdrant is running (QDRANT_URL configured)")),
                input_schema: match serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Topic or question to search in history. Use natural language. Example: 'rust async web scraping' or 'how to configure Qdrant'"
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 50,
                            "default": 10,
                            "description": "Max number of results to return. GUIDANCE: 5-10 for quick context, 20+ for comprehensive review"
                        },
                        "threshold": {
                            "type": "number",
                            "minimum": 0.0,
                            "maximum": 1.0,
                            "default": 0.7,
                            "description": "Similarity threshold (0-1). GUIDANCE: 0.6-0.7 for broad topics, 0.75-0.85 for specific queries, 0.9+ for near-exact matches"
                        },
                        "entry_type": {
                            "type": "string",
                            "description": "Filter by entry type. Use 'search' for past web searches, 'scrape' for scraped pages. Omit to search both types.",
                            "enum": ["search", "scrape"]
                        }
                    },
                    "required": ["query"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
            Tool {
                name: Cow::Borrowed("scrape_batch"),
                description: Some(Cow::Borrowed("Scrape multiple URLs concurrently in a single request. Ideal for bulk content extraction from search results or link lists.\n\nKEY FEATURES:\n• Concurrent scraping with configurable parallelism (default: 10, max: 50)\n• Returns success/failure status for each URL with timing info\n• Automatic retry logic and error handling per URL\n• Same quality extraction as scrape_url (code blocks, metadata, etc.)\n• Efficient: Uses connection pooling and caching\n\nAGENT BEST PRACTICES:\n1. Use after search_web to scrape top results in one call\n2. Set max_concurrent=5-10 for stability, increase to 20-30 for speed\n3. Keep max_chars=3000-5000 per URL to manage total response size\n4. Check 'failed' count in response - some URLs may be unreachable\n5. Use output_format='json' for programmatic processing\n6. For 50+ URLs, consider batching into multiple calls\n\nPERFORMANCE:\n• 10 URLs @ concurrency 10: ~2-5 seconds\n• 50 URLs @ concurrency 20: ~5-15 seconds\n• Failed URLs don't block others")),
                input_schema: match serde_json::json!({
                    "type": "object",
                    "properties": {
                        "urls": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "List of URLs to scrape. Maximum 100 URLs per request.",
                            "minItems": 1,
                            "maxItems": 100
                        },
                        "max_concurrent": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 50,
                            "default": 10,
                            "description": "Max concurrent requests. GUIDANCE: 5-10 for stability, 15-30 for speed, 50 max. Higher = faster but more resource intensive"
                        },
                        "max_chars": {
                            "type": "integer",
                            "minimum": 100,
                            "maximum": 50000,
                            "default": 10000,
                            "description": "Max content chars per URL. GUIDANCE: 3000-5000 for summaries, 10000 for articles. Lower values = smaller total response"
                        },
                        "output_format": {
                            "type": "string",
                            "enum": ["text", "json"],
                            "default": "json",
                            "description": "Output format. 'json' (default) returns structured data, 'text' returns formatted markdown summary"
                        }
                    },
                    "required": ["urls"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
            Tool {
                name: Cow::Borrowed("crawl_website"),
                description: Some(Cow::Borrowed("Recursively crawl a website to discover and extract content from multiple pages. Ideal for documentation sites, blogs, or any multi-page content.\n\nKEY FEATURES:\n• BFS crawling with configurable depth (default: 3 levels)\n• Smart link filtering (same domain, exclude patterns)\n• Concurrent page processing for speed\n• Returns sitemap of all discovered URLs\n• Content preview for each page\n• Automatic deduplication of URLs\n\nAGENT BEST PRACTICES:\n1. Start with max_depth=2 and max_pages=20 for exploration\n2. Increase to max_depth=3-5 and max_pages=50-100 for comprehensive crawls\n3. Use include_patterns to focus on specific sections (e.g., ['/docs/', '/guide/'])\n4. Use exclude_patterns to skip unwanted content (e.g., ['/api/', '/changelog/'])\n5. Set same_domain_only=true (default) to avoid crawling external sites\n6. Check 'sitemap' in response for list of all discovered URLs\n\nPERFORMANCE:\n• 20 pages @ depth 2: ~10-20 seconds\n• 50 pages @ depth 3: ~30-60 seconds\n• Uses caching - repeated URLs are fast")),
                input_schema: match serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "Starting URL to crawl. Should be the root or section root of the site you want to explore."
                        },
                        "max_depth": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 10,
                            "default": 3,
                            "description": "Maximum link depth to crawl. GUIDANCE: 1=start page only, 2=start+linked pages, 3=comprehensive (default), 5+=deep crawl. Higher = more pages but slower"
                        },
                        "max_pages": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 500,
                            "default": 50,
                            "description": "Maximum total pages to crawl. GUIDANCE: 10-20 for quick exploration, 50 (default) for standard sites, 100-200 for large docs, 500 max"
                        },
                        "max_concurrent": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 20,
                            "default": 5,
                            "description": "Concurrent page requests. GUIDANCE: 3-5 (default) for stability, 10-15 for speed on robust servers"
                        },
                        "same_domain_only": {
                            "type": "boolean",
                            "default": true,
                            "description": "Only crawl pages on the same domain (and subdomains). Set false to follow external links"
                        },
                        "include_patterns": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Only crawl URLs containing these patterns. Example: ['/docs/', '/tutorial/'] to focus on documentation"
                        },
                        "exclude_patterns": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Skip URLs containing these patterns. Default excludes common non-content paths (/login, /api/, .pdf, etc.)"
                        },
                        "max_chars_per_page": {
                            "type": "integer",
                            "minimum": 100,
                            "maximum": 50000,
                            "default": 5000,
                            "description": "Max content chars per page in results. Lower = smaller response size"
                        }
                    },
                    "required": ["url"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
            Tool {
                name: Cow::Borrowed("extract_structured"),
                description: Some(Cow::Borrowed("Extract structured JSON data from a webpage using schema definitions or natural language prompts. No external LLM required - uses intelligent pattern matching.\n\nKEY FEATURES:\n• Schema-based extraction: Define fields with names, types, and descriptions\n• Prompt-based extraction: Describe what you want in natural language\n• Auto-detection: Automatically finds emails, prices, dates, phones\n• Built-in patterns: Products, articles, contacts, code blocks\n• Metadata included: Title, author, publish date, word count\n\nEXTRACTION TYPES:\n• string: Text content\n• number: Numeric values\n• boolean: True/false\n• array: Lists of items\n• object: Nested structures\n\nBUILT-IN FIELDS (use these names for automatic extraction):\n• title, description, author, date, published\n• price, email, phone\n• links, headings, images, code_blocks\n\nUSE CASES:\n• Product pages: Extract name, price, description, images\n• Articles: Extract title, author, date, content summary\n• Contact pages: Extract emails, phones, addresses\n• Documentation: Extract headings, code examples")),
                input_schema: match serde_json::json!({
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
                            "description": "Schema defining fields to extract. Each field has name, description, optional type and required flag"
                        },
                        "prompt": {
                            "type": "string",
                            "description": "Natural language description of what to extract. Example: 'Extract product information including name, price, and features' or 'Find all contact information'"
                        },
                        "max_chars": {
                            "type": "integer",
                            "minimum": 100,
                            "maximum": 50000,
                            "default": 10000,
                            "description": "Max chars of raw content to include in response"
                        }
                    },
                    "required": ["url"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
            Tool {
                name: Cow::Borrowed("deep_research"),
                description: Some(Cow::Borrowed("Perform deep research on a topic by combining search, crawl, and analysis. Ideal for comprehensive understanding of complex topics.\n\nWORKFLOW:\n1. Search the web for relevant pages on the topic\n2. Scrape top search results for full content\n3. Optionally crawl linked pages for more depth\n4. Analyze and summarize all gathered information\n5. Extract topics, key findings, and related queries\n\nKEY FEATURES:\n• Multi-source aggregation from search results\n• Automatic source type classification (docs, repo, blog, Q&A, news)\n• Topic clustering from headings across sources\n• Relevance scoring based on content quality\n• Code block detection and counting\n• Related query suggestions for follow-up research\n\nAGENT BEST PRACTICES:\n1. Use for complex topics requiring multiple sources\n2. Start with max_search_results=10, increase for broader coverage\n3. Set crawl_depth=0 for quick research, 2 for comprehensive\n4. Use include_domains to focus on trusted sources\n5. Use exclude_domains to skip low-quality sites\n6. Check 'key_findings' for quick insights\n7. Use 'related_queries' for follow-up research\n\nPERFORMANCE:\n• 10 sources, no crawl: ~10-30 seconds\n• 10 sources, crawl depth 2: ~30-90 seconds\n• Results are cached for repeated queries")),
                input_schema: match serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Research topic or question. Be specific for better results. Example: 'How to implement authentication in Next.js 14'"
                        },
                        "max_search_results": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 30,
                            "default": 10,
                            "description": "Number of search results to analyze. GUIDANCE: 5 for quick research, 10 (default) for standard, 20-30 for comprehensive"
                        },
                        "max_pages_per_site": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 20,
                            "default": 5,
                            "description": "Max pages to crawl from each domain. Higher = more depth per source"
                        },
                        "max_total_pages": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 100,
                            "default": 30,
                            "description": "Total page limit across all sources. Controls overall research breadth"
                        },
                        "crawl_depth": {
                            "type": "integer",
                            "minimum": 0,
                            "maximum": 3,
                            "default": 2,
                            "description": "How deep to crawl from each search result. 0=no crawl (just scrape), 1-2=follow links, 3=deep crawl"
                        },
                        "include_domains": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Only include results from these domains. Example: ['rust-lang.org', 'docs.rs', 'github.com']"
                        },
                        "exclude_domains": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Exclude results from these domains. Example: ['pinterest.com', 'facebook.com']"
                        },
                        "search_engines": {
                            "type": "string",
                            "description": "Comma-separated search engines. Example: 'google,bing,duckduckgo'"
                        },
                        "time_range": {
                            "type": "string",
                            "enum": ["day", "week", "month", "year"],
                            "description": "Limit to recent content. Useful for fast-moving topics"
                        },
                        "language": {
                            "type": "string",
                            "description": "Preferred language code (e.g., 'en', 'id', 'ja')"
                        }
                    },
                    "required": ["query"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
            Tool {
                name: Cow::Borrowed("map_website"),
                description: Some(Cow::Borrowed("Discover all URLs on a website by crawling and returning a URL list (sitemap-like). Lightweight discovery-only tool -- does not return page content.\n\nKEY FEATURES:\n• Fast URL discovery using BFS crawl\n• Optional search filter to match URLs by substring\n• Subdomain inclusion control\n• Sitemap mode: 'crawl' (default, follows links) or 'sitemap_xml' (parses sitemap.xml)\n• Returns a flat list of discovered URLs\n\nAGENT BEST PRACTICES:\n1. Use to get a site map before targeted scraping\n2. Set limit=100-500 to control output size\n3. Use search filter to narrow results (e.g., '/docs/' or '/blog/')\n4. Set include_subdomains=true to discover across subdomains\n5. For large sites, start with sitemap_mode='sitemap_xml' for fast results")),
                input_schema: match serde_json::json!({
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
                            "description": "Maximum number of URLs to return. GUIDANCE: 50-100 for quick overview, 500+ for comprehensive mapping"
                        },
                        "search": {
                            "type": "string",
                            "description": "Filter URLs by substring match. Example: '/docs/' to only return documentation pages. Case-insensitive"
                        },
                        "include_subdomains": {
                            "type": "boolean",
                            "default": true,
                            "description": "Include URLs from subdomains (e.g., docs.example.com when mapping example.com). Always stays website-scoped; never follows external domains. Default: true"
                        },
                        "sitemap_mode": {
                            "type": "string",
                            "enum": ["crawl", "sitemap_xml"],
                            "default": "crawl",
                            "description": "Discovery method. 'crawl' (default) follows links via BFS. 'sitemap_xml' attempts to parse /sitemap.xml first (faster for sites that provide it)"
                        }
                    },
                    "required": ["url"]
                }) {
                    serde_json::Value::Object(map) => std::sync::Arc::new(map),
                    _ => std::sync::Arc::new(serde_json::Map::new()),
                },
                output_schema: None,
                annotations: None,
            },
        ]
    }

    /// Build a CrawlConfig for the map_website tool.
    /// Always website-scoped: same_domain_only is always true.
    pub fn build_map_crawl_config(limit: usize, _include_subdomains: bool) -> crawl::CrawlConfig {
        let mut config = crawl::CrawlConfig::default();
        config.max_pages = limit.min(5000);
        config.max_depth = 5;
        config.max_concurrent = 10;
        config.same_domain_only = true;
        config.max_chars_per_page = 100;
        config
    }
}

impl rmcp::ServerHandler for McpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            server_info: Implementation {
                name: "search-scrape".to_string(),
                version: "1.0.0".to_string(),
            },
            instructions: Some(
                "A pure Rust web search and scraping service using SearXNG for federated search and a native Rust scraper for content extraction.".to_string(),
            ),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
        }
    }

    async fn list_tools(
        &self,
        _page: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.tool_definitions();
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        info!("MCP tool call: {} with args: {:?}", request.name, request.arguments);
        match request.name.as_ref() {
            "search_web" => {
                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: query",
                        None,
                    ))?;
                
                let engines = args.get("engines").and_then(|v| v.as_str()).map(|s| s.to_string());
                let categories = args.get("categories").and_then(|v| v.as_str()).map(|s| s.to_string());
                let language = args.get("language").and_then(|v| v.as_str()).map(|s| s.to_string());
                let time_range = args.get("time_range").and_then(|v| v.as_str()).map(|s| s.to_string());
                let safesearch = args.get("safesearch").and_then(|v| v.as_i64()).and_then(|n| if (0..=2).contains(&n) { Some(n as u8) } else { None });
                let pageno = args.get("pageno").and_then(|v| v.as_u64()).map(|n| n as u32);

                let max_results = args.get("max_results").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(10);
                let overrides = crate::search::SearchParamOverrides { engines, categories, language, safesearch, time_range, pageno };

                match search::search_web_with_params(&self.state, query, Some(overrides)).await {
                    Ok((results, extras)) => {
                        let content_text = if results.is_empty() {
                            let mut text = format!("No search results found for query: '{}'\n\n", query);
                            
                            // Show suggestions/corrections to help user refine query
                            if !extras.suggestions.is_empty() {
                                text.push_str(&format!("**Suggestions:** {}\n", extras.suggestions.join(", ")));
                            }
                            if !extras.corrections.is_empty() {
                                text.push_str(&format!("**Did you mean:** {}\n", extras.corrections.join(", ")));
                            }
                            if !extras.unresponsive_engines.is_empty() {
                                text.push_str(&format!("\n**Note:** {} search engine(s) did not respond. Try different engines or retry.\n", extras.unresponsive_engines.len()));
                            }
                            text
                        } else {
                            let limited_results = results.iter().take(max_results);
                            let result_count = results.len();
                            let _showing = result_count.min(max_results);
                            
                            let mut text = String::new();
                            
                            // Phase 2: Show duplicate warning if present
                            if let Some(dup_warning) = &extras.duplicate_warning {
                                text.push_str(&format!("{}\n\n", dup_warning));
                            }
                            
                            // Phase 2: Show query rewrite info if query was enhanced
                            if let Some(ref rewrite) = extras.query_rewrite {
                                if rewrite.was_rewritten() {
                                    text.push_str(&format!("🔍 **Query Enhanced:** '{}' → '{}'\n\n", rewrite.original, rewrite.best_query()));
                                } else if rewrite.is_developer_query && !rewrite.suggestions.is_empty() {
                                    text.push_str("💡 **Query Optimization Tips:**\n");
                                    for (i, suggestion) in rewrite.suggestions.iter().take(2).enumerate() {
                                        text.push_str(&format!("   {}. {}\n", i + 1, suggestion));
                                    }
                                    text.push('\n');
                                }
                            }
                            
                            text.push_str(&format!("Found {} search results for '{}':", result_count, query));
                            if result_count > max_results {
                                text.push_str(&format!(" (showing top {})\n", max_results));
                            }
                            text.push_str("\n\n");
                            
                            // Show instant answers first if available
                            if !extras.answers.is_empty() {
                                text.push_str("**Instant Answers:**\n");
                                for answer in &extras.answers {
                                    text.push_str(&format!("📌 {}\n\n", answer));
                                }
                            }
                            
                            // Show search results
                            for (i, result) in limited_results.enumerate() {
                                text.push_str(&format!(
                                    "{}. **{}**\n   URL: {}\n   Snippet: {}\n\n",
                                    i + 1,
                                    result.title,
                                    result.url,
                                    result.content.chars().take(200).collect::<String>()
                                ));
                            }
                            
                            // Show helpful metadata at the end
                            if !extras.suggestions.is_empty() {
                                text.push_str(&format!("\n**Related searches:** {}\n", extras.suggestions.join(", ")));
                            }
                            if !extras.unresponsive_engines.is_empty() {
                                text.push_str(&format!("\n⚠️ **Note:** {} engine(s) did not respond (may affect completeness)\n", extras.unresponsive_engines.len()));
                            }
                            
                            text
                        };
                        
                        Ok(CallToolResult::success(vec![Content::text(content_text)]))
                    }
                    Err(e) => {
                        error!("Search tool error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!("Search failed: {}", e))]))
                    }
                }
            }
            "scrape_url" => {
                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: url",
                        None,
                    ))?;
                
                self.state.scrape_cache.invalidate(url).await;
                
                match scrape::scrape_url(&self.state, url).await {
                    Ok(mut content) => {
                        info!("Scraped content: {} words, {} chars clean_content, score: {:?}", 
                              content.word_count, content.clean_content.len(), content.extraction_score);
                        
                        let max_chars = args
                            .get("max_chars")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as usize)
                            .or_else(|| std::env::var("MAX_CONTENT_CHARS").ok().and_then(|s| s.parse().ok()))
                            .unwrap_or(10000);
                        
                        // Set truncation metadata (Priority 1)
                        content.actual_chars = content.clean_content.len();
                        content.max_chars_limit = Some(max_chars);
                        content.truncated = content.clean_content.len() > max_chars;
                        
                        if content.truncated {
                            content.warnings.push("content_truncated".to_string());
                        }
                        if content.word_count < 50 {
                            content.warnings.push("short_content".to_string());
                        }
                        if content.extraction_score.map(|s| s < 0.4).unwrap_or(false) {
                            content.warnings.push("low_extraction_score".to_string());
                        }
                        
                        // Check for output_format parameter (Priority 1)
                        let output_format = args
                            .get("output_format")
                            .and_then(|v| v.as_str())
                            .unwrap_or("text");
                        
                        if output_format == "json" {
                            // Return JSON format
                            let json_str = serde_json::to_string_pretty(&content)
                                .unwrap_or_else(|e| format!(r#"{{"error": "Failed to serialize: {}"}}"#, e));
                            return Ok(CallToolResult::success(vec![Content::text(json_str)]));
                        }
                        
                        // Otherwise return formatted text (backward compatible)
                        let content_preview = if content.clean_content.is_empty() {
                            let msg = "[No content extracted]\n\n**Possible reasons:**\n\
                            • Page is JavaScript-heavy (requires browser execution)\n\
                            • Content is behind authentication/paywall\n\
                            • Site blocks automated access\n\n\
                            **Suggestion:** For JS-heavy sites, try using the Playwright MCP tool instead.";
                            msg.to_string()
                        } else if content.word_count < 10 {
                            format!("{}\n\n⚠️ **Very short content** ({} words). Page may be mostly dynamic/JS-based.", 
                                content.clean_content.chars().take(max_chars).collect::<String>(),
                                content.word_count)
                        } else {
                            let preview = content.clean_content.chars().take(max_chars).collect::<String>();
                            if content.clean_content.len() > max_chars {
                                format!("{}\n\n[Content truncated: {}/{} chars shown. Increase max_chars parameter to see more]",
                                    preview, max_chars, content.clean_content.len())
                            } else {
                                preview
                            }
                        };
                        
                        // Build Sources section from links
                        let sources_section = if content.links.is_empty() {
                            String::new()
                        } else {
                            let mut sources = String::from("\n\n**Sources:**\n");
                            // Get max_links from args or env var or default
                            let max_sources = args
                                .get("max_links")
                                .and_then(|v| v.as_u64())
                                .map(|n| n as usize)
                                .or_else(|| std::env::var("MAX_LINKS").ok().and_then(|s| s.parse().ok()))
                                .unwrap_or(100);
                            let link_count = content.links.len();
                            for (i, link) in content.links.iter().take(max_sources).enumerate() {
                                if !link.text.is_empty() {
                                    sources.push_str(&format!("[{}]: {} ({})", i + 1, link.url, link.text));
                                } else {
                                    sources.push_str(&format!("[{}]: {}", i + 1, link.url));
                                }
                                sources.push('\n');
                            }
                            if link_count > max_sources {
                                sources.push_str(&format!("\n(Showing {} of {} total links)\n", max_sources, link_count));
                            }
                            sources
                        };
                        
                        let content_text = format!(
                            "**{}**\n\nURL: {}\nWord Count: {}\nLanguage: {}\n\n**Content:**\n{}\n\n**Metadata:**\n- Description: {}\n- Keywords: {}\n\n**Headings:**\n{}\n\n**Links Found:** {}\n**Images Found:** {}{}",
                            content.title,
                            content.url,
                            content.word_count,
                            content.language,
                            content_preview,
                            content.meta_description,
                            content.meta_keywords,
                            content.headings.iter()
                                .map(|h| format!("- {} {}", h.level.to_uppercase(), h.text))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            content.links.len(),
                            content.images.len(),
                            sources_section
                        );
                        
                        Ok(CallToolResult::success(vec![Content::text(content_text)]))
                    }
                    Err(e) => {
                        error!("Scrape tool error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!("Scraping failed: {}", e))]))
                    }
                }
            }
            "research_history" => {
                // Check if memory is enabled
                let memory = match &self.state.memory {
                    Some(m) => m,
                    None => {
                        return Ok(CallToolResult::success(vec![Content::text(
                            "Research history feature is not available. Set QDRANT_URL environment variable to enable.\n\nExample: QDRANT_URL=http://localhost:6333".to_string()
                        )]));
                    }
                };

                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;
                
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: query",
                        None,
                    ))?;
                
                let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(10);
                let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.7) as f32;
                
                // Parse entry_type filter if provided
                let entry_type_filter = args.get("entry_type")
                    .and_then(|v| v.as_str())
                    .and_then(|s| match s.to_lowercase().as_str() {
                        "search" => Some(crate::history::EntryType::Search),
                        "scrape" => Some(crate::history::EntryType::Scrape),
                        _ => None
                    });

                match memory.search_history(query, limit, threshold, entry_type_filter).await {
                    Ok(results) => {
                        if results.is_empty() {
                            let text = format!("No relevant history found for: '{}'\n\nTry:\n- Lower threshold (currently {:.2})\n- Broader search terms\n- Check if you have any saved history", query, threshold);
                            Ok(CallToolResult::success(vec![Content::text(text)]))
                        } else {
                            let mut text = format!("Found {} relevant entries for '{}':\n\n", results.len(), query);
                            
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
                                
                                // query field is always a String, show it
                                text.push_str(&format!("   Query: {}\n", entry.query));
                                
                                text.push('\n');
                            }
                            
                            text.push_str(&format!("\n💡 Tip: Use threshold={:.2} for similar results, or higher (0.8-0.9) for more specific matches", threshold));
                            
                            Ok(CallToolResult::success(vec![Content::text(text)]))
                        }
                    }
                    Err(e) => {
                        error!("History search error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!("History search failed: {}", e))]))
                    }
                }
            }
            "scrape_batch" => {
                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;

                // Parse URLs array
                let urls: Vec<String> = args
                    .get("urls")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: urls (array of strings)",
                        None,
                    ))?;

                if urls.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "Error: urls array cannot be empty".to_string()
                    )]));
                }

                if urls.len() > 100 {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("Error: Maximum 100 URLs per request, got {}", urls.len())
                    )]));
                }

                let max_concurrent = args.get("max_concurrent").and_then(|v| v.as_u64()).map(|n| n as usize);
                let max_chars = args.get("max_chars").and_then(|v| v.as_u64()).map(|n| n as usize);
                let output_format = args.get("output_format").and_then(|v| v.as_str()).unwrap_or("json");

                info!("Batch scraping {} URLs", urls.len());

                match scrape::scrape_batch(&self.state, urls, max_concurrent, max_chars).await {
                    Ok(response) => {
                        if output_format == "json" {
                            // Return JSON format
                            let json_str = serde_json::to_string_pretty(&response)
                                .unwrap_or_else(|e| format!(r#"{{"error": "Failed to serialize: {}"}}"#, e));
                            Ok(CallToolResult::success(vec![Content::text(json_str)]))
                        } else {
                            // Return text format summary
                            let mut text = format!(
                                "**Batch Scrape Results**\n\nTotal: {} | Successful: {} | Failed: {} | Duration: {}ms\n\n",
                                response.total, response.successful, response.failed, response.total_duration_ms
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

                            Ok(CallToolResult::success(vec![Content::text(text)]))
                        }
                    }
                    Err(e) => {
                        error!("Batch scrape error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!("Batch scrape failed: {}", e))]))
                    }
                }
            }
            "crawl_website" => {
                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;

                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: url",
                        None,
                    ))?;

                // Parse configuration from args
                let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(3);
                let max_pages = args.get("max_pages").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(50);
                let max_concurrent = args.get("max_concurrent").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(5);
                let same_domain_only = args.get("same_domain_only").and_then(|v| v.as_bool()).unwrap_or(true);
                let max_chars_per_page = args.get("max_chars_per_page").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(5000);

                let include_patterns: Vec<String> = args
                    .get("include_patterns")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();

                let exclude_patterns: Vec<String> = args
                    .get("exclude_patterns")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();

                // Build config with defaults + user overrides
                let mut config = crawl::CrawlConfig::default();
                config.max_depth = max_depth.min(10);
                config.max_pages = max_pages.min(500);
                config.max_concurrent = max_concurrent.min(20);
                config.same_domain_only = same_domain_only;
                config.max_chars_per_page = max_chars_per_page;

                if !include_patterns.is_empty() {
                    config.include_patterns = include_patterns;
                }
                if !exclude_patterns.is_empty() {
                    // Merge with defaults instead of replacing
                    for pattern in exclude_patterns {
                        if !config.exclude_patterns.contains(&pattern) {
                            config.exclude_patterns.push(pattern);
                        }
                    }
                }

                info!("Starting crawl of {} (depth: {}, max_pages: {})", url, config.max_depth, config.max_pages);

                match crawl::crawl_website(&self.state, url, config).await {
                    Ok(response) => {
                        // Build text response
                        let mut text = format!(
                            "**Crawl Results for {}**\n\n\
                            📊 **Summary:**\n\
                            • Pages crawled: {}\n\
                            • Pages failed: {}\n\
                            • Max depth reached: {}\n\
                            • Unique domains: {}\n\
                            • Total duration: {}ms\n\n",
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
                                    result.title.as_deref().unwrap_or("Untitled").chars().take(60).collect::<String>(),
                                    result.url,
                                    result.depth,
                                    result.word_count.unwrap_or(0),
                                    result.links_found.unwrap_or(0),
                                    result.duration_ms
                                ));
                                if let Some(preview) = &result.content_preview {
                                    let short_preview: String = preview.chars().take(200).collect();
                                    text.push_str(&format!("   Preview: {}...\n", short_preview.replace('\n', " ")));
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

                        // Add sitemap
                        if let Some(sitemap) = &response.sitemap {
                            if !sitemap.is_empty() {
                                text.push_str(&format!("\n**🗺️ Sitemap ({} URLs):**\n", sitemap.len()));
                                for url in sitemap.iter().take(50) {
                                    text.push_str(&format!("• {}\n", url));
                                }
                                if sitemap.len() > 50 {
                                    text.push_str(&format!("... and {} more URLs\n", sitemap.len() - 50));
                                }
                            }
                        }

                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    Err(e) => {
                        error!("Crawl error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!("Crawl failed: {}", e))]))
                    }
                }
            }
            "extract_structured" => {
                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;

                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: url",
                        None,
                    ))?;

                // Parse schema if provided
                let schema: Option<Vec<crate::types::ExtractField>> = args
                    .get("schema")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| {
                                let name = item.get("name")?.as_str()?.to_string();
                                let description = item.get("description")?.as_str()?.to_string();
                                let field_type = item.get("field_type").and_then(|v| v.as_str()).map(|s| s.to_string());
                                let required = item.get("required").and_then(|v| v.as_bool());
                                Some(crate::types::ExtractField { name, description, field_type, required })
                            })
                            .collect()
                    });

                let prompt = args.get("prompt").and_then(|v| v.as_str()).map(|s| s.to_string());
                let max_chars = args.get("max_chars").and_then(|v| v.as_u64()).map(|n| n as usize);

                info!("Extracting structured data from: {}", url);

                match extract::extract_structured(&self.state, url, schema, prompt, max_chars).await {
                    Ok(response) => {
                        let mut text = format!(
                            "**Structured Extraction Results**\n\n\
                            📊 **Extraction Info:**\n\
                            • URL: {}\n\
                            • Title: {}\n\
                            • Method: {}\n\
                            • Fields Extracted: {}\n\
                            • Confidence: {:.0}%\n\
                            • Duration: {}ms\n\n",
                            response.url,
                            response.title,
                            response.extraction_method,
                            response.field_count,
                            response.confidence * 100.0,
                            response.duration_ms
                        );

                        if !response.warnings.is_empty() {
                            text.push_str(&format!("⚠️ **Warnings:** {}\n\n", response.warnings.join(", ")));
                        }

                        // Format extracted data as JSON
                        text.push_str("**📋 Extracted Data:**\n```json\n");
                        let json_str = serde_json::to_string_pretty(&response.extracted_data)
                            .unwrap_or_else(|_| "{}".to_string());
                        text.push_str(&json_str);
                        text.push_str("\n```\n\n");

                        // Raw content preview
                        text.push_str("**📄 Raw Content Preview:**\n");
                        let preview: String = response.raw_content_preview.chars().take(1000).collect();
                        text.push_str(&preview);
                        if response.raw_content_preview.len() > 1000 {
                            text.push_str("...\n[truncated]");
                        }

                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    Err(e) => {
                        error!("Extract error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!("Extraction failed: {}", e))]))
                    }
                }
            }
            "deep_research" => {
                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;

                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: query",
                        None,
                    ))?;

                // Parse configuration
                let max_search_results = args.get("max_search_results").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(10);
                let max_pages_per_site = args.get("max_pages_per_site").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(5);
                let max_total_pages = args.get("max_total_pages").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(30);
                let crawl_depth = args.get("crawl_depth").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(2);
                let search_engines = args.get("search_engines").and_then(|v| v.as_str()).map(|s| s.to_string());
                let time_range = args.get("time_range").and_then(|v| v.as_str()).map(|s| s.to_string());
                let language = args.get("language").and_then(|v| v.as_str()).map(|s| s.to_string());

                let include_domains: Vec<String> = args
                    .get("include_domains")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();

                let exclude_domains: Vec<String> = args
                    .get("exclude_domains")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();

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

                info!("Starting deep research for: {}", query);

                match research::deep_research(&self.state, query, config).await {
                    Ok(response) => {
                        let mut text = format!(
                            "# 🔬 Deep Research Results\n\n\
                            **Query:** {}\n\n\
                            ## 📊 Statistics\n\
                            • Search results: {}\n\
                            • Pages scraped: {}\n\
                            • Pages crawled: {}\n\
                            • Total words: {}\n\
                            • Unique domains: {}\n\
                            • Code blocks found: {}\n\
                            • Duration: {}ms\n\n",
                            response.query,
                            response.statistics.search_results_found,
                            response.statistics.pages_scraped,
                            response.statistics.pages_crawled,
                            response.statistics.total_words,
                            response.statistics.unique_domains,
                            response.statistics.code_blocks_found,
                            response.statistics.duration_ms
                        );

                        // Warnings
                        if !response.warnings.is_empty() {
                            text.push_str(&format!("⚠️ **Warnings:** {}\n\n", response.warnings.join(", ")));
                        }

                        // Summary
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

                        // Key Findings
                        if !response.key_findings.is_empty() {
                            text.push_str("## 💡 Key Findings\n\n");
                            for finding in &response.key_findings {
                                text.push_str(&format!("• {}\n", finding));
                            }
                            text.push('\n');
                        }

                        // Topics
                        if !response.topics.is_empty() {
                            text.push_str("## 🏷️ Topics Discovered\n\n");
                            for topic in response.topics.iter().take(8) {
                                text.push_str(&format!(
                                    "• **{}** (mentioned {} times across {} sources)\n",
                                    topic.topic, topic.mentions, topic.sources.len()
                                ));
                            }
                            text.push('\n');
                        }

                        // Sources
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

                            // Show top headings
                            if !source.headings.is_empty() {
                                let headings_preview: String = source.headings.iter().take(3).cloned().collect::<Vec<_>>().join(" | ");
                                text.push_str(&format!("   📑 {}\n", headings_preview));
                            }

                            // Content preview
                            let preview: String = source.content_preview.chars().take(150).collect();
                            text.push_str(&format!("   {}\n\n", preview.replace('\n', " ")));
                        }

                        if response.sources.len() > 15 {
                            text.push_str(&format!("... and {} more sources\n\n", response.sources.len() - 15));
                        }

                        // Related Queries
                        if !response.related_queries.is_empty() {
                            text.push_str("## 🔍 Related Queries for Further Research\n\n");
                            for query in response.related_queries.iter().take(5) {
                                text.push_str(&format!("• {}\n", query));
                            }
                        }

                        // Content types breakdown
                        text.push_str("\n## 📊 Content Types\n\n");
                        for (content_type, count) in &response.summary.content_types {
                            text.push_str(&format!("• {}: {}\n", content_type, count));
                        }

                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                    Err(e) => {
                        error!("Deep research error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!(
                            "Deep research failed: {}\n\n\
                            **Suggestions:**\n\
                            • Try a more specific query\n\
                            • Check if SearXNG is running\n\
                            • Reduce max_search_results or max_total_pages\n\
                            • Try different search_engines",
                            e
                        ))]))
                    }
                }
            }
            "map_website" => {
                let args = request.arguments.as_ref().ok_or_else(|| ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing required arguments object",
                    None,
                ))?;

                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing required parameter: url",
                        None,
                    ))?;

                let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(100);
                let search_filter = args.get("search").and_then(|v| v.as_str()).map(|s| s.to_string());
                let include_subdomains = args.get("include_subdomains").and_then(|v| v.as_bool()).unwrap_or(true);
                let sitemap_mode = args.get("sitemap_mode").and_then(|v| v.as_str()).unwrap_or("crawl");

                info!("Mapping website: {} (limit: {}, mode: {})", url, limit, sitemap_mode);

                let start_time = std::time::Instant::now();

                // Build crawl config -- always website-scoped.
                // The include_subdomains flag is handled via post-filtering below.
                let config = Self::build_map_crawl_config(limit, include_subdomains);

                // Extract the base host for subdomain filtering
                let base_host = url::Url::parse(url)
                    .ok()
                    .and_then(|u| u.host_str().map(|h| h.to_lowercase()));

                match crawl::crawl_website(&self.state, url, config).await {
                    Ok(response) => {
                        // Extract URLs from crawl results
                        let mut discovered_urls: Vec<String> = response.results
                            .iter()
                            .filter(|r| r.success)
                            .map(|r| r.url.clone())
                            .collect();

                        // When include_subdomains is false, restrict to exact base host
                        if !include_subdomains {
                            if let Some(ref base) = base_host {
                                discovered_urls.retain(|u| {
                                    url::Url::parse(u)
                                        .ok()
                                        .and_then(|parsed| parsed.host_str().map(|h| h.to_lowercase() == *base))
                                        .unwrap_or(false)
                                });
                            }
                        }

                        // Apply search filter if provided
                        if let Some(ref filter) = search_filter {
                            let filter_lower = filter.to_lowercase();
                            discovered_urls.retain(|u| u.to_lowercase().contains(&filter_lower));
                        }

                        // Sort for deterministic output
                        discovered_urls.sort();
                        discovered_urls.dedup();

                        // Apply limit
                        discovered_urls.truncate(limit);

                        let map_response = crate::types::MapWebsiteResponse {
                            url: url.to_string(),
                            total_urls: discovered_urls.len(),
                            urls: discovered_urls,
                            search_filter,
                            include_subdomains,
                            duration_ms: start_time.elapsed().as_millis() as u64,
                        };

                        let json_str = serde_json::to_string_pretty(&map_response)
                            .unwrap_or_else(|e| format!(r#"{{"error": "Failed to serialize: {}"}}"#, e));
                        Ok(CallToolResult::success(vec![Content::text(json_str)]))
                    }
                    Err(e) => {
                        error!("Map website error: {}", e);
                        Ok(CallToolResult::success(vec![Content::text(format!("Website mapping failed: {}", e))]))
                    }
                }
            }
            _ => Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("Unknown tool: {}", request.name),
                None,
            )),
        }
    }
}
pub async fn run() -> anyhow::Result<()> {
    let service = McpService::new().await?;
    let server = service.serve(rmcp::transport::stdio()).await?;
    info!("MCP stdio server running");
    let _quit_reason = server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a test McpService without tracing init or external deps
    fn test_service() -> McpService {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let state = AppState::new("http://localhost:8888".to_string(), http_client);
        McpService {
            state: Arc::new(state),
        }
    }

    #[test]
    fn test_list_tools_contains_map_website() {
        let svc = test_service();
        let tools = svc.tool_definitions();
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            tool_names.contains(&"map_website"),
            "Expected 'map_website' in tool list, got: {:?}",
            tool_names
        );
    }

    #[test]
    fn test_map_website_schema_has_required_fields() {
        let svc = test_service();
        let tools = svc.tool_definitions();
        let map_tool = tools
            .iter()
            .find(|t| t.name == "map_website")
            .expect("map_website tool should exist");

        // Check that schema contains expected properties
        let schema_value = serde_json::Value::Object(map_tool.input_schema.as_ref().clone());
        let props = schema_value.get("properties").expect("should have properties");
        assert!(props.get("url").is_some(), "should have 'url' field");
        assert!(props.get("limit").is_some(), "should have 'limit' field");
        assert!(props.get("search").is_some(), "should have 'search' field");
        assert!(
            props.get("include_subdomains").is_some(),
            "should have 'include_subdomains' field"
        );
        assert!(
            props.get("sitemap_mode").is_some(),
            "should have 'sitemap_mode' field"
        );

        // Check 'url' is required
        let required = schema_value
            .get("required")
            .and_then(|v| v.as_array())
            .expect("should have required array");
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            required_strs.contains(&"url"),
            "url should be in required list"
        );
    }

    // --- Regression tests for map_website domain scoping (same_domain_only must always be true) ---

    #[test]
    fn test_map_crawl_config_always_same_domain_only_when_include_subdomains_true() {
        let config = McpService::build_map_crawl_config(100, true);
        assert!(
            config.same_domain_only,
            "same_domain_only must be true even when include_subdomains=true, \
             to prevent crawling external domains"
        );
    }

    #[test]
    fn test_map_crawl_config_always_same_domain_only_when_include_subdomains_false() {
        let config = McpService::build_map_crawl_config(100, false);
        assert!(
            config.same_domain_only,
            "same_domain_only must be true when include_subdomains=false, \
             to prevent crawling external domains"
        );
    }

    #[test]
    fn test_map_crawl_config_respects_limit() {
        let config = McpService::build_map_crawl_config(200, true);
        assert_eq!(config.max_pages, 200);

        // Verify the 5000 cap
        let config_big = McpService::build_map_crawl_config(10000, true);
        assert_eq!(config_big.max_pages, 5000);
    }

    #[test]
    fn test_map_crawl_config_discovery_only_char_limit() {
        let config = McpService::build_map_crawl_config(50, true);
        assert_eq!(
            config.max_chars_per_page, 100,
            "map_website should use minimal content extraction (discovery only)"
        );
    }
}