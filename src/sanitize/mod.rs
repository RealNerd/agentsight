pub mod content;
pub mod paths;

use paths::PathMapper;

/// Mutable context threaded through sanitization of a single session.
pub struct SanitizeContext {
    pub path_mapper: PathMapper,
    text_counter: usize,
    prompt_counter: usize,
    tool_output_counter: usize,
    code_counter: usize,
    pattern_counter: usize,
    file_content_counter: usize,
}

impl Default for SanitizeContext {
    fn default() -> Self {
        Self {
            path_mapper: PathMapper::new(),
            text_counter: 0,
            prompt_counter: 0,
            tool_output_counter: 0,
            code_counter: 0,
            pattern_counter: 0,
            file_content_counter: 0,
        }
    }
}

impl SanitizeContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_text(&mut self) -> String {
        self.text_counter += 1;
        format!("[assistant text {}]", self.text_counter)
    }

    pub fn next_prompt(&mut self) -> String {
        self.prompt_counter += 1;
        format!("[user prompt {}]", self.prompt_counter)
    }

    pub fn next_tool_output(&mut self) -> String {
        self.tool_output_counter += 1;
        format!("[tool output {}]", self.tool_output_counter)
    }

    pub fn next_error_output(&mut self) -> String {
        self.tool_output_counter += 1;
        format!("[error output {}]", self.tool_output_counter)
    }

    pub fn next_old_code(&mut self) -> String {
        self.code_counter += 1;
        format!("[old code {}]", self.code_counter)
    }

    pub fn next_new_code(&mut self) -> String {
        self.code_counter += 1;
        format!("[new code {}]", self.code_counter)
    }

    pub fn next_file_content(&mut self) -> String {
        self.file_content_counter += 1;
        format!("[file content {}]", self.file_content_counter)
    }

    pub fn next_pattern(&mut self) -> String {
        self.pattern_counter += 1;
        format!("pattern_{}", self.pattern_counter)
    }
}

/// Sanitize a single JSONL line. Returns the sanitized JSON string.
/// Malformed lines pass through unchanged.
pub fn sanitize_line(ctx: &mut SanitizeContext, line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return line.to_string();
    }

    let mut val: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return line.to_string(), // malformed passthrough
    };

    // Extract type field to dispatch
    let entry_type = val
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Set project root from first cwd we see
    if let Some(cwd) = val.get("cwd").and_then(|v| v.as_str()) {
        ctx.path_mapper.set_project_root(cwd);
    }

    match entry_type.as_str() {
        "assistant" => content::sanitize_assistant(&mut val, ctx),
        "user" => content::sanitize_user(&mut val, ctx),
        "progress" => content::sanitize_progress(&mut val, ctx),
        "system" => content::sanitize_system(&mut val, ctx),
        "file-history-snapshot" => content::sanitize_file_history(&mut val, ctx),
        "queue-operation" => content::sanitize_queue_op(&mut val, ctx),
        _ => {} // unknown types: preserve as-is
    }

    // Sanitize cwd if present
    if let Some(cwd) = val.get("cwd").and_then(|v| v.as_str()).map(String::from) {
        val["cwd"] = serde_json::Value::String(ctx.path_mapper.sanitize_path(&cwd));
    }

    serde_json::to_string(&val).unwrap_or_else(|_| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_malformed_passthrough() {
        let mut ctx = SanitizeContext::new();
        let bad = "this is not json {{{";
        assert_eq!(sanitize_line(&mut ctx, bad), bad);
    }

    #[test]
    fn test_empty_line_passthrough() {
        let mut ctx = SanitizeContext::new();
        assert_eq!(sanitize_line(&mut ctx, ""), "");
        assert_eq!(sanitize_line(&mut ctx, "  "), "  ");
    }

    #[test]
    fn test_unknown_type_preserved() {
        let mut ctx = SanitizeContext::new();
        let line = r#"{"type":"summary","data":"stuff"}"#;
        let result = sanitize_line(&mut ctx, line);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["type"], "summary");
        assert_eq!(v["data"], "stuff");
    }

    #[test]
    fn test_cwd_sanitized() {
        let mut ctx = SanitizeContext::new();
        let line = r#"{"type":"system","cwd":"/Users/alice/repos/myproject","subtype":"init"}"#;
        let result = sanitize_line(&mut ctx, line);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cwd"], "/project");
    }

    #[test]
    fn test_counter_increments() {
        let mut ctx = SanitizeContext::new();
        assert_eq!(ctx.next_text(), "[assistant text 1]");
        assert_eq!(ctx.next_text(), "[assistant text 2]");
        assert_eq!(ctx.next_prompt(), "[user prompt 1]");
        assert_eq!(ctx.next_prompt(), "[user prompt 2]");
    }
}
