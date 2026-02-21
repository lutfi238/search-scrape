# Batch Status + Agent Status Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Menambahkan async batch operations dengan polling status - `scrape_batch_async`, `check_batch_status`, `deep_research_async`, `check_agent_status` - sesuai Firecrawl API.

**Architecture:** In-memory job stores (mirip CrawlJobStore) dengan TTL 24 jam. Dua stores: BatchJobStore untuk scrape dan ResearchJobStore untuk deep research.

**Tech Stack:** Rust (tokio async), uuid for job IDs, moka caching pattern.

---

## Task 1: Add Types to types.rs

**Files:**
- Modify: `mcp-server/src/types.rs`

**Step 1: Add JobStatus enum (reuse CrawlJobStatus)**

Tambahkan di line 307 (setelah CrawlJobStatus):
```rust
// Re-export JobStatus as alias for CrawlJobStatus
pub type JobStatus = CrawlJobStatus;
```

**Step 2: Add BatchJob types**

Tambahkan setelah ScrapeBatchResponse (sekitar line 165):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJobRequest {
    pub urls: Vec<String>,
    #[serde(default)]
    pub max_concurrent: Option<usize>,
    #[serde(default)]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJobResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub urls_total: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchProgress {
    pub urls_completed: usize,
    pub urls_failed: usize,
    pub progress_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStatusResponse {
    pub status: JobStatus,
    pub urls_total: usize,
    pub urls_completed: usize,
    pub urls_failed: usize,
    pub progress_percent: f32,
    #[serde(default)]
    pub results: Option<Vec<ScrapeBatchResult>>,
    #[serde(default)]
    pub error: Option<String>,
}
```

**Step 3: Add ResearchJob types**

Tambahkan setelah BatchStatusResponse:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchJobRequest {
    pub query: String,
    #[serde(default)]
    pub max_search_results: Option<usize>,
    #[serde(default)]
    pub crawl_depth: Option<usize>,
    #[serde(default)]
    pub max_pages_per_site: Option<usize>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub time_range: Option<String>,
    #[serde(default)]
    pub include_domains: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_domains: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchJobResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchProgress {
    pub current_phase: String,
    pub sources_processed: usize,
    pub total_sources: usize,
    pub progress_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchStatusResponse {
    pub status: JobStatus,
    pub query: String,
    pub current_phase: String,
    pub sources_processed: usize,
    pub total_sources: usize,
    pub progress_percent: f32,
    #[serde(default)]
    pub final_report: Option<ResearchReport>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchReport {
    pub query: String,
    pub summary: String,
    pub key_findings: Vec<String>,
    pub sources: Vec<ResearchSource>,
    pub statistics: ResearchStatistics,
}
```

**Step 4: Commit**

```bash
git add mcp-server/src/types.rs
git commit -m "feat(types): add batch and research job types"
```

---

## Task 2: Create batch_jobs.rs

**Files:**
- Create: `mcp-server/src/batch_jobs.rs`

**Step 1: Write the failing test**

```rust
// Di file baru batch_jobs.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::JobStatus;

    #[tokio::test]
    async fn test_create_batch_job_initial_state_is_queued() {
        let store = BatchJobStore::new(std::time::Duration::from_secs(60));
        let urls = vec!["https://example.com".to_string()];
        let job_id = store.create_job(urls.clone(), 10, 10000).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.urls.len(), 1);
    }

    #[tokio::test]
    async fn test_update_progress_tracks_completed_and_failed() {
        let store = BatchJobStore::new(std::time::Duration::from_secs(60));
        let urls = vec!["https://example.com".to_string(), "https://example.org".to_string()];
        let job_id = store.create_job(urls, 10, 10000).await;

        store.update_progress(&job_id, 1, 0, 1).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.progress.urls_completed, 1);
        assert_eq!(job.progress.urls_failed, 1);
    }
}
```

**Step 2: Write implementation**

```rust
// mcp-server/src/batch_jobs.rs
// Async batch scrape job store - in-memory job lifecycle management

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::types::{BatchProgress, CrawlJobStatus, JobStatus, ScrapeBatchResult};

pub type BatchJobStatus = JobStatus;

/// A single batch scrape job record.
#[derive(Debug, Clone)]
pub struct BatchJobRecord {
    pub job_id: String,
    pub urls: Vec<String>,
    pub max_concurrent: usize,
    pub max_chars: usize,
    pub status: BatchJobStatus,
    pub progress: BatchProgress,
    pub results: Option<Vec<ScrapeBatchResult>>,
    pub error: Option<String>,
    pub created_at: Instant,
    pub updated_at: Instant,
}

pub struct BatchJobStore {
    jobs: RwLock<HashMap<String, BatchJobRecord>>,
    ttl: Duration,
}

impl BatchJobStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    pub async fn create_job(
        &self,
        urls: Vec<String>,
        max_concurrent: usize,
        max_chars: usize,
    ) -> String {
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = Instant::now();
        let total = urls.len();
        let record = BatchJobRecord {
            job_id: job_id.clone(),
            urls,
            max_concurrent,
            max_chars,
            status: BatchJobStatus::Queued,
            progress: BatchProgress {
                urls_completed: 0,
                urls_failed: 0,
                progress_percent: 0.0,
            },
            results: None,
            error: None,
            created_at: now,
            updated_at: now,
        };
        let mut jobs = self.jobs.write().await;
        jobs.insert(job_id.clone(), record);
        job_id
    }

    pub async fn mark_running(&self, job_id: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = BatchJobStatus::Running;
            job.updated_at = Instant::now();
        }
    }

    pub async fn mark_completed(&self, job_id: &str, results: Vec<ScrapeBatchResult>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            let total = job.urls.len();
            let completed = results.iter().filter(|r| r.success).count();
            let failed = results.len().saturating_sub(completed);
            job.progress = BatchProgress {
                urls_completed: completed,
                urls_failed: failed,
                progress_percent: if total > 0 { (completed as f32 / total as f32) * 100.0 } else { 100.0 },
            };
            job.results = Some(results);
            job.status = BatchJobStatus::Completed;
            job.updated_at = Instant::now();
        }
    }

    pub async fn mark_failed(&self, job_id: &str, error: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = BatchJobStatus::Failed;
            job.error = Some(error);
            job.updated_at = Instant::now();
        }
    }

    pub async fn get_job(&self, job_id: &str) -> Option<BatchJobRecord> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    pub async fn update_progress(&self, job_id: &str, completed: usize, failed: usize, total: usize) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.progress = BatchProgress {
                urls_completed: completed,
                urls_failed: failed,
                progress_percent: if total > 0 { (completed as f32 / total as f32) * 100.0 } else { 0.0 },
            };
            job.updated_at = Instant::now();
        }
    }

    pub async fn expire_jobs(&self) -> usize {
        let mut jobs = self.jobs.write().await;
        let before = jobs.len();
        jobs.retain(|_, record| record.created_at.elapsed() < self.ttl);
        before - jobs.len()
    }
}
```

**Step 3: Run tests**

```bash
cargo test --manifest-path mcp-server/Cargo.toml batch_jobs
```

Expected: PASS

**Step 4: Commit**

```bash
git add mcp-server/src/batch_jobs.rs
git commit -m "feat(jobs): add BatchJobStore for async batch scrape"
```

---

## Task 3: Create research_jobs.rs

**Files:**
- Create: `mcp-server/src/research_jobs.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_research_job_initial_state_is_queued() {
        let store = ResearchJobStore::new(std::time::Duration::from_secs(60));
        let job_id = store.create_job("rust async".to_string(), None).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, crate::types::JobStatus::Queued);
        assert_eq!(job.query, "rust async");
    }
}
```

**Step 2: Write implementation**

```rust
// mcp-server/src/research_jobs.rs
// Async research job store - in-memory job lifecycle management

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::types::{JobStatus, ResearchProgress, ResearchReport, ResearchSource, ResearchStatistics};

pub type ResearchJobStatus = JobStatus;

#[derive(Debug, Clone)]
pub struct ResearchConfig {
    pub max_search_results: Option<usize>,
    pub crawl_depth: Option<usize>,
    pub max_pages_per_site: Option<usize>,
    pub language: Option<String>,
    pub time_range: Option<String>,
    pub include_domains: Option<Vec<String>>,
    pub exclude_domains: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ResearchJobRecord {
    pub job_id: String,
    pub query: String,
    pub config: ResearchConfig,
    pub status: ResearchJobStatus,
    pub progress: ResearchProgress,
    pub final_report: Option<ResearchReport>,
    pub error: Option<String>,
    pub created_at: Instant,
    pub updated_at: Instant,
}

pub struct ResearchJobStore {
    jobs: RwLock<HashMap<String, ResearchJobRecord>>,
    ttl: Duration,
}

impl ResearchJobStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    pub async fn create_job(&self, query: String, config: ResearchConfig) -> String {
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = Instant::now();
        let record = ResearchJobRecord {
            job_id: job_id.clone(),
            query,
            config,
            status: ResearchJobStatus::Queued,
            progress: ResearchProgress {
                current_phase: "queued".to_string(),
                sources_processed: 0,
                total_sources: 0,
                progress_percent: 0.0,
            },
            final_report: None,
            error: None,
            created_at: now,
            updated_at: now,
        };
        let mut jobs = self.jobs.write().await;
        jobs.insert(job_id.clone(), record);
        job_id
    }

    pub async fn mark_running(&self, job_id: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = ResearchJobStatus::Running;
            job.updated_at = Instant::now();
        }
    }

    pub async fn mark_completed(&self, job_id: &str, report: ResearchReport) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.final_report = Some(report);
            job.status = ResearchJobStatus::Completed;
            job.progress.progress_percent = 100.0;
            job.updated_at = Instant::now();
        }
    }

    pub async fn mark_failed(&self, job_id: &str, error: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = ResearchJobStatus::Failed;
            job.error = Some(error);
            job.updated_at = Instant::now();
        }
    }

    pub async fn get_job(&self, job_id: &str) -> Option<ResearchJobRecord> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    pub async fn update_progress(&self, job_id: &str, phase: String, processed: usize, total: usize) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.progress = ResearchProgress {
                current_phase: phase,
                sources_processed: processed,
                total_sources: total,
                progress_percent: if total > 0 { (processed as f32 / total as f32) * 100.0 } else { 0.0 },
            };
            job.updated_at = Instant::now();
        }
    }

    pub async fn expire_jobs(&self) -> usize {
        let mut jobs = self.jobs.write().await;
        let before = jobs.len();
        jobs.retain(|_, record| record.created_at.elapsed() < self.ttl);
        before - jobs.len()
    }
}
```

**Step 3: Run tests**

```bash
cargo test --manifest-path mcp-server/Cargo.toml research_jobs
```

Expected: PASS

**Step 4: Commit**

```bash
git add mcp-server/src/research_jobs.rs
git commit -m "feat(jobs): add ResearchJobStore for async deep research"
```

---

## Task 4: Register Job Stores in AppState

**Files:**
- Modify: `mcp-server/src/lib.rs`

**Step 1: Add imports**

Tambahkan setelah line 11:
```rust
pub mod batch_jobs;
pub mod research_jobs;
```

**Step 2: Add to AppState**

Di struct AppState (line 16-30), tambahkan:
```rust
// Async batch scrape job store
pub batch_jobs: std::sync::Arc<batch_jobs::BatchJobStore>,
// Async research job store
pub research_jobs: std::sync::Arc<research_jobs::ResearchJobStore>,
```

**Step 3: Initialize in new()**

Di AppState::new (sekitar line 46), tambahkan:
```rust
// 24 hour TTL for async jobs
let job_ttl = std::time::Duration::from_secs(24 * 60 * 60);

Self {
    // ... existing fields ...
    batch_jobs: std::sync::Arc::new(batch_jobs::BatchJobStore::new(job_ttl)),
    research_jobs: std::sync::Arc::new(research_jobs::ResearchJobStore::new(job_ttl)),
}
```

**Step 4: Commit**

```bash
git add mcp-server/src/lib.rs
git commit -m "feat(state): register batch and research job stores in AppState"
```

---

## Task 5: Add Async Batch Scrape Logic

**Files:**
- Modify: `mcp-server/src/scrape.rs`

**Step 1: Add async batch function**

Tambahkan di akhir file (setelah scrape_batch):
```rust
use crate::batch_jobs::BatchJobStore;

/// Start an async batch scrape job, returns job_id
pub async fn scrape_batch_async(
    state: &Arc<AppState>,
    urls: Vec<String>,
    max_concurrent: Option<usize>,
    max_chars: Option<usize>,
) -> Result<types::BatchJobResponse> {
    let urls = urls.into_iter().take(100).collect::<Vec<_>>();
    let total = urls.len();

    if total == 0 {
        return Err(Error::InvalidInput("urls cannot be empty".to_string()));
    }

    let concurrency = max_concurrent.unwrap_or(10).min(50);
    let max_chars = max_chars.unwrap_or(10000);

    // Create job
    let job_id = state.batch_jobs
        .create_job(urls.clone(), concurrency, max_chars)
        .await;

    // Spawn async task to process
    let state_clone = Arc::clone(state);
    let urls_clone = urls.clone();
    tokio::spawn(async move {
        let store = &state_clone.batch_jobs;

        store.mark_running(&job_id).await;

        // Process URLs
        let results = process_batch_urls(&state_clone, urls_clone, concurrency, max_chars).await;

        match results {
            Ok(r) => {
                store.mark_completed(&job_id, r).await;
            }
            Err(e) => {
                store.mark_failed(&job_id, e.to_string()).await;
            }
        }
    });

    Ok(types::BatchJobResponse {
        job_id,
        status: types::JobStatus::Queued,
        urls_total: total,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Check batch job status
pub async fn check_batch_status(
    state: &Arc<AppState>,
    job_id: &str,
    include_results: Option<bool>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<types::BatchStatusResponse> {
    let job = state.batch_jobs.get_job(job_id).await
        .ok_or_else(|| Error::NotFound(format!("job {} not found", job_id)))?;

    let include_results = include_results.unwrap_or(false);
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(50);

    let results = if include_results && job.results.is_some() {
        let all_results = job.results.unwrap();
        let slice: Vec<_> = all_results.into_iter().skip(offset).take(limit).collect();
        Some(slice)
    } else {
        None
    };

    Ok(types::BatchStatusResponse {
        status: job.status,
        urls_total: job.urls.len(),
        urls_completed: job.progress.urls_completed,
        urls_failed: job.progress.urls_failed,
        progress_percent: job.progress.progress_percent,
        results,
        error: job.error,
    })
}

/// Internal: process URLs for batch scrape
async fn process_batch_urls(
    state: &Arc<AppState>,
    urls: Vec<String>,
    concurrency: usize,
    max_chars: usize,
) -> Result<Vec<types::ScrapeBatchResult>> {
    use futures::stream::StreamExt;
    use tokio::sync::Semaphore;

    let semaphore = Arc::new(Semaphore::new(concurrency));
    let state_clone = Arc::clone(state);

    let results: Vec<types::ScrapeBatchResult> = stream::iter(urls)
        .map(|url| {
            let state = Arc::clone(&state_clone);
            let sem = Arc::clone(&semaphore);
            async move {
                let _permit = sem.acquire().await.unwrap();

                match scrape_url(&state, &url).await {
                    Ok(data) => {
                        let actual = data.clean_content.len();
                        types::ScrapeBatchResult {
                            url: url.clone(),
                            success: true,
                            title: data.title,
                            content: if max_chars > 0 && actual > max_chars {
                                data.clean_content[..max_chars].to_string()
                            } else {
                                data.clean_content
                            },
                            actual_chars: actual,
                            extraction_score: data.extraction_score,
                            warnings: data.warnings,
                            content_links: data.content_links,
                            code_blocks: data.code_blocks,
                        }
                    }
                    Err(e) => types::ScrapeBatchResult {
                        url,
                        success: false,
                        title: None,
                        content: String::new(),
                        actual_chars: 0,
                        extraction_score: 0.0,
                        warnings: vec![],
                        content_links: vec![],
                        code_blocks: vec![],
                    },
                }
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    Ok(results)
}
```

**Step 2: Commit**

```bash
git add mcp-server/src/scrape.rs
git commit -m "feat(scrape): add async batch scrape with status checking"
```

---

## Task 6: Add Async Deep Research Logic

**Files:**
- Modify: `mcp-server/src/research.rs`

**Step 1: Add async research function**

Tambahkan di akhir file:
```rust
use crate::research_jobs::{ResearchConfig, ResearchJobStore};

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
        language: config.language,
        time_range: config.time_range,
        include_domains: config.include_domains,
        exclude_domains: config.exclude_domains,
    };

    // Create job
    let job_id = state.research_jobs
        .create_job(query.clone(), research_config)
        .await;

    // Spawn async task to process
    let state_clone = Arc::clone(state);
    let query_clone = query.clone();
    tokio::spawn(async move {
        let store = &state_clone.research_jobs;

        store.mark_running(&job_id).await;

        match deep_research(&state_clone, query_clone, config.clone()).await {
            Ok(report) => {
                store.mark_completed(&job_id, report).await;
            }
            Err(e) => {
                store.mark_failed(&job_id, e.to_string()).await;
            }
        }
    });

    Ok(crate::types::ResearchJobResponse {
        job_id,
        status: crate::types::JobStatus::Queued,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Check research job status
pub async fn check_agent_status(
    state: &Arc<AppState>,
    job_id: &str,
    include_results: Option<bool>,
) -> Result<crate::types::ResearchStatusResponse> {
    let job = state.research_jobs.get_job(job_id).await
        .ok_or_else(|| crate::scrape::Error::NotFound(format!("job {} not found", job_id)))?;

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
```

**Step 2: Commit**

```bash
git add mcp-server/src/research.rs
git commit -m "feat(research): add async deep research with status checking"
```

---

## Task 7: Register New MCP Tools

**Files:**
- Modify: `mcp-server/src/stdio_service.rs`

**Step 1: Add tool definitions**

Cari section tool definitions (sekitar line 270-300), tambahkan:
```rust
// scrape_batch_async
ToolDefinition {
    name: Cow::Borrowed("scrape_batch_async"),
    description: Cow::Borrowed("Start an async batch scrape job. Returns a job_id to poll status."),
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
ToolDefinition {
    name: Cow::Borrowed("check_batch_status"),
    description: Cow::Borrowed("Check status of an async batch scrape job."),
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

// deep_research_async
ToolDefinition {
    name: Cow::Borrowed("deep_research_async"),
    description: Cow::Borrowed("Start an async deep research job. Returns a job_id to poll status."),
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
ToolDefinition {
    name: Cow::Borrowed("check_agent_status"),
    description: Cow::Borrowed("Check status of an async deep research job."),
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
```

**Step 2: Add handlers**

Cari section tool handlers (sekitar line 1340+), tambahkan:
```rust
"scrape_batch_async" => {
    let urls: Vec<String> = get_required("urls")?;
    let max_concurrent = get_optional("max_concurrent");
    let max_chars = get_optional("max_chars");

    match scrape::scrape_batch_async(&self.state, urls, max_concurrent, max_chars).await {
        Ok(resp) => Ok(serde_json::to_value(resp)?),
        Err(e) => Err(standard_error("SCRAPE_ERROR", &e.to_string())),
    }
}

"check_batch_status" => {
    let job_id: String = get_required("job_id")?;
    let include_results = get_optional("include_results");
    let offset = get_optional("offset");
    let limit = get_optional("limit");

    match scrape::check_batch_status(&self.state, &job_id, include_results, offset, limit).await {
        Ok(resp) => Ok(serde_json::to_value(resp)?),
        Err(e) => Err(standard_error("BATCH_STATUS_ERROR", &e.to_string())),
    }
}

"deep_research_async" => {
    let query: String = get_required("query")?;
    let config = crate::types::ResearchJobRequest {
        query: query.clone(),
        max_search_results: get_optional("max_search_results"),
        crawl_depth: get_optional("crawl_depth"),
        max_pages_per_site: get_optional("max_pages_per_site"),
        language: get_optional("language"),
        time_range: get_optional("time_range"),
        include_domains: get_optional("include_domains"),
        exclude_domains: get_optional("exclude_domains"),
    };

    match research::deep_research_async(&self.state, query, config).await {
        Ok(resp) => Ok(serde_json::to_value(resp)?),
        Err(e) => Err(standard_error("RESEARCH_ERROR", &e.to_string())),
    }
}

"check_agent_status" => {
    let job_id: String = get_required("job_id")?;
    let include_results = get_optional("include_results");

    match research::check_agent_status(&self.state, &job_id, include_results).await {
        Ok(resp) => Ok(serde_json::to_value(resp)?),
        Err(e) => Err(standard_error("AGENT_STATUS_ERROR", &e.to_string())),
    }
}
```

**Step 3: Commit**

```bash
git add mcp-server/src/stdio_service.rs
git commit -m "feat(mcp): add async batch and agent status tools"
```

---

## Task 8: Build and Test

**Step 1: Build**

```bash
cargo build --release --manifest-path mcp-server/Cargo.toml
```

Fix any compilation errors.

**Step 2: Run tests**

```bash
cargo test --manifest-path mcp-server/Cargo.toml
```

**Step 3: Integration test**

Start server dan test manually:
```bash
SEARXNG_URL=http://localhost:8888 cargo run --release --manifest-path mcp-server/Cargo.toml --bin mcp-server
```

Test curl:
```bash
# Start async batch
curl -X POST "http://localhost:5000/mcp/call" \
  -H "Content-Type: application/json" \
  -d '{"name": "scrape_batch_async", "arguments": {"urls": ["https://example.com", "https://example.org"]}}'

# Check status (replace job_id)
curl -X POST "http://localhost:5000/mcp/call" \
  -H "Content-Type: application/json" \
  -d '{"name": "check_batch_status", "arguments": {"job_id": "<job_id>"}}'
```

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: add batch status and agent status tools - complete"
```

---

## Plan Complete

Plan saved to: `docs/plans/2026-02-22-batch-agent-status-design.md`

**Two execution options:**

1. **Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

2. **Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

Which approach?
