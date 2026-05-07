use serde_json::Value;

use super::SanitizeContext;

/// Sanitize an assistant entry's message content blocks.
pub fn sanitize_assistant(val: &mut Value, ctx: &mut SanitizeContext) {
    if let Some(content) = val
        .get_mut("message")
        .and_then(|m| m.get_mut("content"))
        .and_then(|c| c.as_array_mut())
    {
        for block in content.iter_mut() {
            let block_type = block
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();

            match block_type.as_str() {
                "text" => {
                    block["text"] = Value::String(ctx.next_text());
                }
                "thinking" => {
                    // Replace thinking content but preserve signature field
                    block["thinking"] = Value::String("[thinking]".to_string());
                }
                "tool_use" => {
                    sanitize_tool_input(block, ctx);
                }
                _ => {}
            }
        }
    }
}

/// Sanitize tool_use input based on the tool name.
fn sanitize_tool_input(block: &mut Value, ctx: &mut SanitizeContext) {
    let tool_name = block
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();

    let input = match block.get_mut("input") {
        Some(i) => i,
        None => return,
    };

    match tool_name.as_str() {
        "Edit" => {
            if input.get("old_string").is_some() {
                input["old_string"] = Value::String(ctx.next_old_code());
            }
            if input.get("new_string").is_some() {
                input["new_string"] = Value::String(ctx.next_new_code());
            }
            sanitize_input_path(input, "file_path", ctx);
        }
        "Write" => {
            if input.get("content").is_some() {
                input["content"] = Value::String(ctx.next_file_content());
            }
            sanitize_input_path(input, "file_path", ctx);
        }
        "Read" => {
            sanitize_input_path(input, "file_path", ctx);
        }
        "Bash" => {
            if let Some(cmd) = input
                .get("command")
                .and_then(|c| c.as_str())
                .map(String::from)
            {
                input["command"] = Value::String(ctx.path_mapper.sanitize_bash_command(&cmd));
            }
            if let Some(desc) = input
                .get("description")
                .and_then(|d| d.as_str())
                .map(String::from)
            {
                input["description"] = Value::String(ctx.path_mapper.sanitize_bash_command(&desc));
            }
        }
        "Grep" | "Glob" => {
            if input.get("pattern").is_some() {
                input["pattern"] = Value::String(ctx.next_pattern());
            }
            sanitize_input_path(input, "path", ctx);
        }
        "Task" => {
            if input.get("prompt").is_some() {
                input["prompt"] = Value::String(ctx.next_text());
            }
        }
        "WebFetch" | "WebSearch" => {
            // Preserve URLs but sanitize prompt text
            if input.get("prompt").is_some() {
                input["prompt"] = Value::String(ctx.next_text());
            }
            if input.get("query").is_some() {
                input["query"] = Value::String(ctx.next_text());
            }
        }
        _ => {
            // Unknown tools: sanitize any file_path or path field
            sanitize_input_path(input, "file_path", ctx);
            sanitize_input_path(input, "path", ctx);
        }
    }
}

/// Sanitize a path field in a tool input object.
fn sanitize_input_path(input: &mut Value, field: &str, ctx: &mut SanitizeContext) {
    if let Some(path) = input.get(field).and_then(|p| p.as_str()).map(String::from) {
        input[field] = Value::String(ctx.path_mapper.sanitize_path(&path));
    }
}

