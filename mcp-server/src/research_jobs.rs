// Async research job store - in-memory job lifecycle management

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::types::{JobStatus, ResearchProgress, ResearchReport};

pub type ResearchJobStatus = JobStatus;

#[derive(Debug, Clone, Default)]
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

    pub async fn create_job(&self, query: String, config: Option<ResearchConfig>) -> String {
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = Instant::now();
        let record = ResearchJobRecord {
            job_id: job_id.clone(),
            query,
            config: config.unwrap_or_default(),
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
            job.progress.current_phase = "running".to_string();
            job.updated_at = Instant::now();
        }
    }

    pub async fn mark_completed(&self, job_id: &str, report: ResearchReport) {
        let sources_len = report.sources.len();
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.final_report = Some(report);
            job.status = ResearchJobStatus::Completed;
            job.progress.current_phase = "completed".to_string();
            job.progress.sources_processed = sources_len;
            job.progress.total_sources = sources_len;
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
                progress_percent: if total > 0 {
                    (processed as f32 / total as f32) * 100.0
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

    #[tokio::test]
    async fn test_create_research_job_initial_state_is_queued() {
        let store = ResearchJobStore::new(std::time::Duration::from_secs(60));
        let job_id = store.create_job("rust async".to_string(), None).await;
        let job = store.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, crate::types::JobStatus::Queued);
        assert_eq!(job.query, "rust async");
    }
}
