use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobPriority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub current_step: u32,
    pub total_steps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub identity: String,
    pub job_type: String,
    pub priority: JobPriority,
    pub status: JobStatus,
    pub cancelable: bool,
    pub progress: JobProgress,
    pub step_label: String,
    pub completion_message: Option<String>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsSnapshot {
    pub active: Vec<JobView>,
    pub history: Vec<JobView>,
    pub max_parallel: u32,
    pub active_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueJobResult {
    pub job_id: Option<String>,
    /// True when the job was not queued because the work is already satisfied (e.g. cached sprite).
    pub skipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFinishedEvent {
    pub job_id: String,
    pub identity: String,
    pub job_type: String,
    pub status: JobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueScrubSpriteJob {
    pub path: String,
    pub priority: JobPriority,
    pub anime_title: Option<String>,
    pub episode_label: Option<String>,
    #[serde(default)]
    pub follow_up: Vec<EnqueueScrubSpriteJob>,
}
