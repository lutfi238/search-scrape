// Async crawl job store - in-memory job lifecycle management
//
// Provides create/update/query/expire operations for crawl jobs
// backed by a tokio::sync::RwLock<HashMap<...>>.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::types::{CrawlJobStatus, CrawlPageResult};

/// A single crawl job record tracking lifecycle state and results.
#[derive(Debug, Clone)]
pub struct CrawlJobRecord {
    pub job_id: String,
    pub start_url: String,
    pub status: CrawlJobStatus,
    pub pages_crawled: usize,
    pub pages_total: Option<usize>,
    pub error: Option<String>,
    pub results: Option<Vec<CrawlPageResult>>,
    pub created_at: Instant,
    pub updated_at: Instant,
}

/// In-memory async job store with TTL-based expiration.
pub struct CrawlJobStore {
    jobs: RwLock<HashMap<String, CrawlJobRecord>>,
    ttl: Duration,
}

impl CrawlJobStore {
    /// Create a new store with the given job time-to-live.
    pub fn new(ttl: Duration) -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// Create a new job in Queued state. Returns the generated job ID.
    pub async fn create_job(&self, start_url: String) -> String {
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = Instant::now();
        let record = CrawlJobRecord {
            job_id: job_id.clone(),
            start_url,
            status: CrawlJobStatus::Queued,
            pages_crawled: 0,
            pages_total: None,
            error: None,
            results: None,
            created_at: now,
            updated_at: now,
        };
        let mut jobs = self.jobs.write().await;
        jobs.insert(job_id.clone(), record);
        job_id
    }

    /// Transition a job to Running state.
    pub async fn mark_running(&self, job_id: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = CrawlJobStatus::Running;
            job.updated_at = Instant::now();
        }
    }

    /// Transition a job to Completed state with final results.
    pub async fn mark_completed(&self, job_id: &str, results: Vec<CrawlPageResult>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.pages_crawled = results.len();
            job.results = Some(results);
            job.status = CrawlJobStatus::Completed;
            job.updated_at = Instant::now();
        }
    }

    /// Transition a job to Failed state with an error message.
    pub async fn mark_failed(&self, job_id: &str, error: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = CrawlJobStatus::Failed;
            job.error = Some(error);
            job.updated_at = Instant::now();
        }
    }

    /// Retrieve a snapshot of a job record, or None if not found.
    pub async fn get_job(&self, job_id: &str) -> Option<CrawlJobRecord> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    /// Remove all jobs older than the configured TTL.
    /// Returns the number of expired jobs removed.
    pub async fn expire_jobs(&self) -> usize {
        let mut jobs = self.jobs.write().await;
        let before = jobs.len();
        jobs.retain(|_, record| record.created_at.elapsed() < self.ttl);
        before - jobs.len()
    }

    /// Update progress counters for a running job.
    pub async fn update_progress(
        &self,
        job_id: &str,
        pages_crawled: usize,
        pages_total: Option<usize>,
    ) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.pages_crawled = pages_crawled;
            job.pages_total = pages_total;
            job.updated_at = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CrawlJobStatus;

    #[tokio::test]
    async fn test_create_job_initial_state_is_queued() {
        let store = CrawlJobStore::new(std::time::Duration::from_secs(60));
        let job_id = store.create_job("https://example.com".to_string()).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, CrawlJobStatus::Queued);
        assert_eq!(job.start_url, "https://example.com");
        assert_eq!(job.pages_crawled, 0);
        assert!(job.error.is_none());
        assert!(job.results.is_none());
    }

    #[tokio::test]
    async fn test_transition_to_running_and_completed() {
        let store = CrawlJobStore::new(std::time::Duration::from_secs(60));
        let job_id = store.create_job("https://example.com".to_string()).await;

        store.mark_running(&job_id).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, CrawlJobStatus::Running);

        let results = vec![crate::types::CrawlPageResult {
            url: "https://example.com".to_string(),
            depth: 0,
            success: true,
            title: Some("Example".to_string()),
            word_count: Some(100),
            links_found: Some(5),
            content_preview: None,
            error: None,
            duration_ms: 42,
        }];
        store.mark_completed(&job_id, results.clone()).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, CrawlJobStatus::Completed);
        assert_eq!(job.pages_crawled, 1);
        let r = job.results.as_ref().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "https://example.com");
    }

    #[tokio::test]
    async fn test_job_not_found_returns_none() {
        let store = CrawlJobStore::new(std::time::Duration::from_secs(60));
        let result = store.get_job("nonexistent-id").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_expire_old_jobs_removes_stale_entries() {
        // Use a very short TTL so the job is immediately stale
        let store = CrawlJobStore::new(std::time::Duration::from_millis(1));
        let job_id = store.create_job("https://example.com".to_string()).await;

        // Small delay to ensure TTL has passed
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let expired_count = store.expire_jobs().await;
        assert_eq!(expired_count, 1);

        // Job should no longer be retrievable
        assert!(store.get_job(&job_id).await.is_none());
    }

    #[tokio::test]
    async fn test_mark_failed_sets_error() {
        let store = CrawlJobStore::new(std::time::Duration::from_secs(60));
        let job_id = store.create_job("https://example.com".to_string()).await;
        store.mark_running(&job_id).await;
        store
            .mark_failed(&job_id, "connection timeout".to_string())
            .await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, CrawlJobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("connection timeout"));
    }

    #[tokio::test]
    async fn test_expire_does_not_remove_fresh_jobs() {
        let store = CrawlJobStore::new(std::time::Duration::from_secs(3600));
        let job_id = store.create_job("https://example.com".to_string()).await;
        let expired_count = store.expire_jobs().await;
        assert_eq!(expired_count, 0);
        assert!(store.get_job(&job_id).await.is_some());
    }
}
