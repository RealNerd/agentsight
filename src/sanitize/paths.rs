use std::collections::HashMap;

/// Deterministic path mapper for sanitizing file paths in session data.
///
/// The first `cwd` encountered sets the project root. Paths under that root
/// are mapped to `/project/...`. Home directories map to `/home/user/...`.
/// Everything else maps to `/external/...`.
#[derive(Default)]
pub struct PathMapper {
    project_root: Option<String>,
    home_prefix: Option<String>,
    /// Cache: real prefix → sanitized prefix
    prefix_cache: HashMap<String, String>,
}

impl PathMapper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the project root from the first `cwd` field encountered.
    /// Subsequent calls are no-ops.
    pub fn set_project_root(&mut self, cwd: &str) {
        if self.project_root.is_some() {
            return;
        }
        let root = cwd.trim_end_matches('/').to_string();
        self.project_root = Some(root);

        // Also detect home directory from the cwd
        if self.home_prefix.is_none() {
            if let Some(home) = detect_home_prefix(cwd) {
                self.home_prefix = Some(home);
            }
        }
    }

    /// Sanitize a single file path.
    pub fn sanitize_path(&mut self, path: &str) -> String {
        if path.is_empty() {
            return String::new();
        }

        // Check prefix cache first
        for (real, sanitized) in &self.prefix_cache {
            if let Some(rest) = path.strip_prefix(real.as_str()) {
                let rest = rest.strip_prefix('/').unwrap_or(rest);
                if rest.is_empty() {
                    return sanitized.clone();
                }
                return format!("{}/{}", sanitized, rest);
            }
        }

        // Try project root match
        if let Some(ref root) = self.project_root {
            if let Some(rest) = path.strip_prefix(root.as_str()) {
                let rest = rest.strip_prefix('/').unwrap_or(rest);
                if rest.is_empty() {
                    return "/project".to_string();
                }
                return format!("/project/{}", rest);
            }
        }

        // Try home directory match
        if let Some(ref home) = self.home_prefix {
            if let Some(rest) = path.strip_prefix(home.as_str()) {
                let rest = rest.strip_prefix('/').unwrap_or(rest);
                if rest.is_empty() {
                    return "/home/user".to_string();
                }
                return format!("/home/user/{}", rest);
            }
        }

        // Detect home pattern even if we haven't seen cwd yet
        if let Some(home) = detect_home_prefix(path) {
            let rest = path
                .strip_prefix(&home)
                .unwrap_or("")
                .strip_prefix('/')
                .unwrap_or("");
            self.home_prefix = Some(home);
            if rest.is_empty() {
                return "/home/user".to_string();
            }
            return format!("/home/user/{}", rest);
        }

        // External path
        if path.starts_with('/') {
            // Keep last two path components for readability
            let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            if parts.len() <= 2 {
                return format!("/external/{}", parts.join("/"));
            }
            let tail = &parts[parts.len() - 2..];
            return format!("/external/{}", tail.join("/"));
        }

        // Relative path — pass through
        path.to_string()
    }

    /// Sanitize a bash command string by replacing path-like tokens.
    pub fn sanitize_bash_command(&mut self, cmd: &str) -> String {
        let mut result = String::with_capacity(cmd.len());
        let chars: Vec<char> = cmd.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Detect path-like tokens starting with / or ~/
            if chars[i] == '/' || (chars[i] == '~' && i + 1 < len && chars[i + 1] == '/') {
                let start = i;
                // Scan ahead to find end of path token
                i += 1;
                while i < len && is_path_char(chars[i]) {
                    i += 1;
                }
                let token: String = chars[start..i].iter().collect();
                // Expand ~ to home prefix if known
                let expanded = if let Some(rest) = token.strip_prefix("~/") {
                    if let Some(ref home) = self.home_prefix {
                        format!("{}/{}", home, rest)
                    } else {
                        token.clone()
                    }
                } else {
                    token.clone()
                };
                result.push_str(&self.sanitize_path(&expanded));
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        result
    }

    /// Register an explicit prefix mapping (for testing or special cases).
    pub fn add_prefix(&mut self, real: String, sanitized: String) {
        self.prefix_cache.insert(real, sanitized);
    }
}

