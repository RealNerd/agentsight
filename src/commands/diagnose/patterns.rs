use crate::parser::types::TurnSummary;

use super::types::{BashLoop, BashRetry, BashRetryPattern, ToolPatterns};

/// Detect tool usage anti-patterns:
/// - Bash loops: 3+ consecutive turns where the only tool calls are Bash
/// - Exploration heavy: Read+Glob calls > 5x Edit+Write calls
/// - Subagent overhead: >3 Task tool calls
pub fn analyze_tool_patterns(turns: &[TurnSummary]) -> ToolPatterns {
    // Bash loop detection
    let mut bash_loops = Vec::new();
    let mut streak_start: Option<usize> = None;
    let mut streak_len = 0;

    for (i, turn) in turns.iter().enumerate() {
        let is_bash_only = !turn.tools.is_empty() && turn.tools.iter().all(|t| t == "Bash");

        if is_bash_only {
            if streak_start.is_none() {
                streak_start = Some(i);
                streak_len = 1;
            } else {
                streak_len += 1;
            }
        } else {
            if streak_len >= 3 {
                bash_loops.push(BashLoop {
                    start_turn: streak_start.unwrap(),
                    length: streak_len,
                });
            }
            streak_start = None;
            streak_len = 0;
        }
    }
    // Flush trailing streak
    if streak_len >= 3 {
        bash_loops.push(BashLoop {
            start_turn: streak_start.unwrap(),
            length: streak_len,
        });
    }

    // Read/Edit ratio from aggregated tool counts across all turns
    let mut read_glob_count: u32 = 0;
    let mut edit_write_count: u32 = 0;
    let mut task_count: usize = 0;

    for turn in turns {
        for tool in &turn.tools {
            match tool.as_str() {
                "Read" | "Glob" | "Grep" => read_glob_count += 1,
                "Edit" | "Write" => edit_write_count += 1,
                "Task" => task_count += 1,
                _ => {}
            }
        }
    }

    let read_edit_ratio = if edit_write_count > 0 {
        read_glob_count as f64 / edit_write_count as f64
    } else if read_glob_count > 0 {
        read_glob_count as f64 // treat 0 edits as ratio = read count
    } else {
        0.0
    };

    let bash_retries = detect_identical_command_retries(turns);

    ToolPatterns {
        bash_loops,
        bash_retries,
        read_edit_ratio,
        exploration_flagged: read_edit_ratio > 5.0,
        subagent_count: task_count,
        subagent_flagged: task_count > 3,
    }
}

