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

/// Resource pool for parallel scheduling (`none` = only the global cap applies).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobResourceType {
    #[default]
    None,
    Ffmpeg,
    Chroma,
}

impl JobResourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ffmpeg => "ffmpeg",
            Self::Chroma => "chroma",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "ffmpeg" => Some(Self::Ffmpeg),
            "chroma" => Some(Self::Chroma),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub current_step: u32,
    pub total_steps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPrerequisiteView {
    pub job_id: String,
    pub short_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    /// Short numeric label shown in the UI (e.g. `#42`).
    #[serde(default)]
    pub short_id: u32,
    pub name: String,
    pub desc: String,
    pub identity: String,
    pub job_type: String,
    #[serde(default)]
    pub resource_type: JobResourceType,
    pub priority: JobPriority,
    pub status: JobStatus,
    pub cancelable: bool,
    pub progress: JobProgress,
    pub step_label: String,
    pub completion_message: Option<String>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    /// Prerequisite jobs still queued or running (completed ones are omitted).
    #[serde(default)]
    pub waiting_for: Vec<JobPrerequisiteView>,
    /// Total prerequisites registered at enqueue time (for queued progress UI).
    #[serde(default)]
    pub prerequisite_total: u32,
    /// Prerequisites still queued or running (uncapped; `waiting_for` may list fewer).
    #[serde(default)]
    pub prerequisite_pending: u32,
    /// Queued-job progress: two steps per prerequisite (start + complete).
    #[serde(default)]
    pub prerequisite_progress_current: u32,
    #[serde(default)]
    pub prerequisite_progress_total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeMaxParallel {
    pub resource_type: String,
    pub max_parallel: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsSnapshot {
    pub active: Vec<JobView>,
    pub history: Vec<JobView>,
    pub max_parallel: u32,
    pub type_max_parallel: Vec<TypeMaxParallel>,
    pub active_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueJobResult {
    pub job_id: Option<String>,
    /// True when the job was not queued because the work is already satisfied (e.g. cached sprite).
    pub skipped: bool,
    /// Detect was skipped because manual templates own skip matching; only chroma was queued.
    #[serde(default)]
    pub chroma_only: bool,
}

impl EnqueueJobResult {
    pub fn queued(job_id: Option<String>) -> Self {
        Self {
            job_id,
            skipped: false,
            chroma_only: false,
        }
    }

    pub fn skipped() -> Self {
        Self {
            job_id: None,
            skipped: true,
            chroma_only: false,
        }
    }

    pub fn chroma_only() -> Self {
        Self {
            job_id: None,
            skipped: false,
            chroma_only: true,
        }
    }

    pub fn with_skip(job_id: Option<String>, skipped: bool) -> Self {
        Self {
            job_id,
            skipped,
            chroma_only: false,
        }
    }
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

/// One IPC for the episode page scrub queue (avoids N invokes while browsing a title).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodePageScrubItem {
    pub path: String,
    pub episode_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueEpisodePageScrubSprites {
    pub priority: JobPriority,
    pub anime_title: Option<String>,
    pub episodes: Vec<EpisodePageScrubItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueOpEdDetectJob {
    pub anime_id: i64,
    pub priority: JobPriority,
    pub anime_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueOpEdChromaAnimeJob {
    pub anime_id: i64,
    pub priority: JobPriority,
    pub anime_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueOpEdChromaEpisodeJob {
    pub episode_id: i64,
    pub priority: JobPriority,
    pub anime_title: Option<String>,
}
