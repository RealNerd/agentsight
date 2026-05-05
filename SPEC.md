# AgentSight CLI — SPEC.md

## 1. Objective

A Rust CLI tool that parses Claude Code session logs and provides token attribution, cost analysis, and session intelligence. It reads the JSONL files Claude Code already writes to `~/.claude/projects/` and turns them into actionable cost and usage data.

**Target users:** Developers using Claude Code who want to understand where their tokens and money go.

**MVP scope:** Four commands — list sessions, drill into a session, cross-session summary, and live-watch the current session.

**Non-goals for v1:** No hooks, no capture layer, no cloud dashboard, no non-Claude-Code agent support.

---

## 2. Commands

### `agentsight sessions`

List recent sessions with cost summaries.

```
$ agentsight sessions

 Session                  Project         Date         Model           Cost    Turns
 delegated-noodling-robin personal/repo   Apr 10       claude-opus-4-6 $4.32   17
 focused-coding-hawk      work/api        Apr 10       claude-opus-4-6 $1.87   8
 quiet-refactor-owl       work/api        Apr 09       claude-sonnet-4 $0.43   12
 ...

 Total (last 7 days): $23.41 across 14 sessions
```

**Flags:**
- `--days <N>` — How far back to look (default: 7)
- `--project <path>` — Filter to a specific project
- `--sort <field>` — Sort by: cost (default), date, turns, project
- `--limit <N>` — Max sessions to show (default: 20)
- `--json` — Output as JSON

### `agentsight session <identifier>`

Drill into a single session. Accepts session slug, UUID prefix, or index from `sessions` list.

```
$ agentsight session delegated-noodling-robin

 Session: delegated-noodling-robin
 Project: /Users/user/repos/personal/repo
 Date:    Apr 10, 2026 15:10 — 15:45 (35m)
 Model:   claude-opus-4-6
 Branch:  main
 Turns:   17

 ── Cost Breakdown ──────────────────────────────
 Input tokens:          1,204      $0.018
 Cache creation:       89,412      $2.683
 Cache read:          214,830      $0.644
 Output tokens:        12,847      $0.963
 ─────────────────────────────────────────────────
 Total:               318,293      $4.308

 ── Cache Efficiency ─────────────────────────────
 Cache hit ratio:      70.6%
 Estimated savings:    $2.89  (vs. no caching)

 ── Tool Usage ───────────────────────────────────
 Read          14 calls     (files: src/main.rs, lib.rs, ...)
 Bash          11 calls
 Edit           6 calls
 Grep           4 calls
 Write          2 calls
 Glob           1 call

 ── Turn-by-Turn ─────────────────────────────────
  #  Timestamp  In     Cache+  Cache~  Out    Cost    Tools
  1  15:10:59   6      7,187   21,815  847    $0.38   Glob, Read
  2  15:12:14   6      3,201   28,996  1,203  $0.42   Bash, Read
  ...
```

**Flags:**
- `--turns` — Show full turn-by-turn table (default: collapsed)
- `--tools` — Show tool usage breakdown (default: shown)
- `--json` — Output as JSON

### `agentsight summary`

Cross-session aggregation and trends.

```
$ agentsight summary

 ── Last 7 Days ──────────────────────────────────
 Sessions:  14
 Total cost: $23.41
 Avg cost/session: $1.67

 ── Cost by Day ──────────────────────────────────
 Mon Apr 07:  $3.21  ███████
 Tue Apr 08:  $5.82  █████████████
 Wed Apr 09:  $2.10  █████
 Thu Apr 10:  $6.32  ██████████████
 Fri Apr 11:  $5.96  █████████████

 ── Cost by Project ──────────────────────────────
 work/api:         $14.20  (60.7%)
 personal/repo:     $6.81  (29.1%)
 personal/tools:    $2.40  (10.2%)

 ── Cost by Model ────────────────────────────────
 claude-opus-4-6:   $19.12  (81.7%)
 claude-sonnet-4:    $4.29  (18.3%)

 ── Cache Performance ────────────────────────────
 Avg cache hit ratio:  68.2%
 Total estimated savings: $31.40
```

