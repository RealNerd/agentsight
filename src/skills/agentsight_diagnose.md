---
description: Diagnose session efficiency and suggest CLAUDE.md improvements
---

# AgentSight Diagnose

Analyze the most recent (or specified) Claude Code session for inefficiency patterns and suggest concrete CLAUDE.md improvements.

## Arguments

$ARGUMENTS

If arguments are provided, pass them as the session identifier. Otherwise analyze the most recent session for the current project.

## Step 1: Run Diagnosis

Determine the current project name from the working directory basename (e.g., if the cwd is `/home/user/repos/myapp`, the project name is `myapp`).

Run agentsight diagnose with JSON output, scoped to the current project:

```bash
agentsight diagnose $ARGUMENTS --project <project-name> --json
```

If agentsight is not found on PATH, try:
```bash
~/.cargo/bin/agentsight diagnose $ARGUMENTS --project <project-name> --json
```

If neither works, tell the user to install agentsight first.

## Step 2: Interpret Results

Parse the JSON output and identify flagged patterns:

- **cache_stability.classification**: If "churning" or "degrading", the session's context kept being rebuilt
- **context_growth.flagged**: If true, context grew significantly — session was too long
- **tool_patterns.bash_loops**: Each entry is a sequence where the agent was stuck retrying shell commands
- **tool_patterns.exploration_flagged**: Agent spent too much time reading vs writing — it didn't know where to look
- **tool_patterns.subagent_flagged**: Too many Task/subagent spawns, each creating new context windows

## Step 3: Read Current CLAUDE.md

Read the project's CLAUDE.md file to understand what documentation already exists.

## Step 4: Suggest Improvements

For each flagged pattern, suggest a specific CLAUDE.md addition:

### If bash_loops detected:
- Check what commands were being retried (look at recent session context)
- Suggest adding build/test/deploy commands with correct working directories and flags
- Suggest adding a "stop after 2 failures and re-evaluate" rule

### If exploration_flagged:
- Suggest adding an Architecture section listing key source files and their purposes
- Suggest adding a "where to find things" section for common tasks

### If cache_stability is churning/degrading:
- Suggest breaking multi-topic work into focused sessions
- If CLAUDE.md is very large, suggest splitting into focused sections

### If context_growth flagged:
- Suggest a workflow rule to start new sessions for new topics
- Note the turn count where growth became problematic

### If subagent_flagged:
- Suggest documenting which operations are simple enough to do inline vs delegate

## Step 5: Present Changes

Present each suggested CLAUDE.md change as a specific edit:
1. Show the exact text to add and where it should go
2. Explain why this change addresses the diagnosed pattern
3. Ask the user if they want to apply each change

Do NOT make changes without user approval.
