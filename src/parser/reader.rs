use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::types::{
    AssistantEntry, ContentBlock, SessionEntry, SessionSummary, TokenUsage, TurnSummary,
};

/// Parse a session JSONL file into a sequence of typed entries.
/// Skips malformed lines. Warnings are printed to stderr only if `verbose` is true.
pub fn parse_session_file(path: &Path, verbose: bool) -> Result<Vec<SessionEntry>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if verbose {
                    eprintln!(
                        "warn: failed to read line {} in {}: {}",
                        line_num + 1,
                        path.display(),
                        e
                    );
                }
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<SessionEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                if verbose {
                    eprintln!(
                        "warn: skipping malformed line {} in {}: {}",
                        line_num + 1,
                        path.display(),
                        e
                    );
                }
            }
        }
    }

    Ok(entries)
}

/// Build a SessionSummary from parsed entries.
pub fn summarize_session(
    entries: &[SessionEntry],
    session_id: String,
    project_path: String,
) -> SessionSummary {
    let mut summary = SessionSummary {
        session_id,
        project_path,
        ..Default::default()
    };

    let mut turn_index = 0;

    for entry in entries {
        match entry {
            SessionEntry::Assistant(assistant) => {
                // Extract session metadata from first assistant entry
                if summary.slug.is_none() {
                    summary.slug = assistant.common.slug.clone();
                }
                if summary.git_branch.is_none() {
                    summary.git_branch = assistant.common.git_branch.clone();
                }
                if summary.model.is_none() {
                    if let Some(ref m) = assistant.message.model {
                        if m != "<synthetic>" {
                            summary.model = Some(m.clone());
                        }
                    }
                }

                update_time_range(&mut summary, &assistant.common.timestamp);
                accumulate_assistant(&mut summary, assistant, &mut turn_index);
            }
            SessionEntry::User(user) => {
                update_time_range(&mut summary, &user.common.timestamp);
            }
            SessionEntry::System(system) => {
                update_time_range(&mut summary, &system.common.timestamp);
            }
            SessionEntry::Progress(_)
            | SessionEntry::FileHistorySnapshot(_)
            | SessionEntry::QueueOperation(_)
            | SessionEntry::Unknown => {}
        }
    }

    summary
}

fn update_time_range(
    summary: &mut SessionSummary,
    timestamp: &Option<chrono::DateTime<chrono::Utc>>,
) {
    if let Some(ts) = timestamp {
        if summary.start_time.is_none() || summary.start_time.as_ref().is_some_and(|s| ts < s) {
            summary.start_time = Some(*ts);
        }
        if summary.end_time.is_none() || summary.end_time.as_ref().is_some_and(|e| ts > e) {
            summary.end_time = Some(*ts);
        }
    }
}

fn accumulate_assistant(
    summary: &mut SessionSummary,
    assistant: &AssistantEntry,
    turn_index: &mut usize,
) {
    let usage = assistant.message.usage.clone().unwrap_or_default();

    // Accumulate totals
    summary.total_usage.input_tokens += usage.input_tokens;
    summary.total_usage.cache_creation_input_tokens += usage.cache_creation_input_tokens;
    summary.total_usage.cache_read_input_tokens += usage.cache_read_input_tokens;
    summary.total_usage.output_tokens += usage.output_tokens;

    // Extract tool calls from content blocks
    let mut tools = Vec::new();
    if let Some(content) = &assistant.message.content {
        for block in content {
            if let ContentBlock::ToolUse { name, .. } = block {
                tools.push(name.clone());
                *summary.tool_calls.entry(name.clone()).or_insert(0) += 1;
            }
        }
    }

    // Filter out "<synthetic>" model from per-turn data
    let turn_model = assistant
        .message
        .model
        .as_deref()
        .filter(|m| *m != "<synthetic>")
        .map(|m| m.to_string());

    summary.turns.push(TurnSummary {
        index: *turn_index,
        timestamp: assistant.common.timestamp,
        usage,
        tools,
        model: turn_model,
    });

    *turn_index += 1;
}

/// Decode a project path from the directory name encoding Claude Code uses.
/// e.g. "-Users-Shared-sv-repo" -> "/Users/Shared/sv-repo"
pub fn decode_project_path(encoded: &str) -> String {
    if encoded.starts_with('-') {
        let path = encoded.replacen('-', "/", 1);
        path.replace('-', "/")
    } else {
        encoded.replace('-', "/")
    }
}

// ── Helpers for TokenUsage ─────────────────────────────────────────

impl TokenUsage {
    /// Total tokens across all buckets.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
            + self.output_tokens
    }
}

impl std::ops::AddAssign for TokenUsage {
    fn add_assign(&mut self, rhs: Self) {
        self.input_tokens += rhs.input_tokens;
        self.cache_creation_input_tokens += rhs.cache_creation_input_tokens;
        self.cache_read_input_tokens += rhs.cache_read_input_tokens;
        self.output_tokens += rhs.output_tokens;
    }
}
