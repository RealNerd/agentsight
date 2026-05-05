use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Metadata about a discovered session file.
#[derive(Debug)]
pub struct SessionFile {
    pub path: PathBuf,
    pub session_id: String,
    pub project_dir_name: String,
}

/// Discover all session JSONL files under the Claude Code projects directory.
pub fn discover_sessions(claude_dir: &Path) -> Result<Vec<SessionFile>> {
    let projects_dir = claude_dir.join("projects");
    if !projects_dir.exists() {
        anyhow::bail!(
            "Claude Code projects directory not found: {}",
            projects_dir.display()
        );
    }

    let mut sessions = Vec::new();

    for project_entry in fs::read_dir(&projects_dir)? {
        let project_entry = project_entry?;
        let project_path = project_entry.path();

        if !project_path.is_dir() {
            continue;
        }

        let project_dir_name = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        for file_entry in fs::read_dir(&project_path)? {
            let file_entry = file_entry?;
            let file_path = file_entry.path();

            if file_path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            // Skip non-UUID filenames (e.g., memory files)
            let stem = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if !looks_like_uuid(&stem) {
                continue;
            }

            sessions.push(SessionFile {
                path: file_path,
                session_id: stem,
                project_dir_name: project_dir_name.clone(),
            });
        }
    }

    Ok(sessions)
}

/// Quick check if a string looks like a UUID (has hyphens, right length).
fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4
}