**Flags:**
- `--days <N>` — Period to summarize (default: 7)
- `--project <path>` — Filter to a specific project
- `--json` — Output as JSON

### `agentsight watch`

Live-watch the most recently active session. Re-reads the JSONL file on change and updates the display.

```
$ agentsight watch

 ⠋ Watching: focused-coding-hawk (work/api) — claude-opus-4-6
   Running cost: $1.87 | Turns: 8 | Cache hit: 72.3%
   Last tool: Bash (git status) — 3s ago
```

Updates in-place as new entries appear in the JSONL file. Exits when the session goes idle (no new entries for configurable timeout) or on Ctrl+C.

**Flags:**
- `--session <id>` — Watch a specific session instead of the most recent
- `--idle-timeout <seconds>` — Exit after N seconds of inactivity (default: 300)
- `--json` — Stream JSON lines instead of TUI

### `agentsight timeline` (planned)

Visualize concurrent session activity to help evaluate usage patterns when multi-tasking across projects. Many CC users run 4-5 sessions simultaneously — this command reveals whether that pattern is efficient or wasteful.

```
$ agentsight timeline --days 1

 13:00  14:00  15:00  16:00  17:00  18:00  19:00  20:00  21:00  22:00
 goldthread   ████████████████████████████████████████████████████████████
 extraction        ██   █████████    ████████████████   ██████    ████████████
 fairbound            ██████  ███████  ██████  ████████████  ████████  ████████
 fanscloud        ████████
 llc                       █████████████
 agentsight                                                          █████████

 ── Concurrency ────────────────────────────────────
 Peak concurrent sessions:     5  (13:48 — 14:09)
 Avg concurrent sessions:      2.8
 Total tokens during peak:     12,847,203

 ── Efficiency by Concurrency Level ────────────────
 1 session:   avg 14.2M tokens/session   avg cache hit 96.1%
 2 sessions:  avg 15.8M tokens/session   avg cache hit 94.3%
 3 sessions:  avg 17.1M tokens/session   avg cache hit 92.0%
 4+ sessions: avg 19.4M tokens/session   avg cache hit 88.7%
```

**Key insight:** Each concurrent session maintains its own context window and cache. Running N sessions simultaneously means N independent cache warm-ups. This command quantifies whether parallelism costs more tokens per session than sequential work, helping users decide when multi-tasking is worth it vs. focusing on one project at a time.

**Flags:**
- `--days <N>` — How far back to look (default: 1)
- `--project <path>` — Highlight a specific project in the timeline
- `--json` — Output as JSON

### Global Flags

- `--json` — Machine-readable JSON output (all commands)
- `--config <path>` — Custom config file path
- `--claude-dir <path>` — Override Claude Code data directory (default: `~/.claude`)

---

## 3. Project Structure

```
agentsight/
├── Cargo.toml
├── SPEC.md
├── CLAUDE.md
├── config/
│   └── default_pricing.toml      # Default model pricing, embedded at compile time
├── src/
│   ├── main.rs                   # Entry point, clap CLI setup
│   ├── commands/                 # Command implementations
│   │   ├── mod.rs
│   │   ├── sessions.rs           # List sessions
│   │   ├── session.rs            # Single session detail
│   │   ├── summary.rs            # Cross-session aggregation
│   │   ├── watch.rs              # Live file watcher
│   │   └── timeline.rs           # (planned) Concurrent session visualization
│   ├── parser/                   # JSONL parsing layer
│   │   ├── mod.rs
│   │   ├── types.rs              # Serde structs matching JSONL schema
│   │   ├── reader.rs             # Stream JSONL files, yield typed entries
│   │   └── session_index.rs      # Discover sessions from ~/.claude/projects/
│   ├── cost/                     # Cost calculation engine
│   │   ├── mod.rs
│   │   ├── pricing.rs            # Model pricing table (from config)
│   │   └── calculator.rs         # Token counts → dollar amounts
│   ├── config/                   # Configuration
│   │   └── mod.rs                # Load/merge config files
│   └── output/                   # Output formatting
│       ├── mod.rs
│       ├── table.rs              # Terminal table rendering
│       └── json.rs               # JSON serialization
└── tests/
    ├── fixtures/                 # Sanitized sample JSONL files
    │   ├── small_session.jsonl
    │   └── large_session.jsonl
    └── integration/
        ├── parser_test.rs
        ├── cost_test.rs
        └── commands_test.rs
```