/// Detect the home directory prefix from a path.
/// Matches `/Users/<name>` (macOS) or `/home/<name>` (Linux).
fn detect_home_prefix(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    // /Users/<name>/... → parts = ["", "Users", "<name>", ...]
    if parts.len() >= 3 && parts[1] == "Users" && !parts[2].is_empty() {
        return Some(format!("/Users/{}", parts[2]));
    }
    // /home/<name>/... → parts = ["", "home", "<name>", ...]
    if parts.len() >= 3 && parts[1] == "home" && !parts[2].is_empty() {
        return Some(format!("/home/{}", parts[2]));
    }
    None
}

/// Check if a character is valid in a file path token in a bash command.
fn is_path_char(c: char) -> bool {
    matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '.' | '_' | '-' | '+' | '@' | ':' | '~')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_root_mapping() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        assert_eq!(
            pm.sanitize_path("/Users/alice/repos/myproject/src/main.rs"),
            "/project/src/main.rs"
        );
        assert_eq!(pm.sanitize_path("/Users/alice/repos/myproject"), "/project");
    }

    #[test]
    fn test_determinism() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        let a = pm.sanitize_path("/Users/alice/repos/myproject/src/lib.rs");
        let b = pm.sanitize_path("/Users/alice/repos/myproject/src/lib.rs");
        assert_eq!(a, b);
    }

    #[test]
    fn test_home_dir_mapping() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        assert_eq!(
            pm.sanitize_path("/Users/alice/.config/settings.toml"),
            "/home/user/.config/settings.toml"
        );
    }

    #[test]
    fn test_linux_home_dir() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/home/bob/projects/app");

        assert_eq!(
            pm.sanitize_path("/home/bob/.local/bin/tool"),
            "/home/user/.local/bin/tool"
        );
        assert_eq!(
            pm.sanitize_path("/home/bob/projects/app/Cargo.toml"),
            "/project/Cargo.toml"
        );
    }

    #[test]
    fn test_external_path() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        let result = pm.sanitize_path("/usr/local/bin/rustc");
        assert_eq!(result, "/external/bin/rustc");
    }

    #[test]
    fn test_external_short_path() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        assert_eq!(pm.sanitize_path("/tmp"), "/external/tmp");
        assert_eq!(pm.sanitize_path("/tmp/foo"), "/external/tmp/foo");
    }

    #[test]
    fn test_empty_path() {
        let mut pm = PathMapper::new();
        assert_eq!(pm.sanitize_path(""), "");
    }

    #[test]
    fn test_relative_path_passthrough() {
        let mut pm = PathMapper::new();
        assert_eq!(pm.sanitize_path("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn test_set_project_root_idempotent() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/proj1");
        pm.set_project_root("/Users/alice/repos/proj2");

        // Should still use first root
        assert_eq!(
            pm.sanitize_path("/Users/alice/repos/proj1/file.rs"),
            "/project/file.rs"
        );
    }

    #[test]
    fn test_bash_command_path_replacement() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        let result = pm.sanitize_bash_command(
            "cargo test --manifest-path /Users/alice/repos/myproject/Cargo.toml",
        );
        assert_eq!(result, "cargo test --manifest-path /project/Cargo.toml");
    }

    #[test]
    fn test_bash_command_tilde_expansion() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        let result = pm.sanitize_bash_command("cat ~/.config/settings.toml");
        assert_eq!(result, "cat /home/user/.config/settings.toml");
    }

    #[test]
    fn test_bash_command_no_paths() {
        let mut pm = PathMapper::new();
        let result = pm.sanitize_bash_command("echo hello world");
        assert_eq!(result, "echo hello world");
    }

    #[test]
    fn test_bash_command_multiple_paths() {
        let mut pm = PathMapper::new();
        pm.set_project_root("/Users/alice/repos/myproject");

        let result = pm.sanitize_bash_command(
            "diff /Users/alice/repos/myproject/a.rs /Users/alice/repos/myproject/b.rs",
        );
        assert_eq!(result, "diff /project/a.rs /project/b.rs");
    }

    #[test]
    fn test_home_detection_without_project_root() {
        let mut pm = PathMapper::new();
        // Even without set_project_root, home dirs should be detected
        assert_eq!(
            pm.sanitize_path("/Users/charlie/Documents/file.txt"),
            "/home/user/Documents/file.txt"
        );
    }

    #[test]
    fn test_prefix_cache() {
        let mut pm = PathMapper::new();
        pm.add_prefix("/opt/homebrew".to_string(), "/homebrew".to_string());
        assert_eq!(
            pm.sanitize_path("/opt/homebrew/bin/node"),
            "/homebrew/bin/node"
        );
    }
}
