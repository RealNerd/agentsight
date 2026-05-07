# Changelog

All notable changes to AgentSight will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
- Cargo.toml publishing metadata (repository, keywords, categories)

### Changed
- Removed permissive CORS (allow-origin: *) from dashboard server — same-origin only
- Removed `tower-http` dependency (no longer needed without CORS layer)

### Fixed
- Dashboard SSE deduplication and pause-on-hidden-tab for performance
- Synthetic model entries filtered from display

## [0.1.0] - Initial Development

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