### Key Dependencies (Cargo.toml)

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }       # CLI argument parsing
serde = { version = "1", features = ["derive"] }       # Serialization framework
serde_json = "1"                                       # JSON parsing
toml = "0.8"                                           # Config file parsing
chrono = { version = "0.4", features = ["serde"] }     # Timestamp handling
comfy-table = "7"                                      # Terminal table rendering
notify = "7"                                           # File system watcher (for watch cmd)
dirs = "6"                                             # Home directory detection
uuid = { version = "1", features = ["serde"] }         # UUID handling
anyhow = "1"                                           # Error handling
crossterm = "0.28"                                     # Terminal control (for watch cmd)
```

---

## 4. Code Style

### Rust Conventions
- **Edition 2021** minimum
- `cargo fmt` before every commit — no exceptions
- `cargo clippy` must pass with no warnings
- All public functions and types get doc comments
- Use `thiserror` or `anyhow` for error handling — no `.unwrap()` in library code (`.unwrap()` acceptable only in tests)

### Naming
- Modules: `snake_case`
- Types/structs: `PascalCase`
- Functions: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`

### Architecture Principles
- **Parser layer is pure** — takes bytes in, returns typed structs out. No side effects, no config awareness.
- **Cost layer is pure** — takes token counts + pricing table, returns dollar amounts. Easy to test.
- **Command layer orchestrates** — calls parser, calls cost, calls output. Thin glue.
- **Output layer is swappable** — table vs JSON decided at the command layer, formatters don't know about each other.

### Error Handling
- Malformed JSONL lines: skip and warn to stderr, don't crash. Session files may have partial writes.
- Missing fields: use `Option<T>` with serde defaults. Claude Code's schema evolves — be resilient.
- Missing config file: fall back to compiled-in defaults silently.
- Permission errors on `~/.claude`: clear error message explaining what's needed.

---

## 5. Testing Strategy

### Unit Tests (in-module)
- **Parser types**: Deserialize sample JSON strings into structs, verify all fields.
- **Cost calculator**: Given known token counts and pricing, assert exact dollar amounts. Test each cost bucket (input, cache creation, cache read, output) independently.
- **Config loading**: Default pricing loads correctly. Override file merges correctly. Missing file falls back gracefully.

### Integration Tests (tests/ directory)
- **Fixture-based**: Sanitized JSONL files (real structure, fake content) in `tests/fixtures/`. Parse entire files, verify session-level aggregates.
- **Command output**: Run commands against fixture data, assert output contains expected values. Test both table and JSON modes.
- **Edge cases**: Empty session file. Session with only user messages (no assistant). Session mid-write (truncated last line). Very large session (performance).

### Test Fixtures
Create sanitized versions of real session files. Replace:
- Actual code content → placeholder strings
- File paths → generic paths
- Thinking content → "[thinking]"
- Keep: token counts, timestamps, model names, tool names, UUIDs (structure matters)

### CI Checks
```bash
cargo fmt --check          # Formatting
cargo clippy               # Lints
cargo test                 # All tests
cargo build --release      # Release build succeeds
```

---

## 6. Boundaries

