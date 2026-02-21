# Design: Batch Status + Agent Status

## Overview

Menambahkan tools untuk async batch operations yang konsisten dengan Firecrawl:
- `check_batch_status` - cek status async batch scrape
- `check_agent_status` - cek status async deep research

Serta mengkonversi operations menjadi async:
- `scrape_batch_async` - async version of batch scrape
- `deep_research_async` - async version of deep research

## Goals

1.long-running batch scrape bisa di-poll statusnya
2. Long-running research bisa di-poll statusnya
3. Konsisten dengan Firecrawl API
4. Backward compatible - existing sync tools tetap work

## Architecture

### Job Stores (In-Memory)

```rust
// BatchJobStore - untuk scrape_batch_async
struct BatchJobRecord {
    job_id: String,
    urls: Vec<String>,
    status: JobStatus,  // Queued, Running, Completed, Failed, Expired
    progress: BatchProgress,
    results: Option<Vec<ScrapeBatchResult>>,
    error: Option<String>,
    created_at: Instant,
    updated_at: Instant,
}

// ResearchJobStore - untuk deep_research_async
struct ResearchJobRecord {
    job_id: String,
    query: String,
    config: ResearchConfig,
    status: JobStatus,
    progress: ResearchProgress,
    final_report: Option<ResearchReport>,
    error: Option<String>,
    created_at: Instant,
    updated_at: Instant,
}
```

### Status Enum

```rust
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Expired,
}
```

### Tools

| Tool | Description |
|------|-------------|
| `scrape_batch_async` | Start async batch scrape, return job_id |
| `check_batch_status` | Poll batch job status, optional include results |
| `deep_research_async` | Start async research, return job_id |
| `check_agent_status` | Poll research job status, optional include final report |

### TTL

- 24 jam (sama dengan crawl_jobs)
- Jobs automatically expired after TTL

## API

### scrape_batch_async

**Input:**
```json
{
  "urls": ["https://example.com", "https://example.org"],
  "max_concurrent": 10,
  "max_chars": 10000
}
```

**Output:**
```json
{
  "job_id": "uuid-v4",
  "status": "queued",
  "urls_total": 2,
  "created_at": "2026-02-22T10:00:00Z"
}
```

### check_batch_status

**Input:**
```json
{
  "job_id": "uuid-v4",
  "include_results": true,
  "offset": 0,
  "limit": 50
}
```

**Output (running):**
```json
{
  "status": "running",
  "urls_total": 100,
  "urls_completed": 45,
  "urls_failed": 2,
  "progress_percent": 45
}
```

**Output (completed):**
```json
{
  "status": "completed",
  "urls_total": 100,
  "urls_completed": 98,
  "urls_failed": 2,
  "progress_percent": 100,
  "results": [...]
}
```

### deep_research_async

**Input:**
```json
{
  "query": "rust async programming best practices",
  "max_search_results": 10,
  "crawl_depth": 2
}
```

**Output:**
```json
{
  "job_id": "uuid-v4",
  "status": "queued",
  "created_at": "2026-02-22T10:00:00Z"
}
```

### check_agent_status

**Input:**
```json
{
  "job_id": "uuid-v4",
  "include_results": true
}
```

**Output (running):**
```json
{
  "status": "running",
  "current_phase": "scraping",
  "sources_processed": 5,
  "total_sources": 10,
  "progress_percent": 50
}
```

**Output (completed):**
```json
{
  "status": "completed",
  "sources_processed": 10,
  "total_sources": 10,
  "progress_percent": 100,
  "final_report": {
    "query": "...",
    "summary": "...",
    "key_findings": [...],
    "sources": [...]
  }
}
```

## Backward Compatibility

- `scrape_batch` (sync) tetap ada dan work seperti sekarang
- `deep_research` (sync) tetap ada dan work seperti sekarang
- User bisa pilih sync atau async sesuai kebutuhan

## Files to Modify

1. `types.rs` - Add JobStatus enum, BatchJobRecord, ResearchJobRecord types
2. `scrape.rs` - Add BatchJobStore and async batch logic
3. `research.rs` - Add ResearchJobStore and async research logic
4. `lib.rs` - Register job stores in AppState
5. `stdio_service.rs` - Add new tools to MCP registry

## Testing

- Unit tests untuk job stores
- Integration tests untuk async workflow
- Test TTL expiration
