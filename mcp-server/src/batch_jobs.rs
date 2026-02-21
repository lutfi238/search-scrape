// Async batch scrape job store - in-memory job lifecycle management

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::types::{BatchProgress, JobStatus, ScrapeBatchResult};

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
            let completed = results.iter().filter(|result| result.success).count();
            let failed = results.len().saturating_sub(completed);
            job.progress = BatchProgress {
                urls_completed: completed,
                urls_failed: failed,
                progress_percent: if total > 0 {
                    (completed as f32 / total as f32) * 100.0
                } else {
                    100.0
                },
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

    pub async fn update_progress(
        &self,
        job_id: &str,
        completed: usize,
        failed: usize,
        total: usize,
    ) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.progress = BatchProgress {
                urls_completed: completed,
                urls_failed: failed,
                progress_percent: if total > 0 {
                    (completed as f32 / total as f32) * 100.0
                } else {
                    0.0
                },
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
        let urls = vec![
            "https://example.com".to_string(),
            "https://example.org".to_string(),
        ];
        let job_id = store.create_job(urls, 10, 10000).await;

        store.update_progress(&job_id, 1, 1, 2).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.progress.urls_completed, 1);
        assert_eq!(job.progress.urls_failed, 1);
    }
}