### Always Do
- Keep all data local — never send anything to a network endpoint
- Handle malformed/evolving JSONL gracefully — skip bad lines, warn, continue
- Show costs in USD with 2-3 decimal places
- Include cache efficiency metrics alongside raw costs (this is a key insight users won't get elsewhere)
- Support `--json` on every command for scriptability
- Embed default pricing at compile time so the tool works with zero config

### Ask First
- Before adding support for non-Claude-Code agents (different log format, different scope)
- Before adding any network features (telemetry, update checks, cloud sync)
- Before adding interactive/TUI features beyond the watch command
- Before changing the config file format or location
- Before adding subcommands beyond the five specified (sessions, session, summary, watch, timeline)

### Never Do
- Never read or transmit code content from session logs — only metadata and token counts
- Never modify Claude Code's files (`~/.claude/` is read-only to us)
- Never require an API key or authentication for basic functionality
- Never add telemetry or phone-home behavior without explicit opt-in
- Never panic on malformed input — always handle gracefully

---

## 7. Data Model Reference

### JSONL Entry Types (from Claude Code session files)

| Type | Key Fields | Use |
|------|-----------|-----|
| `assistant` | `message.usage.*`, `message.model`, `message.content[]` | Token attribution, cost calculation |
| `user` | `message.content`, `toolUseResult` | Turn counting, tool result tracking |
| `progress` | `data.type`, `data.hookEvent`, `toolUseID` | Tool execution tracking |
| `system` | `subtype: "turn_duration"`, `durationMs` | Turn timing |
| `file-history-snapshot` | `snapshot.trackedFileBackups` | File change tracking (future use) |
| `queue-operation` | `operation`, `content` | Task queue tracking (future use) |

### Token Usage Struct (from assistant entries)

```
input_tokens                          — base input cost
cache_creation_input_tokens           — 1.25x input price (writing to cache)
  └─ ephemeral_5m_input_tokens        — 5-minute cache tier
  └─ ephemeral_1h_input_tokens        — 1-hour cache tier
cache_read_input_tokens               — 0.1x input price (reading from cache)
output_tokens                         — output price
```

### Pricing Config Format (TOML)

```toml
# ~/.agentsight/pricing.toml

[models."claude-opus-4-7"]
input_per_million = 5.00
output_per_million = 25.00
cache_creation_per_million = 6.25     # 1.25x input
cache_read_per_million = 0.50         # 0.1x input

[models."claude-opus-4-6"]
input_per_million = 5.00
output_per_million = 25.00
cache_creation_per_million = 6.25     # 1.25x input
cache_read_per_million = 0.50         # 0.1x input

[models."claude-sonnet-4-6"]
input_per_million = 3.00
output_per_million = 15.00
cache_creation_per_million = 3.75     # 1.25x input
cache_read_per_million = 0.30         # 0.1x input

[models."claude-haiku-4-5"]
input_per_million = 1.00
output_per_million = 5.00
cache_creation_per_million = 1.25     # 1.25x input
cache_read_per_million = 0.10         # 0.1x input

# Legacy models (may appear in older session logs)
[models."claude-3-5-sonnet-20241022"]
input_per_million = 3.00
output_per_million = 15.00
cache_creation_per_million = 3.75
cache_read_per_million = 0.30
```

### Session Discovery

Sessions are found by scanning:
```
~/.claude/projects/<encoded-project-path>/<session-uuid>.jsonl
```

The global index at `~/.claude/history.jsonl` maps timestamps and prompts to session IDs but does not contain token data. Use it for session metadata (display name, project path) and the per-session JSONL files for actual attribution data.

### Session Identification

Each session has multiple identifiers:
- `sessionId` (UUID) — primary key, used in filenames
- `slug` — human-readable name like "delegated-noodling-robin"
- Encoded project path — directory name maps to the project's filesystem path

The CLI should accept slug, UUID prefix, or list index for the `session` command.
