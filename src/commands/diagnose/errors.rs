use std::collections::HashMap;

use crate::parser::types::{ContentBlock, SessionEntry};

use super::types::{BashRetry, BashRetryPattern};

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC [ ... <letter> sequences
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Normalize an error string for comparison: strip ANSI, shorten paths, normalize line numbers, truncate.
fn normalize_error(s: &str) -> String {
    let stripped = strip_ansi(s);

    // Shorten filesystem paths to last 2 segments
    let mut normalized = String::with_capacity(stripped.len());
    let mut i = 0;
    let bytes = stripped.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] != b' ' {
            // Found start of a path -- collect the whole path
            let start = i;
            let mut end = i + 1;
            while end < bytes.len() && !matches!(bytes[end], b' ' | b'\n' | b'\t' | b':' | b')') {
                end += 1;
            }
            let path = &stripped[start..end];
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            if segments.len() > 2 {
                normalized.push_str(".../");
                normalized.push_str(&segments[segments.len() - 2..].join("/"));
            } else {
                normalized.push_str(path);
            }
            i = end;
        } else {
            normalized.push(bytes[i] as char);
            i += 1;
        }
    }

    // Normalize line numbers like :123: to :_:
    let mut result = String::with_capacity(normalized.len());
    let mut chars = normalized.chars().peekable();
    while let Some(c) = chars.next() {
        result.push(c);
        if c == ':' {
            // Check if followed by digits then ':'
            let mut digits = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    digits.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            if !digits.is_empty() && chars.peek() == Some(&':') {
                result.push('_');
                result.push(':');
                chars.next(); // consume the trailing ':'
            } else {
                result.push_str(&digits);
            }
        }
    }

    // Truncate to 200 chars
    result.chars().take(200).collect()
}