/// Detect 3+ consecutive turns where the first bash command is identical.
pub fn detect_identical_command_retries(turns: &[TurnSummary]) -> Vec<BashRetry> {
    let mut retries = Vec::new();
    let mut streak_start: Option<usize> = None;
    let mut streak_len = 0;
    let mut streak_cmd: Option<&str> = None;

    for (i, turn) in turns.iter().enumerate() {
        let first_cmd = turn.bash_commands.first().map(|s| s.as_str());

        let continues = match (first_cmd, streak_cmd) {
            (Some(current), Some(prev)) => current == prev,
            _ => false,
        };

        if continues {
            streak_len += 1;
        } else {
            // Flush previous streak
            if streak_len >= 3 {
                if let Some(cmd) = streak_cmd {
                    retries.push(BashRetry {
                        pattern: BashRetryPattern::IdenticalCommand {
                            command: cmd.to_string(),
                        },
                        start_turn: streak_start.unwrap(),
                        length: streak_len,
                    });
                }
            }

            // Start new streak if this turn has a bash command
            if first_cmd.is_some() {
                streak_start = Some(i);
                streak_len = 1;
                streak_cmd = first_cmd;
            } else {
                streak_start = None;
                streak_len = 0;
                streak_cmd = None;
            }
        }
    }

    // Flush trailing streak
    if streak_len >= 3 {
        if let Some(cmd) = streak_cmd {
            retries.push(BashRetry {
                pattern: BashRetryPattern::IdenticalCommand {
                    command: cmd.to_string(),
                },
                start_turn: streak_start.unwrap(),
                length: streak_len,
            });
        }
    }

    retries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnose::test_helpers::{make_turn, make_turn_with_bash};

    #[test]
    fn test_bash_loop_detection() {
        // 3+ consecutive Bash-only turns detected
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Read"]),
            make_turn(1, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(2, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(3, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(4, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(5, 100, 0, 0, 50, vec!["Edit"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert_eq!(result.bash_loops.len(), 1);
        assert_eq!(result.bash_loops[0].start_turn, 1);
        assert_eq!(result.bash_loops[0].length, 4);
    }

    #[test]
    fn test_bash_loop_no_false_positive() {
        // Mixed tools -> no bash loop flag
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Bash", "Read"]),
            make_turn(1, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(2, 100, 0, 0, 50, vec!["Edit"]),
            make_turn(3, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(4, 100, 0, 0, 50, vec!["Read"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert!(result.bash_loops.is_empty());
    }

    #[test]
    fn test_exploration_ratio_flagged() {
        // >5:1 Read:Edit -> flagged
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Read", "Read", "Glob"]),
            make_turn(1, 100, 0, 0, 50, vec!["Read", "Read", "Grep"]),
            make_turn(2, 100, 0, 0, 50, vec!["Read"]),
            make_turn(3, 100, 0, 0, 50, vec!["Edit"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert!(result.exploration_flagged);
        assert!(result.read_edit_ratio > 5.0);
    }

    #[test]
    fn test_bash_loop_long_streak_with_interruptions() {
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Read"]),
            // First bash streak: 5 turns
            make_turn(1, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(2, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(3, 100, 0, 0, 50, vec!["Bash", "Bash"]),
            make_turn(4, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(5, 100, 0, 0, 50, vec!["Bash"]),
            // Text-only interruption
            make_turn(6, 100, 0, 0, 50, vec![]),
            // Second bash streak: 4 turns
            make_turn(7, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(8, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(9, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(10, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(11, 100, 0, 0, 50, vec!["Edit"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert_eq!(result.bash_loops.len(), 2);
        assert_eq!(result.bash_loops[0].start_turn, 1);
        assert_eq!(result.bash_loops[0].length, 5);
        assert_eq!(result.bash_loops[1].start_turn, 7);
        assert_eq!(result.bash_loops[1].length, 4);
    }

    #[test]
    fn test_identical_command_3x_detected() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Read"], vec![]),
            make_turn_with_bash(1, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(2, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(3, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(4, vec!["Edit"], vec![]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_turn, 1);
        assert_eq!(result[0].length, 3);
        if let BashRetryPattern::IdenticalCommand { ref command } = result[0].pattern {
            assert_eq!(command, "cargo test");
        } else {
            panic!("expected IdenticalCommand pattern");
        }
    }

    #[test]
    fn test_identical_command_different_commands_not_triggered() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(1, vec!["Bash"], vec!["cargo build"]),
            make_turn_with_bash(2, vec!["Bash"], vec!["cargo check"]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert!(result.is_empty());
    }

    #[test]
    fn test_identical_command_multi_bash_uses_first() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash", "Bash"], vec!["cargo test", "echo done"]),
            make_turn_with_bash(1, vec!["Bash", "Bash"], vec!["cargo test", "echo other"]),
            make_turn_with_bash(2, vec!["Bash"], vec!["cargo test"]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].length, 3);
    }

    #[test]
    fn test_identical_command_non_bash_breaks_streak() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(1, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(2, vec!["Read"], vec![]), // no bash commands
            make_turn_with_bash(3, vec!["Bash"], vec!["cargo test"]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert!(result.is_empty()); // no streak >= 3
    }

    #[test]
    fn test_identical_command_empty_bash_commands_ignored() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash"], vec![]),
            make_turn_with_bash(1, vec!["Bash"], vec![]),
            make_turn_with_bash(2, vec!["Bash"], vec![]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert!(result.is_empty());
    }
}