/// Sanitize a user entry.
pub fn sanitize_user(val: &mut Value, ctx: &mut SanitizeContext) {
    // User message content: can be a plain string or an array
    if let Some(msg) = val.get_mut("message") {
        if let Some(content) = msg.get_mut("content") {
            if content.is_string() {
                *content = Value::String(ctx.next_prompt());
            } else if let Some(arr) = content.as_array_mut() {
                for item in arr.iter_mut() {
                    // Tool result content blocks
                    if let Some(text) = item.get_mut("text") {
                        if text.is_string() {
                            *text = Value::String(ctx.next_tool_output());
                        }
                    }
                    if let Some(content_inner) = item.get_mut("content") {
                        if content_inner.is_string() {
                            *content_inner = Value::String(ctx.next_tool_output());
                        } else if let Some(inner_arr) = content_inner.as_array_mut() {
                            for inner_item in inner_arr.iter_mut() {
                                if let Some(text) = inner_item.get_mut("text") {
                                    if text.is_string() {
                                        *text = Value::String(ctx.next_tool_output());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // toolUseResult
    if let Some(tur) = val.get_mut("toolUseResult") {
        sanitize_tool_use_result(tur, ctx);
    }
}

/// Sanitize a toolUseResult value.
fn sanitize_tool_use_result(tur: &mut Value, ctx: &mut SanitizeContext) {
    // toolUseResult can have .content (string), .file.content, or nested structures
    if let Some(content) = tur.get_mut("content") {
        if content.is_string() {
            *content = Value::String(ctx.next_tool_output());
        } else if let Some(arr) = content.as_array_mut() {
            for item in arr.iter_mut() {
                if let Some(text) = item.get_mut("text") {
                    if text.is_string() {
                        *text = Value::String(ctx.next_tool_output());
                    }
                }
            }
        }
    }
    if let Some(file) = tur.get_mut("file") {
        if let Some(content) = file.get_mut("content") {
            if content.is_string() {
                *content = Value::String("[file content]".to_string());
            }
        }
    }
}

/// Sanitize a progress entry.
pub fn sanitize_progress(val: &mut Value, ctx: &mut SanitizeContext) {
    // data.command may contain paths
    if let Some(data) = val.get_mut("data") {
        if let Some(cmd) = data
            .get("command")
            .and_then(|c| c.as_str())
            .map(String::from)
        {
            data["command"] = Value::String(ctx.path_mapper.sanitize_bash_command(&cmd));
        }
        // data.content may have tool output
        if let Some(content) = data.get_mut("content") {
            if content.is_string() {
                *content = Value::String(ctx.next_tool_output());
            }
        }
    }
}

/// Sanitize a system entry. Mostly just cwd (handled at dispatch level).
pub fn sanitize_system(_val: &mut Value, _ctx: &mut SanitizeContext) {
    // cwd is handled in sanitize_line(); subtype, durationMs preserved.
}

/// Sanitize a file-history-snapshot entry.
pub fn sanitize_file_history(val: &mut Value, _ctx: &mut SanitizeContext) {
    // Clear trackedFileBackups (may contain code diffs)
    if let Some(snapshot) = val.get_mut("snapshot") {
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert(
                "trackedFileBackups".to_string(),
                Value::Object(serde_json::Map::new()),
            );
        }
    }
}

/// Sanitize a queue-operation entry.
pub fn sanitize_queue_op(val: &mut Value, ctx: &mut SanitizeContext) {
    if let Some(content) = val.get_mut("content") {
        if content.is_string() {
            *content = Value::String(ctx.next_prompt());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sanitize_assistant_text() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": "Here is the implementation..."},
                    {"type": "text", "text": "Let me also fix this..."}
                ]
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        let content = val["message"]["content"].as_array().unwrap();
        assert_eq!(content[0]["text"], "[assistant text 1]");
        assert_eq!(content[1]["text"], "[assistant text 2]");
    }

    #[test]
    fn test_sanitize_assistant_thinking() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "thinking", "thinking": "secret reasoning", "signature": "abc123"}
                ]
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        let block = &val["message"]["content"][0];
        assert_eq!(block["thinking"], "[thinking]");
        assert_eq!(block["signature"], "abc123"); // preserved
    }

    #[test]
    fn test_sanitize_tool_use_edit() {
        let mut ctx = SanitizeContext::new();
        ctx.path_mapper
            .set_project_root("/Users/alice/repos/myproject");

        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "tu_123",
                    "name": "Edit",
                    "input": {
                        "file_path": "/Users/alice/repos/myproject/src/main.rs",
                        "old_string": "fn old_function() {}",
                        "new_string": "fn new_function() {}"
                    }
                }]
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        let input = &val["message"]["content"][0]["input"];
        assert_eq!(input["file_path"], "/project/src/main.rs");
        assert_eq!(input["old_string"], "[old code 1]");
        assert_eq!(input["new_string"], "[new code 2]");
    }

    #[test]
    fn test_sanitize_tool_use_write() {
        let mut ctx = SanitizeContext::new();
        ctx.path_mapper
            .set_project_root("/Users/alice/repos/myproject");

        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "tu_456",
                    "name": "Write",
                    "input": {
                        "file_path": "/Users/alice/repos/myproject/src/new_file.rs",
                        "content": "use std::io;\nfn main() {}"
                    }
                }]
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        let input = &val["message"]["content"][0]["input"];
        assert_eq!(input["file_path"], "/project/src/new_file.rs");
        assert_eq!(input["content"], "[file content 1]");
    }

    #[test]
    fn test_sanitize_tool_use_bash() {
        let mut ctx = SanitizeContext::new();
        ctx.path_mapper
            .set_project_root("/Users/alice/repos/myproject");

        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "tu_789",
                    "name": "Bash",
                    "input": {
                        "command": "cargo test --manifest-path /Users/alice/repos/myproject/Cargo.toml"
                    }
                }]
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        let cmd = val["message"]["content"][0]["input"]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains("/project/Cargo.toml"));
        assert!(!cmd.contains("alice"));
    }

    #[test]
    fn test_sanitize_tool_use_grep() {
        let mut ctx = SanitizeContext::new();
        ctx.path_mapper
            .set_project_root("/Users/alice/repos/myproject");

        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "tu_grep",
                    "name": "Grep",
                    "input": {
                        "pattern": "fn main",
                        "path": "/Users/alice/repos/myproject/src"
                    }
                }]
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        let input = &val["message"]["content"][0]["input"];
        assert_eq!(input["pattern"], "pattern_1");
        assert_eq!(input["path"], "/project/src");
    }

    #[test]
    fn test_sanitize_user_string_content() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": "Fix the bug in authentication"
            }
        });
        sanitize_user(&mut val, &mut ctx);

        assert_eq!(val["message"]["content"], "[user prompt 1]");
    }

    #[test]
    fn test_sanitize_user_array_content() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "tu_123", "content": "output from tool"}
                ]
            }
        });
        sanitize_user(&mut val, &mut ctx);

        let item = &val["message"]["content"][0];
        assert_eq!(item["content"], "[tool output 1]");
        assert_eq!(item["tool_use_id"], "tu_123"); // preserved
    }

    #[test]
    fn test_sanitize_tool_use_result() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "user",
            "toolUseResult": {
                "content": "file contents here",
                "file": {
                    "content": "actual code"
                }
            },
            "message": {"role": "user", "content": "ok"}
        });
        sanitize_user(&mut val, &mut ctx);

        assert_eq!(val["toolUseResult"]["content"], "[tool output 1]");
        assert_eq!(val["toolUseResult"]["file"]["content"], "[file content]");
    }

    #[test]
    fn test_sanitize_progress() {
        let mut ctx = SanitizeContext::new();
        ctx.path_mapper
            .set_project_root("/Users/alice/repos/myproject");

        let mut val = json!({
            "type": "progress",
            "data": {
                "type": "bash",
                "command": "cargo build --manifest-path /Users/alice/repos/myproject/Cargo.toml"
            }
        });
        sanitize_progress(&mut val, &mut ctx);

        let cmd = val["data"]["command"].as_str().unwrap();
        assert!(cmd.contains("/project/Cargo.toml"));
        assert!(!cmd.contains("alice"));
    }

    #[test]
    fn test_sanitize_file_history() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "file-history-snapshot",
            "snapshot": {
                "trackedFileBackups": {
                    "/Users/alice/repos/myproject/src/main.rs": "old code content"
                },
                "otherField": "preserved"
            }
        });
        sanitize_file_history(&mut val, &mut ctx);

        assert_eq!(val["snapshot"]["trackedFileBackups"], json!({}));
        assert_eq!(val["snapshot"]["otherField"], "preserved");
    }

    #[test]
    fn test_sanitize_queue_op() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "content": "Please fix the login bug"
        });
        sanitize_queue_op(&mut val, &mut ctx);

        assert_eq!(val["content"], "[user prompt 1]");
        assert_eq!(val["operation"], "enqueue"); // preserved
    }

    #[test]
    fn test_preserves_token_usage() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "hello"}],
                "usage": {
                    "input_tokens": 1000,
                    "output_tokens": 500,
                    "cache_creation_input_tokens": 200,
                    "cache_read_input_tokens": 800
                },
                "model": "claude-opus-4-6",
                "stop_reason": "end_turn"
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        assert_eq!(val["message"]["usage"]["input_tokens"], 1000);
        assert_eq!(val["message"]["usage"]["output_tokens"], 500);
        assert_eq!(val["message"]["model"], "claude-opus-4-6");
        assert_eq!(val["message"]["stop_reason"], "end_turn");
    }

    #[test]
    fn test_preserves_tool_use_id_and_name() {
        let mut ctx = SanitizeContext::new();
        let mut val = json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_abc123",
                    "name": "Read",
                    "input": {"file_path": "/tmp/test"}
                }]
            }
        });
        sanitize_assistant(&mut val, &mut ctx);

        let block = &val["message"]["content"][0];
        assert_eq!(block["id"], "toolu_abc123");
        assert_eq!(block["name"], "Read");
    }
}
