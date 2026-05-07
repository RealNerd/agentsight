# AgentSight

Token attribution and session intelligence for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Parses session logs from `~/.claude/projects/` and surfaces usage analytics, cache efficiency metrics, and optional cost estimation — as a CLI and a web dashboard.

## What it does

Claude Code writes a JSONL log for every session. AgentSight reads those logs (never modifies them) and answers questions like:

- Where are my tokens going? Which projects, which sessions, which hours?
- How efficient is my cache hit ratio? Is context churning wasting tokens?
- Is my CLAUDE.md helping or hurting?
- Am I stuck in bash retry loops?

## Installation

### From source (requires Rust toolchain)

```bash
git clone https://github.com/RealNerd/agentsight.git
cd agentsight
cargo install --path .
```

### Verify

```bash
agentsight --version
agentsight health        # Check environment and baseline usage
```

## Quick start

```bash
# List recent sessions
agentsight sessions

# Drill into a specific session
agentsight session my-feature-slug

# Cross-session summary (last 7 days)
agentsight summary

# Live-watch active sessions
agentsight watch

# Launch the web dashboard
agentsight dashboard
```

## Commands

| Command | Description |
|---------|-------------|
| `sessions` | List recent sessions with usage summaries |
| `session <id>` | Drill into a single session (by slug, UUID prefix, or index) |
| `summary` | Cross-session aggregation, trends, and per-model comparison |
| `watch` | Live-tail all active sessions with real-time token counts |
| `timeline` | Session timeline with concurrency analysis |
| `diagnose` | Session efficiency diagnosis with actionable recommendations |
| `health` | Environment check and baseline usage report |
| `dashboard` | Launch web dashboard with charts and interactive exploration |
| `install-skill` | Install AgentSight as a Claude Code slash command |

Every command supports `--json` for scriptability.

## CLI examples

```bash
# Summary for the last 30 days, filtered to one project
agentsight summary --days 30 --project my-app

# Show cost estimates (for API/pay-per-token users)
agentsight summary --cost

# Per-model comparison
agentsight summary --by-model

# Diagnose a specific session
agentsight diagnose my-feature-slug

# Project-level diagnosis with CLAUDE.md analysis
agentsight diagnose --project my-app --with-context

# Machine-readable output
agentsight sessions --json | jq '.sessions[0]'
```

## Web dashboard

```bash
agentsight dashboard          # Opens at http://127.0.0.1:3141
agentsight dashboard --port 8080 --no-open
```

The dashboard provides interactive views for all CLI commands: session list, summary with charts, timeline visualization, live watch with SSE, and efficiency diagnostics.

## Configuration

AgentSight creates `~/.agentsight/config.toml` on first run with commented defaults.

```toml
# Billing mode: "max" (default) shows token usage only.
# Set to "api" to show dollar cost estimates by default.
# billing = "api"

# Override model pricing (per million tokens, USD).
# Built-in pricing is used for models not listed here.
#
# [models."claude-opus-4-6"]
# input_per_million = 5.00
# output_per_million = 25.00
# cache_creation_per_million = 6.25
# cache_read_per_million = 0.50
```

The `--cost` flag on any command enables cost display regardless of config.

## How it works

Claude Code writes session logs to `~/.claude/projects/{project}/{session-uuid}.jsonl`. Each line is a typed JSON object (`assistant`, `user`, `system`, `progress`, etc.). AgentSight parses `assistant` entries for token usage breakdowns:

- **Input tokens** -- base prompt cost
- **Cache creation tokens** -- writing new context to cache (1.25x input price)
- **Cache read tokens** -- reusing cached context (0.1x input price)
- **Output tokens** -- model response cost

AgentSight is read-only. It never modifies Claude Code's files, never sends data over the network, and requires no API keys.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
