# Changelog

All notable changes to AgentSight will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-06-22

### Added
- `/clear` advisor: tells you when a session has grown large and stopped earning its keep, so you know when to run `/clear` in Claude Code
  - Post-session verdict in `diagnose` (text + `--json` `clear_advice` block) with the reasons that drove it
  - Live `Clear?` column in `watch`, plus the verdict in `watch --json` / dashboard SSE snapshots
  - Urgency is judged on context-window *fill* (fraction), so it's correct on both 200k and 1M-window models
  - Window size is auto-detected from the model and self-corrects against observed usage (never reports >100% fill)

## [0.1.1] - 2026-05-08

### Fixed
- `--version` shows `v0.1.1` instead of `unknown` when installed from tarball (Homebrew, crates.io)

## [0.1.0] - 2026-05-08

### Added
- Core CLI with `sessions`, `session`, `summary`, `watch` subcommands
- JSONL parser for Claude Code session logs (`~/.claude/projects/`)
- Token attribution by bucket: input, cache creation, cache read, output
- Cache efficiency metrics and hit ratio tracking
- Optional cost estimation with configurable model pricing
- `--json` flag on every command for scriptability
- `--cost` flag and billing mode config (max vs api)
- Auto-generated config at `~/.agentsight/config.toml` on first run
- Compiled-in default pricing with user override support
- `diagnose` subcommand for session efficiency analysis
- Token velocity time-series chart on the Summary dashboard page
- Per-model comparison table (`summary --by-model`) and model distribution in diagnose
- `health` subcommand for environment check and baseline usage report
- `sanitize` subcommand for creating anonymized test fixtures from real sessions
- `install-skill` subcommand and `/agentsight-diagnose` Claude Code slash command
- Project-level diagnose with cross-project benchmarking, trends, and CLAUDE.md analysis
- Bash retry loop detection and same-error retry detection in diagnose
- `timeline` subcommand with CLI Gantt chart and dashboard page
- Web dashboard with Chart.js frontend, interactive session exploration, and live SSE watch
- Multi-session live-watch with real-time token counts
- Hourly burn rate and token velocity KPIs on summary page
- Sortable table headers across dashboard views
- Slug matching with same-slug session disambiguation
- Dual MIT/Apache-2.0 licensing
- README with installation, quick start, and command reference
- CONTRIBUTING.md for open-source onboarding
- Cargo.toml publishing metadata (repository, keywords, categories)
- Shell completions subcommand (bash, zsh, fish)
- Build info in `--version` output (git hash, date)
- `--help` examples on all subcommands
- Dashboard port-conflict UX: detect, reuse, or replace existing instances

### Changed
- Removed permissive CORS (allow-origin: *) from dashboard server — same-origin only
- Removed `tower-http` dependency (no longer needed without CORS layer)

### Fixed
- Dashboard SSE deduplication and pause-on-hidden-tab for performance
- Synthetic model entries filtered from display
