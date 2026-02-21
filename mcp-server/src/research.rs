use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

use crate::crawl::{crawl_website, CrawlConfig};
use crate::research_jobs::ResearchConfig;
use crate::scrape::scrape_url;
use crate::search::{search_web_with_params, SearchParamOverrides};
use crate::types::*;
use crate::AppState;
use chrono::Utc;

/// Configuration for deep research
#[derive(Clone)]
pub struct DeepResearchConfig {
    pub max_search_results: usize,
    pub max_pages_per_site: usize,
    pub max_total_pages: usize,
    pub crawl_depth: usize,
    pub max_concurrent: usize,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
    pub search_engines: Option<String>,
    pub time_range: Option<String>,
    pub language: Option<String>,
    pub max_chars_per_page: usize,
}

impl Default for DeepResearchConfig {
    fn default() -> Self {
        Self {
            max_search_results: 10,
            max_pages_per_site: 5,
            max_total_pages: 30,
            crawl_depth: 2,
            max_concurrent: 5,
            include_domains: vec![],
            exclude_domains: vec![],
            search_engines: None,
            time_range: None,
            language: None,
            max_chars_per_page: 5000,
        }
    }
}

/// Response from deep research
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeepResearchResponse {
    pub query: String,
    pub summary: ResearchSummary,
    pub sources: Vec<ResearchSource>,
    pub topics: Vec<TopicCluster>,
    pub key_findings: Vec<String>,
    pub related_queries: Vec<String>,
    pub statistics: ResearchStatistics,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResearchSummary {
    pub overview: String,
    pub key_points: Vec<String>,
    pub domains_covered: Vec<String>,
    pub content_types: HashMap<String, usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResearchSource {
    pub url: String,
    pub title: String,
    pub domain: String,
    pub relevance_score: f64,
    pub content_preview: String,
    pub word_count: usize,
    pub source_type: String,
    pub headings: Vec<String>,
    pub code_blocks_count: usize,
    pub from_crawl: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TopicCluster {
    pub topic: String,
    pub mentions: usize,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResearchStatistics {
    pub search_results_found: usize,
    pub pages_scraped: usize,
    pub pages_crawled: usize,
    pub total_words: usize,
    pub unique_domains: usize,
    pub code_blocks_found: usize,
    pub duration_ms: u64,
    pub search_time_ms: u64,
    pub scrape_time_ms: u64,
    pub analysis_time_ms: u64,
}

/// Perform deep research on a topic
/// 1. Search the web for relevant pages
/// 2. Scrape and optionally crawl top results
/// 3. Analyze and summarize findings
pub async fn deep_research(
    state: &Arc<AppState>,
    query: &str,
    config: DeepResearchConfig,
) -> Result<DeepResearchResponse> {
    let start_time = Instant::now();
    let mut warnings = Vec::new();

    info!("Starting deep research for: {}", query);

    // Phase 1: Search
    let search_start = Instant::now();
    let overrides = SearchParamOverrides {
        engines: config.search_engines.clone(),
        categories: None,
        language: config.language.clone(),
        safesearch: None,
        time_range: config.time_range.clone(),
        pageno: Some(1),
    };

    let (search_results, _extras) = search_web_with_params(state, query, Some(overrides)).await?;

    // Take only the number of results we need
    let search_results: Vec<SearchResult> = search_results
        .into_iter()
        .take(config.max_search_results)
        .collect();

    let search_time_ms = search_start.elapsed().as_millis() as u64;
    info!(
        "Search returned {} results in {}ms",
        search_results.len(),
        search_time_ms
    );

    if search_results.is_empty() {
        return Err(anyhow!("No search results found for query: {}", query));
    }

    // Filter by domain preferences
    let filtered_results: Vec<_> = search_results
        .into_iter()
        .filter(|r| {
            let domain = extract_domain(&r.url);

            // Check exclude list
            if config.exclude_domains.iter().any(|d| domain.contains(d)) {
                return false;
            }

            // Check include list (if not empty, must match)
            if !config.include_domains.is_empty() {
                return config.include_domains.iter().any(|d| domain.contains(d));
            }

            true
        })
        .take(config.max_search_results)
        .collect();

    if filtered_results.is_empty() {
        warnings.push("All search results filtered by domain preferences".to_string());
        return Err(anyhow!("No results after domain filtering"));
    }

    // Phase 2: Scrape and crawl
    let scrape_start = Instant::now();
    let mut all_sources: Vec<ResearchSource> = Vec::new();
    let mut scraped_urls: HashSet<String> = HashSet::new();
    let mut total_pages_scraped = 0;
    let mut total_pages_crawled = 0;

    for result in &filtered_results {
        if total_pages_scraped >= config.max_total_pages {
            break;
        }

        let domain = extract_domain(&result.url);

        // Scrape the main page
        match scrape_url(state, &result.url).await {
            Ok(data) => {
                scraped_urls.insert(result.url.clone());
                total_pages_scraped += 1;

                let source = create_research_source(&result.url, &data, false);
                all_sources.push(source);

                // Optionally crawl for more pages from this domain
                if config.crawl_depth > 0 && config.max_pages_per_site > 1 {
                    let pages_from_domain =
                        all_sources.iter().filter(|s| s.domain == domain).count();

                    if pages_from_domain < config.max_pages_per_site {
                        let crawl_config = CrawlConfig {
                            max_depth: config.crawl_depth,
                            max_pages: config.max_pages_per_site - pages_from_domain,
                            max_concurrent: config.max_concurrent,
                            same_domain_only: true,
                            include_patterns: vec![],
                            exclude_patterns: default_exclude_patterns(),
                            max_chars_per_page: config.max_chars_per_page,
                        };

                        match crawl_website(state, &result.url, crawl_config).await {
                            Ok(crawl_result) => {
                                for page in crawl_result.results {
                                    if page.success && !scraped_urls.contains(&page.url) {
                                        if total_pages_scraped >= config.max_total_pages {
                                            break;
                                        }

                                        // Re-scrape for full content
                                        if let Ok(page_data) = scrape_url(state, &page.url).await {
                                            scraped_urls.insert(page.url.clone());
                                            total_pages_scraped += 1;
                                            total_pages_crawled += 1;

                                            let source =
                                                create_research_source(&page.url, &page_data, true);
                                            all_sources.push(source);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Crawl failed for {}: {}", result.url, e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Failed to scrape {}: {}", result.url, e);
                warnings.push(format!("Failed to scrape: {}", result.url));
            }
        }
    }

    let scrape_time_ms = scrape_start.elapsed().as_millis() as u64;
    info!(
        "Scraped {} pages, crawled {} additional in {}ms",
        total_pages_scraped - total_pages_crawled,
        total_pages_crawled,
        scrape_time_ms
    );

    if all_sources.is_empty() {
        return Err(anyhow!("Failed to scrape any pages"));
    }

    // Phase 3: Analyze and summarize
    let analysis_start = Instant::now();

    // Calculate statistics
    let total_words: usize = all_sources.iter().map(|s| s.word_count).sum();
    let unique_domains: HashSet<_> = all_sources.iter().map(|s| s.domain.clone()).collect();
    let code_blocks_found: usize = all_sources.iter().map(|s| s.code_blocks_count).sum();

    // Extract topics from headings and content
    let topics = extract_topics(&all_sources, query);

    // Generate key findings
    let key_findings = generate_key_findings(&all_sources, query);

    // Generate related queries
    let related_queries = generate_related_queries(&all_sources, query);

    // Create summary
    let summary = create_summary(&all_sources, query);

    // Sort sources by relevance
    let mut sources = all_sources;
    sources.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let analysis_time_ms = analysis_start.elapsed().as_millis() as u64;

    let statistics = ResearchStatistics {
        search_results_found: filtered_results.len(),
        pages_scraped: total_pages_scraped,
        pages_crawled: total_pages_crawled,
        total_words,
        unique_domains: unique_domains.len(),
        code_blocks_found,
        duration_ms: start_time.elapsed().as_millis() as u64,
        search_time_ms,
        scrape_time_ms,
        analysis_time_ms,
    };

    info!(
        "Deep research completed: {} sources, {} words, {} domains, {}ms total",
        sources.len(),
        total_words,
        unique_domains.len(),
        statistics.duration_ms
    );

    Ok(DeepResearchResponse {
        query: query.to_string(),
        summary,
        sources,
        topics,
        key_findings,
        related_queries,
        statistics,
        warnings,
    })
}

/// Extract domain from URL
fn extract_domain(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

/// Create a research source from scrape data
fn create_research_source(url: &str, data: &ScrapeResponse, from_crawl: bool) -> ResearchSource {
    let domain = extract_domain(url);

    // Determine source type
    let source_type = determine_source_type(&domain, &data.title, &data.clean_content);

    // Calculate relevance score
    let relevance_score = calculate_relevance_score(data);

    // Get top headings
    let headings: Vec<String> = data
        .headings
        .iter()
        .filter(|h| h.level == "h1" || h.level == "h2")
        .take(5)
        .map(|h| h.text.clone())
        .collect();

    // Content preview
    let content_preview: String = data.clean_content.chars().take(500).collect();

    ResearchSource {
        url: url.to_string(),
        title: data.title.clone(),
        domain,
        relevance_score,
        content_preview,
        word_count: data.word_count,
        source_type,
        headings,
        code_blocks_count: data.code_blocks.len(),
        from_crawl,
    }
}

/// Determine source type based on domain and content
fn determine_source_type(domain: &str, title: &str, content: &str) -> String {
    let domain_lower = domain.to_lowercase();
    let title_lower = title.to_lowercase();

    // Documentation sites
    if domain_lower.contains("docs.")
        || domain_lower.contains("documentation")
        || domain_lower.contains("readthedocs")
        || domain_lower.contains("devdocs")
        || title_lower.contains("documentation")
        || title_lower.contains("api reference")
    {
        return "documentation".to_string();
    }

    // Code repositories
    if domain_lower.contains("github.com")
        || domain_lower.contains("gitlab.com")
        || domain_lower.contains("bitbucket.org")
    {
        return "repository".to_string();
    }

    // Stack Overflow and Q&A
    if domain_lower.contains("stackoverflow.com")
        || domain_lower.contains("stackexchange.com")
        || domain_lower.contains("quora.com")
    {
        return "qa".to_string();
    }

    // Blog/Tutorial
    if domain_lower.contains("blog.")
        || domain_lower.contains("medium.com")
        || domain_lower.contains("dev.to")
        || domain_lower.contains("hashnode")
        || title_lower.contains("tutorial")
        || title_lower.contains("how to")
        || title_lower.contains("guide")
    {
        return "blog".to_string();
    }

    // News
    if domain_lower.contains("news.")
        || domain_lower.contains("techcrunch")
        || domain_lower.contains("theverge")
        || domain_lower.contains("wired")
    {
        return "news".to_string();
    }

    // Check content for indicators
    if content.contains("```")
        || content.matches("def ").count() > 2
        || content.matches("function ").count() > 2
    {
        return "technical".to_string();
    }

    "article".to_string()
}

/// Calculate relevance score based on content quality
fn calculate_relevance_score(data: &ScrapeResponse) -> f64 {
    let mut score: f64 = 0.5;

    // Word count factor
    score += match data.word_count {
        0..=100 => 0.0,
        101..=300 => 0.1,
        301..=1000 => 0.2,
        _ => 0.25,
    };

    // Has meta description
    if !data.meta_description.is_empty() {
        score += 0.05;
    }

    // Has author
    if data.author.is_some() {
        score += 0.05;
    }

    // Has published date
    if data.published_at.is_some() {
        score += 0.05;
    }

    // Has headings (structured content)
    score += match data.headings.len() {
        0 => 0.0,
        1..=3 => 0.05,
        4..=10 => 0.1,
        _ => 0.1,
    };

    // Has code blocks (technical content)
    if !data.code_blocks.is_empty() {
        score += 0.1;
    }

    score.min(1.0)
}

/// Extract topics from sources
fn extract_topics(sources: &[ResearchSource], query: &str) -> Vec<TopicCluster> {
    let mut topic_counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();
    let query_lower = query.to_lowercase();
    let query_words: HashSet<&str> = query_lower.split_whitespace().collect();

    for source in sources {
        // Extract topics from headings
        for heading in &source.headings {
            let heading_lower = heading.to_lowercase();
            let words: Vec<_> = heading_lower.split_whitespace().collect();

            // Skip if it's just the query
            if words.iter().all(|w| query_words.contains(w)) {
                continue;
            }

            // Use 2-3 word phrases as topics
            for window_size in [2, 3] {
                for window in words.windows(window_size) {
                    let phrase = window.join(" ");
                    if phrase.len() > 5 && !query_words.contains(phrase.as_str()) {
                        let entry = topic_counts
                            .entry(phrase.clone())
                            .or_insert((0, Vec::new()));
                        entry.0 += 1;
                        if !entry.1.contains(&source.url) {
                            entry.1.push(source.url.clone());
                        }
                    }
                }
            }
        }
    }

    // Sort by mention count and take top topics
    let mut topics: Vec<_> = topic_counts
        .into_iter()
        .filter(|(_, (count, _))| *count >= 2)
        .map(|(topic, (mentions, sources))| TopicCluster {
            topic,
            mentions,
            sources,
        })
        .collect();

    topics.sort_by(|a, b| b.mentions.cmp(&a.mentions));
    topics.truncate(10);

    topics
}

/// Generate key findings from sources
fn generate_key_findings(sources: &[ResearchSource], _query: &str) -> Vec<String> {
    let mut findings = Vec::new();

    // Count source types
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for source in sources {
        *type_counts.entry(source.source_type.clone()).or_insert(0) += 1;
    }

    // Find dominant source type
    if let Some((top_type, count)) = type_counts.iter().max_by_key(|(_, v)| *v) {
        findings.push(format!(
            "Primary source type: {} ({} of {} sources)",
            top_type,
            count,
            sources.len()
        ));
    }

    // Check for code examples
    let code_sources: Vec<_> = sources.iter().filter(|s| s.code_blocks_count > 0).collect();
    if !code_sources.is_empty() {
        findings.push(format!(
            "Found {} sources with code examples ({} total code blocks)",
            code_sources.len(),
            code_sources
                .iter()
                .map(|s| s.code_blocks_count)
                .sum::<usize>()
        ));
    }

    // Check for documentation
    let doc_sources: Vec<_> = sources
        .iter()
        .filter(|s| s.source_type == "documentation")
        .collect();
    if !doc_sources.is_empty() {
        findings.push(format!(
            "Found {} official documentation sources",
            doc_sources.len()
        ));
    }

    // Find most comprehensive sources
    let mut by_words: Vec<_> = sources.iter().collect();
    by_words.sort_by(|a, b| b.word_count.cmp(&a.word_count));
    if let Some(top) = by_words.first() {
        findings.push(format!(
            "Most comprehensive source: {} ({} words)",
            top.title.chars().take(50).collect::<String>(),
            top.word_count
        ));
    }

    findings
}

/// Generate related queries from sources
fn generate_related_queries(sources: &[ResearchSource], query: &str) -> Vec<String> {
    let mut related = Vec::new();
    let query_lower = query.to_lowercase();

    // Extract potential related queries from headings
    let mut heading_queries: HashMap<String, usize> = HashMap::new();

    for source in sources {
        for heading in &source.headings {
            let heading_lower = heading.to_lowercase();

            // Skip if too similar to original query
            if heading_lower.contains(&query_lower) || query_lower.contains(&heading_lower) {
                continue;
            }

            // Skip very short or very long headings
            if heading.len() < 10 || heading.len() > 60 {
                continue;
            }

            *heading_queries.entry(heading.clone()).or_insert(0) += 1;
        }
    }

    // Get most common headings as related queries
    let mut sorted: Vec<_> = heading_queries.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    for (heading, _) in sorted.into_iter().take(5) {
        related.push(heading);
    }

    related
}

/// Create research summary
fn create_summary(sources: &[ResearchSource], query: &str) -> ResearchSummary {
    // Collect domains
    let domains: Vec<_> = sources
        .iter()
        .map(|s| s.domain.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(10)
        .collect();

    // Count content types
    let mut content_types: HashMap<String, usize> = HashMap::new();
    for source in sources {
        *content_types.entry(source.source_type.clone()).or_insert(0) += 1;
    }

    // Generate key points from high-relevance sources
    let mut key_points = Vec::new();
    let top_sources: Vec<_> = sources
        .iter()
        .filter(|s| s.relevance_score >= 0.6)
        .take(5)
        .collect();

    for source in &top_sources {
        if !source.headings.is_empty() {
            key_points.push(format!(
                "{}: {}",
                source.domain,
                source.headings.first().unwrap_or(&source.title.clone())
            ));
        }
    }

    // Generate overview
    let total_sources = sources.len();
    let total_words: usize = sources.iter().map(|s| s.word_count).sum();
    let doc_count = content_types.get("documentation").unwrap_or(&0);
    let code_count = sources.iter().filter(|s| s.code_blocks_count > 0).count();

    let overview = format!(
        "Research on \"{}\" found {} sources across {} domains, containing {} total words. \
        {} sources include official documentation and {} contain code examples.",
        query,
        total_sources,
        domains.len(),
        total_words,
        doc_count,
        code_count
    );

    ResearchSummary {
        overview,
        key_points,
        domains_covered: domains,
        content_types,
    }
}

/// Default patterns to exclude when crawling
fn default_exclude_patterns() -> Vec<String> {
    vec![
        "/login".to_string(),
        "/logout".to_string(),
        "/signup".to_string(),
        "/register".to_string(),
        "/account".to_string(),
        "/cart".to_string(),
        "/checkout".to_string(),
        "/admin".to_string(),
        "/api/".to_string(),
        "/search".to_string(),
        ".pdf".to_string(),
        ".zip".to_string(),
        ".exe".to_string(),
    ]
}

/// Start an async deep research job, returns job_id
pub async fn deep_research_async(
    state: &Arc<AppState>,
    query: String,
    config: crate::types::ResearchJobRequest,
) -> Result<crate::types::ResearchJobResponse> {
    let research_config = ResearchConfig {
        max_search_results: config.max_search_results,
        crawl_depth: config.crawl_depth,
        max_pages_per_site: config.max_pages_per_site,
        language: config.language.clone(),
        time_range: config.time_range.clone(),
        include_domains: config.include_domains.clone(),
        exclude_domains: config.exclude_domains.clone(),
    };

    // Create job
    let job_id = state
        .research_jobs
        .create_job(query.clone(), Some(research_config))
        .await;

    // Spawn async task to process
    let state_clone = Arc::clone(state);
    let query_clone = query.clone();
    let config_clone = config.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        let store = &state_clone.research_jobs;

        store.mark_running(&job_id_clone).await;

        match build_research_config(config_clone) {
            Ok(deep_config) => match deep_research(&state_clone, &query_clone, deep_config).await {
                Ok(report) => {
                    let final_report = ResearchReport {
                        query: report.query,
                        summary: report.summary.overview,
                        key_findings: report.key_findings,
                        sources: report.sources,
                        statistics: report.statistics,
                    };
                    store.mark_completed(&job_id_clone, final_report).await;
                }
                Err(e) => {
                    store.mark_failed(&job_id_clone, e.to_string()).await;
                }
            },
            Err(e) => {
                store.mark_failed(&job_id_clone, e.to_string()).await;
            }
        }
    });

    Ok(crate::types::ResearchJobResponse {
        job_id,
        status: crate::types::JobStatus::Queued,
        created_at: Utc::now().to_rfc3339(),
    })
}

/// Check research job status
pub async fn check_agent_status(
    state: &Arc<AppState>,
    job_id: &str,
    include_results: Option<bool>,
) -> Result<crate::types::ResearchStatusResponse> {
    let job = state
        .research_jobs
        .get_job(job_id)
        .await
        .ok_or_else(|| anyhow!("job {} not found", job_id))?;

    let include_results = include_results.unwrap_or(false);

    let final_report = if include_results && job.final_report.is_some() {
        job.final_report
    } else {
        None
    };

    Ok(crate::types::ResearchStatusResponse {
        status: job.status,
        query: job.query,
        current_phase: job.progress.current_phase,
        sources_processed: job.progress.sources_processed,
        total_sources: job.progress.total_sources,
        progress_percent: job.progress.progress_percent,
        final_report,
        error: job.error,
    })
}

fn build_research_config(config: ResearchJobRequest) -> Result<DeepResearchConfig> {
    let max_search_results = config.max_search_results.unwrap_or(10).min(30);
    let max_pages_per_site = config.max_pages_per_site.unwrap_or(5).min(20);
    let crawl_depth = config.crawl_depth.unwrap_or(2).min(3);
    let max_total_pages = (max_search_results * max_pages_per_site).min(100);

    let include_domains = config.include_domains.unwrap_or_default();
    let exclude_domains = config.exclude_domains.unwrap_or_default();

    Ok(DeepResearchConfig {
        max_search_results,
        max_pages_per_site,
        max_total_pages,
        crawl_depth,
        max_concurrent: 5,
        include_domains,
        exclude_domains,
        search_engines: None,
        time_range: config.time_range,
        language: config.language,
        max_chars_per_page: 5000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://docs.rust-lang.org/book/"),
            "docs.rust-lang.org"
        );
        assert_eq!(extract_domain("https://github.com/user/repo"), "github.com");
    }

    #[test]
    fn test_determine_source_type() {
        assert_eq!(
            determine_source_type("docs.python.org", "Python Documentation", ""),
            "documentation"
        );
        assert_eq!(
            determine_source_type("github.com", "repo", ""),
            "repository"
        );
        assert_eq!(
            determine_source_type("stackoverflow.com", "question", ""),
            "qa"
        );
        assert_eq!(determine_source_type("medium.com", "article", ""), "blog");
    }
}
