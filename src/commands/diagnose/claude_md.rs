use std::path::PathBuf;

use super::types::ClaudeMdAnalysis;

/// Try to find a CLAUDE.md file at the decoded project path.
pub fn find_claude_md(decoded_project_path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(decoded_project_path).join("CLAUDE.md");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Analyze a CLAUDE.md file for a project.
pub fn analyze_claude_md(decoded_project_path: &str, include_content: bool) -> ClaudeMdAnalysis {
    match find_claude_md(decoded_project_path) {
        None => ClaudeMdAnalysis {
            exists: false,
            path: None,
            size_bytes: 0,
            estimated_tokens: 0,
            oversized: false,
            content: None,
            recommendations: vec![
                "No CLAUDE.md found. Adding one with project structure and key commands improves cache stability.".to_string(),
            ],
        },
        Some(path) => {
            let content_result = std::fs::read_to_string(&path);
            let (size_bytes, content_str) = match &content_result {
                Ok(s) => (s.len() as u64, Some(s.clone())),
                Err(_) => (0, None),
            };
            let estimated_tokens = size_bytes / 4;
            let oversized = estimated_tokens > 8000;

            let mut recommendations = Vec::new();
            if oversized {
                recommendations.push(format!(
                    "CLAUDE.md is ~{} tokens ({}KB). Consider trimming to <8K tokens for better cache efficiency.",
                    estimated_tokens, size_bytes / 1024
                ));
            }

            ClaudeMdAnalysis {
                exists: true,
                path: Some(path),
                size_bytes,
                estimated_tokens,
                oversized,
                content: if include_content { content_str } else { None },
                recommendations,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_claude_md_missing() {
        let analysis = analyze_claude_md("/nonexistent/path/that/does/not/exist", false);
        assert!(!analysis.exists);
        assert!(analysis.path.is_none());
        assert!(!analysis.recommendations.is_empty());
        assert!(analysis.recommendations[0].contains("No CLAUDE.md"));
    }
}
