# AgentSight

Rust CLI for token attribution and session intelligence for Claude Code. Parses session JSONL files from `~/.claude/projects/` and provides usage analytics, cache efficiency metrics, and optional cost estimation.

## Development Commands

```bash
cargo build                  # Build
cargo test                   # Run tests
cargo run -- sessions        # List recent sessions
cargo run -- session <slug>  # Drill into a session
cargo run -- summary         # Cross-session aggregation
cargo run -- watch           # Live-tail active session
cargo run -- diagnose        # Session efficiency diagnosis
cargo run -- install-skill   # Install CC slash commands
cargo run -- --cost <cmd>    # Add cost estimates to any command
cargo clippy                 # Lint
cargo fmt                    # Format
```

## Architecture

```
src/
├── main.rs              # CLI entry (clap derive)
├── commands/            # One file per subcommand
├── skills/              # Built-in CC skill definitions (embedded via include_str!)
├── parser/              # JSONL types, reader, session discovery
│   ├── types.rs         # Serde structs matching Claude Code JSONL schema
│   ├── reader.rs        # Parse files, summarize sessions
│   └── session_index.rs # Discover sessions from ~/.claude/projects/
├── cost/                # Pricing config → dollar amounts (optional layer)
├── config/              # Config loading, billing mode, pricing tables
└── output/              # Table (comfy-table) and JSON formatters
```

**Key design rule:** Parser and cost layers are pure — no side effects, no config awareness. Command layer is thin glue. Output layer is swappable (table vs JSON decided at command layer).

## Data Source

Claude Code writes session logs to `~/.claude/projects/{encoded-project}/{session-uuid}.jsonl`. Each line is a JSON object with a `type` field: `assistant` (has token usage), `user`, `progress`, `system`, `file-history-snapshot`, `queue-operation`. Token attribution comes from `assistant` entries' `message.usage` object.

## Conventions

- Default output shows token usage only (Max/subscription users)
- `--cost` flag or `billing = "api"` in config adds dollar amounts
- `--json` flag on every command for scriptability
- Malformed JSONL lines: skip and warn to stderr, never crash
- `~/.claude/` is read-only — we never modify Claude Code's files
- Default pricing compiled in at build time; override via `~/.agentsight/config.toml`

## CI

GitHub Actions runs Format, Check, Clippy (`-D warnings`), Test, and Dependency audit on every push. Clippy treats all warnings as errors — fix them before pushing.

## Issue Tracking

Uses `bd` (beads) locally. Run `bd list` to see open work, `bd ready` for available tasks. The `.beads/` directory is gitignored — not tracked in the repo.