/// Detect 3+ consecutive Bash calls that produce the same error output.
pub fn detect_same_error_retries(entries: &[SessionEntry]) -> Vec<BashRetry> {
    // Step 1: Walk assistant entries to build a map of tool_use_id -> (command, turn_index)
    let mut tool_commands: HashMap<String, (String, usize)> = HashMap::new();
    let mut turn_index = 0;

    for entry in entries {
        if let SessionEntry::Assistant(assistant) = entry {
            if let Some(content) = &assistant.message.content {
                for block in content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        if name == "Bash" {
                            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                                tool_commands.insert(
                                    id.clone(),
                                    (cmd.chars().take(500).collect(), turn_index),
                                );
                            }
                        }
                    }
                }
            }
            turn_index += 1;
        }
    }

    // Step 2: Walk user entries to find tool_result blocks with is_error == true
    struct ErrorResult {
        command: String,
        normalized_error: String,
        turn_index: usize,
    }

    let mut error_results: Vec<ErrorResult> = Vec::new();

    for entry in entries {
        if let SessionEntry::User(user) = entry {
            if let Some(content) = &user.message.content {
                // Content can be an array of tool_result objects
                if let Some(arr) = content.as_array() {
                    for item in arr {
                        let is_error = item
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if !is_error {
                            continue;
                        }

                        let tool_use_id = match item.get("tool_use_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => continue,
                        };

                        let error_content =
                            item.get("content").and_then(|v| v.as_str()).unwrap_or("");

                        if let Some((cmd, tidx)) = tool_commands.get(tool_use_id) {
                            error_results.push(ErrorResult {
                                command: cmd.clone(),
                                normalized_error: normalize_error(error_content),
                                turn_index: *tidx,
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort by turn_index
    error_results.sort_by_key(|e| e.turn_index);

    // Step 3: Detect 3+ consecutive same-error streaks
    let mut retries = Vec::new();
    let mut streak_start: Option<usize> = None;
    let mut streak_len = 0;
    let mut streak_error: Option<&str> = None;
    let mut streak_cmd: Option<&str> = None;

    for (i, er) in error_results.iter().enumerate() {
        let continues = match streak_error {
            Some(prev) => prev == er.normalized_error,
            None => false,
        };

        if continues {
            streak_len += 1;
        } else {
            if streak_len >= 3 {
                retries.push(BashRetry {
                    pattern: BashRetryPattern::SameError {
                        command: streak_cmd.unwrap_or("").to_string(),
                        error_snippet: streak_error.unwrap_or("").to_string(),
                    },
                    start_turn: error_results[streak_start.unwrap()].turn_index,
                    length: streak_len,
                });
            }
            streak_start = Some(i);
            streak_len = 1;
            streak_error = Some(&er.normalized_error);
            streak_cmd = Some(&er.command);
        }
    }

    // Flush trailing streak
    if streak_len >= 3 {
        retries.push(BashRetry {
            pattern: BashRetryPattern::SameError {
                command: streak_cmd.unwrap_or("").to_string(),
                error_snippet: streak_error.unwrap_or("").to_string(),
            },
            start_turn: error_results[streak_start.unwrap()].turn_index,
            length: streak_len,
        });
    }

    retries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::{
        AssistantEntry, AssistantMessage, CommonFields, TokenUsage, UserEntry, UserMessage,
    };

    fn make_assistant_entry(tool_use_id: &str, command: &str) -> SessionEntry {
        SessionEntry::Assistant(AssistantEntry {
            common: CommonFields {
                uuid: None,
                session_id: None,
                timestamp: None,
                parent_uuid: None,
                cwd: None,
                version: None,
                git_branch: None,
                slug: None,
                is_sidechain: None,
            },
            message: AssistantMessage {
                model: Some("test-model".to_string()),
                id: None,
                role: Some("assistant".to_string()),
                content: Some(vec![ContentBlock::ToolUse {
                    id: tool_use_id.to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({"command": command}),
                }]),
                stop_reason: None,
                usage: Some(TokenUsage::default()),
            },
            request_id: None,
        })
    }

    fn make_user_entry(tool_use_id: &str, error_content: &str, is_error: bool) -> SessionEntry {
        SessionEntry::User(UserEntry {
            common: CommonFields {
                uuid: None,
                session_id: None,
                timestamp: None,
                parent_uuid: None,
                cwd: None,
                version: None,
                git_branch: None,
                slug: None,
                is_sidechain: None,
            },
            message: UserMessage {
                role: Some("user".to_string()),
                content: Some(serde_json::json!([{
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": error_content,
                    "is_error": is_error
                }])),
            },
            tool_use_result: None,
        })
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[31merror\x1b[0m"), "error");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
        assert_eq!(strip_ansi("\x1b[1;31mbold red\x1b[0m"), "bold red");
    }

    #[test]
    fn test_normalize_error_path_shortening() {
        let input = "error in /Users/foo/bar/baz/src/main.rs";
        let normalized = normalize_error(input);
        assert!(normalized.contains(".../src/main.rs"));
        assert!(!normalized.contains("/Users/foo/bar/baz"));
    }

    #[test]
    fn test_normalize_error_line_numbers() {
        let input = "error at file.rs:42: something failed";
        let normalized = normalize_error(input);
        assert!(normalized.contains("file.rs:_:"));
        assert!(!normalized.contains(":42:"));
    }

    #[test]
    fn test_normalize_error_truncation() {
        let long_error = "x".repeat(300);
        let normalized = normalize_error(&long_error);
        assert_eq!(normalized.len(), 200);
    }

    #[test]
    fn test_same_error_3x_detected() {
        let entries = vec![
            make_assistant_entry("t1", "pulumi up"),
            make_user_entry("t1", "error: no Pulumi.yaml found", true),
            make_assistant_entry("t2", "pulumi up"),
            make_user_entry("t2", "error: no Pulumi.yaml found", true),
            make_assistant_entry("t3", "pulumi up"),
            make_user_entry("t3", "error: no Pulumi.yaml found", true),
        ];

        let result = detect_same_error_retries(&entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].length, 3);
        if let BashRetryPattern::SameError { ref command, .. } = result[0].pattern {
            assert_eq!(command, "pulumi up");
        } else {
            panic!("expected SameError pattern");
        }
    }

    #[test]
    fn test_same_error_different_errors_not_triggered() {
        let entries = vec![
            make_assistant_entry("t1", "cargo test"),
            make_user_entry("t1", "error: cannot find module 'foo'", true),
            make_assistant_entry("t2", "cargo test"),
            make_user_entry("t2", "error: type mismatch in bar", true),
            make_assistant_entry("t3", "cargo test"),
            make_user_entry("t3", "error: unused variable 'x'", true),
        ];

        let result = detect_same_error_retries(&entries);
        assert!(result.is_empty());
    }

    #[test]
    fn test_same_error_non_error_results_ignored() {
        let entries = vec![
            make_assistant_entry("t1", "ls"),
            make_user_entry("t1", "file1.txt\nfile2.txt", false),
            make_assistant_entry("t2", "ls"),
            make_user_entry("t2", "file1.txt\nfile2.txt", false),
            make_assistant_entry("t3", "ls"),
            make_user_entry("t3", "file1.txt\nfile2.txt", false),
        ];

        let result = detect_same_error_retries(&entries);
        assert!(result.is_empty());
    }
}
