use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;

/// A single line from a Claude Code session JSONL file.
/// Unknown entry types (e.g. "summary") are captured as `Other` and silently ignored.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum SessionEntry {
    #[serde(rename = "assistant")]
    Assistant(AssistantEntry),
    #[serde(rename = "user")]
    User(UserEntry),
    #[serde(rename = "progress")]
    Progress(ProgressEntry),
    #[serde(rename = "system")]
    System(SystemEntry),
    #[serde(rename = "file-history-snapshot")]
    FileHistorySnapshot(FileHistorySnapshotEntry),
    #[serde(rename = "queue-operation")]
    QueueOperation(QueueOperationEntry),
    #[serde(other)]
    Unknown,
}

// ── Common fields ──────────────────────────────────────────────────

/// Fields shared across most entry types.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonFields {
    pub uuid: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub parent_uuid: Option<String>,
    pub cwd: Option<String>,
    pub version: Option<String>,
    pub git_branch: Option<String>,
    pub slug: Option<String>,
    pub is_sidechain: Option<bool>,
}

// ── Assistant entries ──────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEntry {
    #[serde(flatten)]
    pub common: CommonFields,
    pub message: AssistantMessage,
    pub request_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub model: Option<String>,
    pub id: Option<String>,
    pub role: Option<String>,
    pub content: Option<Vec<ContentBlock>>,
    pub stop_reason: Option<String>,
    pub usage: Option<TokenUsage>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "thinking")]
    Thinking {
        thinking: Option<String>,
        signature: Option<String>,
    },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    pub cache_creation: Option<CacheCreationBreakdown>,
    pub service_tier: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CacheCreationBreakdown {
    #[serde(default)]
    pub ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    pub ephemeral_1h_input_tokens: u64,
}

// ── User entries ───────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEntry {
    #[serde(flatten)]
    pub common: CommonFields,
    pub message: UserMessage,
    pub tool_use_result: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct UserMessage {
    pub role: Option<String>,
    pub content: Option<serde_json::Value>,
}


// ── Progress entries ───────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEntry {
    #[serde(flatten)]
    pub common: CommonFields,
    pub data: Option<ProgressData>,
    pub tool_use_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressData {
    #[serde(rename = "type")]
    pub data_type: Option<String>,
    pub hook_event: Option<String>,
    pub hook_name: Option<String>,
    pub command: Option<String>,
}

// ── System entries ─────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemEntry {
    #[serde(flatten)]
    pub common: CommonFields,
    pub subtype: Option<String>,
    pub duration_ms: Option<u64>,
}

// ── File history snapshot entries ──────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistorySnapshotEntry {
    pub message_id: Option<String>,
    pub snapshot: Option<serde_json::Value>,
    pub is_snapshot_update: Option<bool>,
}

// ── Queue operation entries ────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueOperationEntry {
    pub operation: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub session_id: Option<String>,
    pub content: Option<String>,
}

// ── Aggregated session data ────────────────────────────────────────

/// Summary of a parsed session, computed from all entries.
#[derive(Debug, Default, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    pub slug: Option<String>,
    pub project_path: String,
    pub model: Option<String>,
    pub git_branch: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub turns: Vec<TurnSummary>,
    pub total_usage: TokenUsage,
    pub tool_calls: HashMap<String, u32>,
}

/// Token usage and tools for a single assistant turn.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TurnSummary {
    pub index: usize,
    pub timestamp: Option<DateTime<Utc>>,
    pub usage: TokenUsage,
    pub tools: Vec<String>,
    pub model: Option<String>,
    /// The command strings from Bash tool_use blocks in this turn (truncated to 500 chars each).
    pub bash_commands: Vec<String>,
}
